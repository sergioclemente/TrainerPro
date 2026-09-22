//! Player runtime: the async task that drives Engine actions into the trainer,
//! records the workout session, and finalizes it as an Activity plus FIT file.
//! The engine stays pure; all time and I/O live here.

use std::collections::VecDeque;
use std::fs::File;
use std::io::BufReader;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{error, warn};

use tp_core::consts::{ENGINE_TICK_MS, ERG_KEEPALIVE_S, SAMPLE_HZ, SMOOTH_WINDOW_S};
use tp_core::engine::{Engine, EngineAction, EngineEvent, Phase, SegmentEndReason};
use tp_core::journal::{
    compute_laps, replay, JournalHeader, JournalWriter, Sample, SessionEvent, SessionEventKind,
};
use tp_core::metrics::{normalized_power, session_totals, tss};
use tp_core::workout_definition::WorkoutDefinition;

use crate::app_error::AppError;
use crate::app_state::{now_unix_ms, AppState};
use crate::database::activities as activity_db;
use crate::heart_rate_monitor::HeartRateMonitor;
use crate::trainer::Trainer;

const PLAYER_COMMAND_CAPACITY: usize = 16;
const MILLIS_PER_SECOND: u64 = 1_000;

/// A message accepted by the async player runtime. The runtime translates
/// workout events into `EngineEvent`s and owns I/O-only controls such as ERG.
pub enum PlayerCommand {
    Start,
    Pause,
    Resume,
    SkipSegment,
    SetIntensity(f64),
    /// Toggle ERG mode. Off = trainer switches to simulation grade 0 (free
    /// resistance); workout targets keep advancing but aren't sent.
    SetErg(bool),
    End(oneshot::Sender<Result<ActivitySummary, AppError>>),
}

pub struct PlayerHandle {
    pub command_tx: mpsc::Sender<PlayerCommand>,
    pub state_rx: watch::Receiver<PlayerState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayerState {
    pub phase: String,
    pub workout_session_id: String,
    pub workout_definition_id: String,
    pub scheduled_workout_id: Option<String>,
    pub workout_name: String,
    pub workout_duration_s: u32,
    pub seg_idx: Option<usize>,
    pub seg_remaining_s: u32,
    /// Position in the workout: drives the graph cursor and "remaining".
    /// A skip jumps this forward.
    pub elapsed_s: u32,
    /// Time actually ridden — pauses and skipped spans excluded.
    pub ride_s: u32,
    pub intensity: f64,
    pub target_power_w: Option<u16>,
    pub target_cadence_rpm: Option<u16>,
    pub average_power_w: Option<u16>,
    /// Live session totals, mirroring the post-ride numbers.
    /// `None` until there is data to compute them from.
    pub normalized_power_w: Option<u16>,
    pub training_stress_score: Option<f64>,
    /// Efficiency factor: NP / average HR. `None` without an HRM.
    pub ef: Option<f64>,
    pub kcal: Option<u32>,
    pub erg_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayerMeasurement {
    pub power_w: Option<u16>,
    pub cadence_rpm: Option<u16>,
    pub heart_rate_bpm: Option<u16>,
    pub power_smoothed_3s_w: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentResult {
    pub workout_session_id: String,
    pub segment_index: usize,
    pub planned_duration_s: u32,
    pub ridden_duration_s: u32,
    pub average_power_w: Option<u16>,
    pub average_cadence_rpm: Option<u16>,
    pub skipped: bool,
}

#[derive(Default)]
struct SegmentAccumulator {
    ridden_ms: u64,
    power_sum: u64,
    power_n: u32,
    cadence_sum: u64,
    cadence_n: u32,
}

impl SegmentAccumulator {
    fn note_tick(&mut self, elapsed_ms: u64) {
        self.ridden_ms = self.ridden_ms.saturating_add(elapsed_ms);
    }

    fn note_sample(&mut self, power_w: Option<u16>, cadence_rpm: Option<u16>) {
        if let Some(power_w) = power_w {
            self.power_sum += u64::from(power_w);
            self.power_n += 1;
        }
        if let Some(cadence_rpm) = cadence_rpm {
            self.cadence_sum += u64::from(cadence_rpm);
            self.cadence_n += 1;
        }
    }

    fn take_result(
        &mut self,
        workout_session_id: &str,
        segment_index: usize,
        planned_duration_s: u32,
        skipped: bool,
    ) -> SegmentResult {
        let completed = std::mem::take(self);
        SegmentResult {
            workout_session_id: workout_session_id.to_owned(),
            segment_index,
            planned_duration_s,
            ridden_duration_s: (completed.ridden_ms / MILLIS_PER_SECOND) as u32,
            average_power_w: (completed.power_n > 0)
                .then(|| (completed.power_sum / u64::from(completed.power_n)) as u16),
            average_cadence_rpm: (completed.cadence_n > 0)
                .then(|| (completed.cadence_sum / u64::from(completed.cadence_n)) as u16),
            skipped,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LapRow {
    pub start_s: u32,
    pub duration_s: u32,
    pub average_power_w: Option<u16>,
    pub max_power_w: Option<u16>,
    pub average_heart_rate_bpm: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivitySummary {
    pub activity_id: String,
    pub scheduled_workout_id: Option<String>,
    pub workout_name: String,
    pub started_at_unix_ms: u64,
    pub elapsed_s: u32,
    pub timer_s: u32,
    pub average_power_w: Option<u16>,
    pub max_power_w: Option<u16>,
    pub normalized_power_w: Option<u16>,
    pub intensity_factor: Option<f64>,
    pub training_stress_score: Option<f64>,
    pub average_heart_rate_bpm: Option<u16>,
    pub max_heart_rate_bpm: Option<u16>,
    pub work_kj: u32,
    pub completed_pct: f64,
    pub fit_path: String,
    pub laps: Vec<LapRow>,
}

fn phase_str(p: Phase) -> &'static str {
    match p {
        Phase::Ready => "ready",
        Phase::Riding => "riding",
        Phase::Paused => "paused",
        Phase::Finished => "finished",
    }
}

/// Spawn the runtime task for a loaded workout. Trainer must be connected.
pub async fn spawn(
    app: AppHandle,
    workout_definition_id: String,
    scheduled_workout_id: Option<String>,
    workout_definition: WorkoutDefinition,
) -> Result<PlayerHandle, AppError> {
    let state = app.state::<AppState>();
    let trainer = state.hub.trainer().clone();
    let trainer_state = trainer.state();
    if trainer_state.platform_id.is_none() {
        return Err(AppError::new("no_trainer", "connect a trainer first"));
    }
    let heart_rate_monitor = state.hub.heart_rate_monitor().clone();
    let heart_rate_state = heart_rate_monitor.state();

    let settings = state.settings();
    let workout_definition_snapshot_json = workout_definition.to_json_pretty()?;
    let workout = workout_definition.compile()?;
    let workout_session_id = uuid::Uuid::new_v4().to_string();
    let session_started_at_ms = now_unix_ms();
    std::fs::create_dir_all(state.activities_dir())?;
    let journal_path = state
        .activities_dir()
        .join(format!("{workout_session_id}.jsonl"));
    let header = JournalHeader {
        workout_session_id: workout_session_id.clone(),
        workout_definition_id: workout_definition_id.clone(),
        scheduled_workout_id: scheduled_workout_id.clone(),
        workout_definition_snapshot_json,
        started_unix_ms: session_started_at_ms,
        workout_name: workout.name.clone(),
        ftp_w: settings.profile.ftp,
        weight_kg: settings.profile.weight_kg,
        trainer: trainer_state.name,
        hrm: heart_rate_state.name,
        app_ver: env!("CARGO_PKG_VERSION").into(),
    };
    let journal = JournalWriter::new(File::create(&journal_path)?, &header)
        .map_err(|e| AppError::new("journal", e.to_string()))?;

    let engine = Engine::new(
        workout.clone(),
        settings.profile.ftp,
        settings.intensity_default,
    );
    let (command_tx, command_rx) = mpsc::channel(PLAYER_COMMAND_CAPACITY);
    let initial = PlayerState {
        phase: "ready".into(),
        workout_session_id: workout_session_id.clone(),
        workout_definition_id: workout_definition_id.clone(),
        scheduled_workout_id: scheduled_workout_id.clone(),
        workout_name: workout.name.clone(),
        workout_duration_s: workout.duration_s(),
        seg_idx: Some(0),
        seg_remaining_s: workout
            .segments
            .first()
            .map(|s| s.duration_s())
            .unwrap_or(0),
        elapsed_s: 0,
        ride_s: 0,
        intensity: settings.intensity_default,
        target_power_w: None,
        target_cadence_rpm: workout.target_cadence_rpm_at(0),
        average_power_w: None,
        normalized_power_w: None,
        training_stress_score: None,
        ef: None,
        kcal: None,
        erg_enabled: true,
    };
    let (state_tx, state_rx) = watch::channel(initial);

    let rt = Runtime {
        app,
        engine,
        trainer,
        heart_rate_monitor,
        journal: Some(journal),
        journal_path: journal_path.to_string_lossy().into_owned(),
        workout_session_id,
        workout_definition_id,
        scheduled_workout_id,
        ride_started_at: None,
        state_tx,
        last_target_power_w: None,
        in_free_ride: false,
        erg_enabled: true,
        latest_power_w: None,
        power_smoothed_3s_w: None,
        latest_cadence_rpm: None,
        latest_heart_rate_bpm: None,
        power_window: VecDeque::new(),
        power_sum: 0,
        power_n: 0,
        live_power: Vec::new(),
        hr_sum: 0,
        hr_n: 0,
        segment_accumulator: SegmentAccumulator::default(),
        ftp_w: settings.profile.ftp,
    };
    tokio::spawn(rt.run(command_rx));
    Ok(PlayerHandle {
        command_tx,
        state_rx,
    })
}

struct Runtime {
    app: AppHandle,
    engine: Engine,
    trainer: Trainer,
    heart_rate_monitor: HeartRateMonitor,
    journal: Option<JournalWriter<File>>,
    journal_path: String,
    workout_session_id: String,
    workout_definition_id: String,
    scheduled_workout_id: Option<String>,
    /// Wall-clock ride origin, set on Start. Journal t_ms is measured from
    /// here (includes paused spans in the timeline; no samples during pause).
    ride_started_at: Option<Instant>,
    state_tx: watch::Sender<PlayerState>,
    last_target_power_w: Option<u16>,
    in_free_ride: bool,
    /// ERG control enabled (user toggle). When false the trainer is left in
    /// simulation mode and target writes are suppressed.
    erg_enabled: bool,
    latest_power_w: Option<u16>,
    power_smoothed_3s_w: Option<u16>,
    latest_cadence_rpm: Option<u16>,
    latest_heart_rate_bpm: Option<u16>,
    /// (arrival, power) pairs for the 3 s display smoothing window.
    power_window: VecDeque<(Instant, u16)>,
    /// Running ride average over 1 Hz samples (riding time only).
    power_sum: u64,
    power_n: u32,
    /// The same 1 Hz power series the journal records (absent = 0 W), kept in
    /// memory so live NP is computed by the very function that produces the
    /// post-ride number — the two can't drift apart. Ordinary rides are a few
    /// thousand samples; recomputing once a second is nothing.
    live_power: Vec<u16>,
    /// Running HR average over 1 Hz samples, for EF.
    hr_sum: u64,
    hr_n: u32,
    /// Ride time and 1 Hz measurements for the current workout segment.
    segment_accumulator: SegmentAccumulator,
    /// Rider FTP at load time — TSS and IF are relative to it.
    ftp_w: u16,
}

impl Runtime {
    async fn run(mut self, mut command_rx: mpsc::Receiver<PlayerCommand>) {
        let mut engine_tick = tokio::time::interval(Duration::from_millis(ENGINE_TICK_MS));
        engine_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut sample_tick =
            tokio::time::interval(Duration::from_secs_f64(1.0 / f64::from(SAMPLE_HZ)));
        let mut keepalive = tokio::time::interval(Duration::from_secs(ERG_KEEPALIVE_S));
        let mut trainer_measurements = self.trainer.subscribe_measurements();
        let mut trainer_state_rx = self.trainer.subscribe_state();
        let mut trainer_generation = trainer_state_rx.borrow().generation;
        let mut heart_rate_measurements = self.heart_rate_monitor.subscribe_measurements();
        let mut heart_rate_state_rx = self.heart_rate_monitor.subscribe_state();
        let mut heart_rate_generation = heart_rate_state_rx.borrow().generation;

        loop {
            tokio::select! {
                command = command_rx.recv() => {
                    let Some(command) = command else { break };
                    match command {
                        PlayerCommand::Start => {
                            self.handle_start().await;
                        }
                        PlayerCommand::Pause => {
                            self.write_event(SessionEventKind::Pause, None);
                            let actions = self.engine.step(EngineEvent::Pause);
                            self.apply_engine_actions(actions).await;
                        }
                        PlayerCommand::Resume => {
                            self.write_event(SessionEventKind::Resume, None);
                            let actions = self.engine.step(EngineEvent::Resume);
                            self.apply_engine_actions(actions).await;
                        }
                        PlayerCommand::SkipSegment => {
                            let actions = self.engine.step(EngineEvent::SkipSegment);
                            self.apply_engine_actions(actions).await;
                        }
                        PlayerCommand::SetIntensity(i) => {
                            let actions = self.engine.step(EngineEvent::SetIntensity(i));
                            self.apply_engine_actions(actions).await;
                        }
                        PlayerCommand::SetErg(on) => {
                            self.erg_enabled = on;
                            let r = if on {
                                match self.last_target_power_w {
                                    Some(w) if !self.in_free_ride => {
                                        self.trainer.set_target_power(w).await
                                    }
                                    _ => Ok(()),
                                }
                            } else {
                                self.trainer.set_flat_road_simulation().await
                            };
                            if let Err(e) = r {
                                self.trainer_error(&e.to_string()).await;
                            }
                        }
                        PlayerCommand::End(reply) => {
                            let actions = self.engine.step(EngineEvent::End);
                            self.execute_engine_actions(&actions).await;
                            let _ = reply.send(self.finalize().await);
                            break;
                        }
                    }
                    self.push_state();
                }
                _ = engine_tick.tick() => {
                    if self.engine.phase() == Phase::Riding {
                        let actions = self.engine.step(EngineEvent::Tick { dt_ms: ENGINE_TICK_MS });
                        self.segment_accumulator.note_tick(ENGINE_TICK_MS);
                        let finished = self.apply_engine_actions(actions).await;
                        if finished { break; }
                    }
                }
                _ = sample_tick.tick() => {
                    if self.engine.phase() == Phase::Riding {
                        self.write_sample();
                        self.push_state();
                    }
                }
                _ = keepalive.tick() => {
                    if self.engine.phase() == Phase::Riding
                        && !self.in_free_ride
                        && self.erg_enabled
                    {
                        if let Some(w) = self.last_target_power_w {
                            if let Err(e) = self.trainer.set_target_power(w).await {
                                self.trainer_error(&e.to_string()).await;
                            }
                        }
                    }
                }
                r = trainer_measurements.recv() => {
                    match r {
                        Ok(sample) if trainer_state_rx.borrow().accepts(&sample) => {
                            let d = sample.value;
                            let now = Instant::now();
                            self.latest_power_w = d.power_w;
                            self.latest_cadence_rpm = d.cadence_rpm.map(|c| c.round() as u16);
                            if let Some(p) = d.power_w {
                                self.power_window.push_back((now, p));
                            }
                            while let Some((t, _)) = self.power_window.front() {
                                if now.duration_since(*t).as_secs_f64()
                                    > f64::from(SMOOTH_WINDOW_S)
                                {
                                    self.power_window.pop_front();
                                } else {
                                    break;
                                }
                            }
                            self.power_smoothed_3s_w = if self.power_window.is_empty() { None } else {
                                Some((self.power_window.iter().map(|(_, p)| u32::from(*p)).sum::<u32>()
                                    / self.power_window.len() as u32) as u16)
                            };
                            self.emit_player_measurement();
                        }
                        Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
                r = heart_rate_measurements.recv() => {
                    match r {
                        Ok(sample) if heart_rate_state_rx.borrow().accepts(&sample) => {
                            self.latest_heart_rate_bpm = Some(sample.value.bpm);
                            self.emit_player_measurement();
                        }
                        Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                changed = heart_rate_state_rx.changed() => {
                    if changed.is_ok() {
                        let state = heart_rate_state_rx.borrow_and_update().clone();
                        if !state.is_connected() || state.generation != heart_rate_generation {
                            heart_rate_generation = state.generation;
                            self.latest_heart_rate_bpm = None;
                        }
                        self.emit_player_measurement();
                    }
                }
                changed = trainer_state_rx.changed() => {
                    if changed.is_ok() {
                        let state = trainer_state_rx.borrow_and_update().clone();
                        let connection_changed = state.generation != trainer_generation;
                        trainer_generation = state.generation;
                        if (connection_changed || !state.is_connected())
                            && self.engine.phase() == Phase::Riding
                        {
                            self.trainer_error("trainer disconnected").await;
                        }
                        if connection_changed || !state.is_connected() {
                            self.latest_power_w = None;
                            self.latest_cadence_rpm = None;
                            self.power_smoothed_3s_w = None;
                            self.power_window.clear();
                            self.emit_player_measurement();
                        }
                    } else {
                        break;
                    }
                }
            }
        }
    }

    async fn handle_start(&mut self) {
        if self.engine.phase() != Phase::Ready {
            return;
        }

        match self.trainer.probe_connection().await {
            Ok(true) => {}
            Ok(false) => {
                let _ = self.app.emit(
                    "toast",
                    serde_json::json!({
                        "level": "warn",
                        "message": "Trainer is disconnected — reconnect it before starting.",
                    }),
                );
                return;
            }
            Err(e) => {
                warn!("trainer connection check failed: {e}");
                let _ = self.app.emit(
                    "toast",
                    serde_json::json!({
                        "level": "warn",
                        "message": format!("Could not check trainer connection: {e}"),
                    }),
                );
                return;
            }
        }

        if !self.heart_rate_monitor.is_connected() {
            let _ = self.app.emit("toast", serde_json::json!({
                "level": "info",
                "message": "Heart-rate monitor not connected — continuing without heart-rate data.",
            }));
        }

        self.ride_started_at = Some(Instant::now());
        self.write_event(SessionEventKind::Start, None);
        let actions = self.engine.step(EngineEvent::Start);
        self.apply_engine_actions(actions).await;
    }

    fn emit_player_measurement(&self) {
        let _ = self.app.emit(
            "player_measurement",
            PlayerMeasurement {
                power_w: self.latest_power_w,
                cadence_rpm: self.latest_cadence_rpm,
                heart_rate_bpm: self.latest_heart_rate_bpm,
                power_smoothed_3s_w: self.power_smoothed_3s_w,
            },
        );
    }

    /// Apply engine actions. Returns true when the ride finalized (workout
    /// completed naturally) and the task should exit.
    async fn apply_engine_actions(&mut self, actions: Vec<EngineAction>) -> bool {
        let complete = actions
            .iter()
            .any(|action| matches!(action, EngineAction::CompleteWorkout));
        self.execute_engine_actions(&actions).await;
        if complete {
            match self.finalize().await {
                Ok(summary) => {
                    let _ = self.app.emit("activity_recorded", &summary);
                }
                Err(e) => {
                    error!("finalize failed: {}", e.message);
                    let _ = self.app.emit(
                        "toast",
                        serde_json::json!({
                            "level": "error",
                            "message": format!("Activity save failed: {} (journal kept at {})",
                                                e.message, self.journal_path),
                        }),
                    );
                }
            }
            self.push_state();
            return true;
        }
        self.push_state();
        false
    }

    async fn execute_engine_actions(&mut self, actions: &[EngineAction]) {
        for action in actions {
            let r: Result<(), tp_ble::BleError> = match action {
                EngineAction::SetTargetPower { watts } => {
                    self.last_target_power_w = Some(*watts);
                    self.in_free_ride = false;
                    if self.erg_enabled {
                        self.trainer.set_target_power(*watts).await
                    } else {
                        Ok(()) // ERG off: track the target, don't drive it
                    }
                }
                EngineAction::EnterFreeRide => {
                    self.in_free_ride = true;
                    self.last_target_power_w = None;
                    self.write_event(SessionEventKind::FreerideEnter, None);
                    self.trainer.set_flat_road_simulation().await
                }
                EngineAction::StartTrainer => self.trainer.start_or_resume_training().await,
                EngineAction::StopTrainer => self.trainer.pause_training().await,
                EngineAction::ResetTrainer => self.trainer.reset_trainer().await,
                EngineAction::FinalizeSegment {
                    segment_index,
                    reason,
                } => {
                    let planned_duration_s =
                        self.engine.workout().segments[*segment_index].duration_s();
                    let result = self.segment_accumulator.take_result(
                        &self.workout_session_id,
                        *segment_index,
                        planned_duration_s,
                        *reason == SegmentEndReason::Skipped,
                    );
                    let _ = self.app.emit("segment_result", &result);
                    self.write_event(SessionEventKind::Lap, Some(*segment_index));
                    Ok(())
                }
                EngineAction::ShowText(t) => {
                    let _ = self.app.emit(
                        "text_event",
                        serde_json::json!({
                            "message": t.message, "duration_s": t.duration_s,
                        }),
                    );
                    Ok(())
                }
                EngineAction::CompleteWorkout => Ok(()),
            };
            if let Err(e) = r {
                self.trainer_error(&e.to_string()).await;
                break;
            }
        }
    }

    /// Trainer command failed mid-ride: auto-pause and tell the user
    /// The trainer owner reconnects in the background.
    async fn trainer_error(&mut self, msg: &str) {
        warn!("trainer error: {msg}");
        if self.engine.phase() == Phase::Riding {
            self.write_event(SessionEventKind::Pause, None);
            let actions = self.engine.step(EngineEvent::Pause);
            // Actions here are trainer stop ops that will likely also fail —
            // apply best-effort without recursing into trainer_error.
            for action in actions {
                if let EngineAction::StopTrainer = action {
                    let _ = self.trainer.pause_training().await;
                }
            }
            let _ = self.app.emit(
                "toast",
                serde_json::json!({
                    "level": "warn",
                    "message": format!("Paused: {msg}. Reconnecting…"),
                }),
            );
            self.push_state();
        }
    }

    fn t_ms(&self) -> u64 {
        self.ride_started_at
            .map(|started_at| started_at.elapsed().as_millis() as u64)
            .unwrap_or(0)
    }

    fn write_event(&mut self, kind: SessionEventKind, segment_index: Option<usize>) {
        let e = SessionEvent {
            t_ms: self.t_ms(),
            kind,
            segment_index,
        };
        if let Some(j) = self.journal.as_mut() {
            if let Err(err) = j.write_event(&e) {
                error!("journal write: {err}");
            }
        }
    }

    fn write_sample(&mut self) {
        self.segment_accumulator
            .note_sample(self.latest_power_w, self.latest_cadence_rpm);
        if let Some(p) = self.latest_power_w {
            self.power_sum += u64::from(p);
            self.power_n += 1;
        }
        self.live_power.push(self.latest_power_w.unwrap_or(0));
        if let Some(h) = self.latest_heart_rate_bpm {
            self.hr_sum += u64::from(h);
            self.hr_n += 1;
        }
        let s = Sample {
            t_ms: self.t_ms(),
            power_w: self.latest_power_w,
            cadence_rpm: self.latest_cadence_rpm,
            heart_rate_bpm: self.latest_heart_rate_bpm,
            target_power_w: if self.in_free_ride {
                None
            } else {
                self.last_target_power_w
            },
            target_cadence_rpm: self.current_target_cadence_rpm(),
        };
        if let Some(j) = self.journal.as_mut() {
            if let Err(err) = j.write_sample(&s) {
                error!("journal write: {err}");
            }
        }
    }

    fn current_target_cadence_rpm(&self) -> Option<u16> {
        let elapsed_s = (self.engine.active_ms() / 1000) as u32;
        self.engine.workout().target_cadence_rpm_at(elapsed_s)
    }

    fn push_state(&self) {
        let elapsed_s = (self.engine.active_ms() / 1000) as u32;
        let ride_s = (self.engine.ridden_ms() / 1000) as u32;
        let workout = self.engine.workout();
        let (seg_idx, seg_remaining_s) = match workout.segment_at(elapsed_s) {
            Some((i, into)) => (Some(i), workout.segments[i].duration_s() - into),
            None => (None, 0),
        };
        // Live totals use the same definitions as post-ride: NP over
        // the 1 Hz series, TSS off moving time, kJ = Σ power × 1 s. NP is only
        // meaningful once some power has arrived.
        let ftp_w = self.ftp_w;
        let normalized_power_w =
            (self.power_n > 0).then(|| normalized_power(&self.live_power));
        let average_heart_rate_bpm =
            (self.hr_n > 0).then(|| (self.hr_sum / u64::from(self.hr_n)) as u16);
        let ps = PlayerState {
            phase: phase_str(self.engine.phase()).into(),
            workout_session_id: self.workout_session_id.clone(),
            workout_definition_id: self.workout_definition_id.clone(),
            scheduled_workout_id: self.scheduled_workout_id.clone(),
            workout_name: workout.name.clone(),
            workout_duration_s: workout.duration_s(),
            seg_idx,
            seg_remaining_s,
            elapsed_s,
            ride_s,
            intensity: self.engine.intensity(),
            target_power_w: if self.in_free_ride {
                None
            } else {
                self.last_target_power_w
            },
            target_cadence_rpm: self.current_target_cadence_rpm(),
            average_power_w: (self.power_n > 0)
                .then(|| (self.power_sum / u64::from(self.power_n)) as u16),
            normalized_power_w,
            training_stress_score: normalized_power_w
                .filter(|_| ftp_w > 0)
                .map(|normalized_power_w| tss(ride_s, normalized_power_w, ftp_w)),
            // EF = NP / average HR. Needs both, and an HRM is optional.
            ef: match (normalized_power_w, average_heart_rate_bpm) {
                (Some(normalized_power_w), Some(heart_rate_bpm)) if heart_rate_bpm > 0 => {
                    Some(f64::from(normalized_power_w) / f64::from(heart_rate_bpm))
                }
                _ => None,
            },
            // Cycling convention: kJ of work ≈ kcal burned (the ~24 % human
            // efficiency and the J→cal factor very nearly cancel).
            kcal: (self.power_n > 0).then(|| (self.power_sum as f64 / 1000.0).round() as u32),
            erg_enabled: self.erg_enabled,
        };
        let _ = self.app.emit("player_state", &ps);
        self.state_tx.send_replace(ps);
    }

    /// Session finalization: close journal → replay → laps/totals → FIT →
    /// Activity row → summary. The journal survives any failure.
    async fn finalize(&mut self) -> Result<ActivitySummary, AppError> {
        self.write_event(SessionEventKind::End, None);
        if let Some(j) = self.journal.take() {
            let f = j.into_inner();
            let _ = f.sync_all();
        }

        let data = replay(BufReader::new(File::open(&self.journal_path)?))?;
        let laps = compute_laps(&data);
        let state = self.app.state::<AppState>();
        let settings = state.settings();
        let totals = session_totals(&data, data.header.ftp_w);

        let fit_bytes = tp_core::fit::encode_activity(&tp_core::fit::FitActivity {
            header: &data.header,
            samples: &data.samples,
            events: &data.events,
            laps: &laps,
            totals: &totals,
            record_distance: settings.record_distance,
        })?;
        let activity_id = uuid::Uuid::new_v4().to_string();
        let fit_path = state.activities_dir().join(format!("{activity_id}.fit"));
        std::fs::write(&fit_path, &fit_bytes)?;

        // Optional user export folder: copy with a friendly name. Failure is
        // non-fatal — the canonical copy in
        // the app data dir is already safe.
        if let Some(dir) = settings.export_dir.as_deref() {
            let safe_name: String = data
                .header
                .workout_name
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect();
            let (y, m, d) = ymd_utc(data.header.started_unix_ms / 1000);
            let dest = std::path::Path::new(dir).join(format!(
                "TrainerPro_{safe_name}_{y:04}-{m:02}-{d:02}_{}.fit",
                &activity_id[..8]
            ));
            if let Err(e) = std::fs::write(&dest, &fit_bytes) {
                let _ = self.app.emit(
                    "toast",
                    serde_json::json!({
                        "level": "warn",
                        "message": format!("Could not copy FIT to export folder: {e}"),
                    }),
                );
            }
        }

        let completed_pct = if self.engine.workout().duration_s() > 0 {
            f64::from((self.engine.active_ms() / 1000) as u32)
                / f64::from(self.engine.workout().duration_s())
                * 100.0
        } else {
            100.0
        }
        .min(100.0);

        let fit_path_string = fit_path.to_string_lossy().into_owned();
        {
            let conn = state.db.lock().unwrap();
            activity_db::insert(
                &conn,
                &activity_db::NewActivity {
                    id: &activity_id,
                    workout_session_id: &data.header.workout_session_id,
                    scheduled_workout_id: data.header.scheduled_workout_id.as_deref(),
                    workout_definition_id: Some(&data.header.workout_definition_id),
                    workout_definition_snapshot_json: &data
                        .header
                        .workout_definition_snapshot_json,
                    workout_name: &data.header.workout_name,
                    started_at_unix_ms: data.header.started_unix_ms as i64,
                    elapsed_s: totals.elapsed_s,
                    timer_s: totals.timer_s,
                    average_power_w: totals.average_power_w,
                    max_power_w: totals.max_power_w,
                    normalized_power_w: totals.normalized_power_w,
                    intensity_factor: totals.intensity_factor,
                    training_stress_score: totals.training_stress_score,
                    average_heart_rate_bpm: totals.average_heart_rate_bpm,
                    max_heart_rate_bpm: totals.max_heart_rate_bpm,
                    average_cadence_rpm: totals.average_cadence_rpm,
                    work_kj: totals.work_kj,
                    ftp_used_w: data.header.ftp_w,
                    final_intensity_multiplier: self.engine.intensity(),
                    fit_path: &fit_path_string,
                    journal_path: &self.journal_path,
                    completed_pct,
                },
            )?;
        }

        Ok(ActivitySummary {
            activity_id,
            scheduled_workout_id: data.header.scheduled_workout_id.clone(),
            workout_name: data.header.workout_name.clone(),
            started_at_unix_ms: data.header.started_unix_ms,
            elapsed_s: totals.elapsed_s,
            timer_s: totals.timer_s,
            average_power_w: totals.average_power_w,
            max_power_w: totals.max_power_w,
            normalized_power_w: totals.normalized_power_w,
            intensity_factor: totals.intensity_factor,
            training_stress_score: totals.training_stress_score,
            average_heart_rate_bpm: totals.average_heart_rate_bpm,
            max_heart_rate_bpm: totals.max_heart_rate_bpm,
            work_kj: totals.work_kj,
            completed_pct,
            fit_path: fit_path_string,
            laps: laps
                .iter()
                .map(|l| LapRow {
                    start_s: (l.start_ms / 1000) as u32,
                    duration_s: ((l.end_ms - l.start_ms) / 1000) as u32,
                    average_power_w: l.average_power_w,
                    max_power_w: l.max_power_w,
                    average_heart_rate_bpm: l.average_heart_rate_bpm,
                })
                .collect(),
        })
    }
}

/// Civil date (UTC) from a unix timestamp — days-to-civil algorithm, avoids
/// a chrono dependency for one filename.
fn ymd_utc(unix_s: u64) -> (u32, u32, u32) {
    let days = (unix_s / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::SegmentAccumulator;

    #[test]
    fn segment_result_uses_ridden_time_and_independent_sample_averages() {
        let mut accumulator = SegmentAccumulator::default();
        accumulator.note_tick(2_250);
        accumulator.note_sample(Some(200), Some(88));
        accumulator.note_sample(Some(220), None);

        let result = accumulator.take_result("session", 3, 300, true);

        assert_eq!(result.workout_session_id, "session");
        assert_eq!(result.segment_index, 3);
        assert_eq!(result.planned_duration_s, 300);
        assert_eq!(result.ridden_duration_s, 2);
        assert_eq!(result.average_power_w, Some(210));
        assert_eq!(result.average_cadence_rpm, Some(88));
        assert!(result.skipped);
    }

    #[test]
    fn taking_a_segment_result_resets_the_accumulator() {
        let mut accumulator = SegmentAccumulator::default();
        accumulator.note_tick(1_000);
        accumulator.note_sample(Some(250), Some(90));
        let _ = accumulator.take_result("session", 0, 60, false);

        let result = accumulator.take_result("session", 1, 30, false);

        assert_eq!(result.ridden_duration_s, 0);
        assert_eq!(result.average_power_w, None);
        assert_eq!(result.average_cadence_rpm, None);
        assert!(!result.skipped);
    }
}
