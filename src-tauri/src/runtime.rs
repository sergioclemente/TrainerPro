//! Player runtime: the async task that drives Engine effects into the
//! trainer, records the journal, and finalizes the ride into a FIT file.
//! SPEC.md §5.2, §6, §7. The engine stays pure; all time and I/O live here.

use std::collections::VecDeque;
use std::fs::File;
use std::io::BufReader;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{error, warn};

use tp_core::consts::{ENGINE_TICK_MS, ERG_KEEPALIVE_S, SAMPLE_HZ, SMOOTH_WINDOW_S};
use tp_core::engine::{Effect, Engine, Input, Phase};
use tp_core::journal::{
    compute_laps, replay, JournalHeader, JournalWriter, RideEvent, RideEventKind, Sample,
};
use tp_core::metrics::{normalized_power, session_totals, tss};
use tp_core::model::Workout;

use crate::err::AppError;
use crate::heart_rate_monitor::HeartRateMonitor;
use crate::state::{now_unix_ms, AppState};
use crate::trainer::Trainer;

const PLAYER_COMMAND_CAPACITY: usize = 16;

pub enum Cmd {
    Start,
    Pause,
    Resume,
    Skip,
    SetIntensity(f64),
    /// Toggle ERG mode. Off = trainer switches to simulation grade 0 (free
    /// resistance); workout targets keep advancing but aren't sent.
    SetErg(bool),
    End(oneshot::Sender<Result<RideSummary, AppError>>),
}

pub struct PlayerHandle {
    pub cmd_tx: mpsc::Sender<Cmd>,
    pub state_rx: watch::Receiver<PlayerState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayerState {
    pub phase: String,
    pub workout_id: String,
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
    pub target: Option<u16>,
    pub avg_power: Option<u16>,
    /// Live session totals, mirroring the post-ride numbers (SPEC §6).
    /// `None` until there is data to compute them from.
    pub np: Option<u16>,
    pub tss: Option<f64>,
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
pub struct LapRow {
    pub start_s: u32,
    pub duration_s: u32,
    pub avg_power: Option<u16>,
    pub max_power: Option<u16>,
    pub avg_hr: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RideSummary {
    pub ride_id: String,
    pub workout_name: String,
    pub started_at: u64,
    pub elapsed_s: u32,
    pub timer_s: u32,
    pub avg_power: Option<u16>,
    pub max_power: Option<u16>,
    pub np: Option<u16>,
    pub if_: Option<f64>,
    pub tss: Option<f64>,
    pub avg_hr: Option<u16>,
    pub max_hr: Option<u16>,
    pub kj: u32,
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
    workout_id: String,
    workout: Workout,
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
    let ride_id = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(state.rides_dir())?;
    let journal_path = state.rides_dir().join(format!("{ride_id}.jsonl"));
    let header = JournalHeader {
        ride_id: ride_id.clone(),
        started_unix_ms: now_unix_ms(),
        workout_name: workout.name.clone(),
        ftp: settings.profile.ftp,
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
    let (cmd_tx, cmd_rx) = mpsc::channel(PLAYER_COMMAND_CAPACITY);
    let initial = PlayerState {
        phase: "ready".into(),
        workout_id: workout_id.clone(),
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
        target: None,
        avg_power: None,
        np: None,
        tss: None,
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
        ride_id,
        workout_id: workout_id.clone(),
        header_started_ms: now_unix_ms(),
        ride_started_at: None,
        state_tx,
        last_target: None,
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
        ftp: settings.profile.ftp,
    };
    tokio::spawn(rt.run(cmd_rx));
    Ok(PlayerHandle { cmd_tx, state_rx })
}

struct Runtime {
    app: AppHandle,
    engine: Engine,
    trainer: Trainer,
    heart_rate_monitor: HeartRateMonitor,
    journal: Option<JournalWriter<File>>,
    journal_path: String,
    ride_id: String,
    workout_id: String,
    header_started_ms: u64,
    /// Wall-clock ride origin, set on Start. Journal t_ms is measured from
    /// here (includes paused spans in the timeline; no samples during pause).
    ride_started_at: Option<Instant>,
    state_tx: watch::Sender<PlayerState>,
    last_target: Option<u16>,
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
    /// Rider FTP at load time — TSS and IF are relative to it.
    ftp: u16,
}

impl Runtime {
    async fn run(mut self, mut cmd_rx: mpsc::Receiver<Cmd>) {
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
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        Cmd::Start => {
                            self.handle_start().await;
                        }
                        Cmd::Pause => {
                            self.write_event(RideEventKind::Pause, None);
                            let fx = self.engine.handle(Input::Pause);
                            self.apply(fx).await;
                        }
                        Cmd::Resume => {
                            self.write_event(RideEventKind::Resume, None);
                            let fx = self.engine.handle(Input::Resume);
                            self.apply(fx).await;
                        }
                        Cmd::Skip => {
                            let fx = self.engine.handle(Input::SkipSegment);
                            self.apply(fx).await;
                        }
                        Cmd::SetIntensity(i) => {
                            let fx = self.engine.handle(Input::SetIntensity(i));
                            self.apply(fx).await;
                        }
                        Cmd::SetErg(on) => {
                            self.erg_enabled = on;
                            let r = if on {
                                match self.last_target {
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
                        Cmd::End(reply) => {
                            let fx = self.engine.handle(Input::End);
                            self.apply_effects_no_finalize(&fx).await;
                            let _ = reply.send(self.finalize().await);
                            break;
                        }
                    }
                    self.push_state();
                }
                _ = engine_tick.tick() => {
                    if self.engine.phase() == Phase::Riding {
                        let fx = self.engine.handle(Input::Tick { dt_ms: ENGINE_TICK_MS });
                        let finished = self.apply(fx).await;
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
                        if let Some(w) = self.last_target {
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
        self.write_event(RideEventKind::Start, None);
        let fx = self.engine.handle(Input::Start);
        self.apply(fx).await;
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

    /// Apply engine effects. Returns true when the ride finalized (workout
    /// completed naturally) and the task should exit.
    async fn apply(&mut self, fx: Vec<Effect>) -> bool {
        let complete = fx.iter().any(|e| matches!(e, Effect::WorkoutComplete));
        self.apply_effects_no_finalize(&fx).await;
        if complete {
            match self.finalize().await {
                Ok(summary) => {
                    let _ = self.app.emit("ride_finished", &summary);
                }
                Err(e) => {
                    error!("finalize failed: {}", e.message);
                    let _ = self.app.emit(
                        "toast",
                        serde_json::json!({
                            "level": "error",
                            "message": format!("Ride save failed: {} (journal kept at {})",
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

    async fn apply_effects_no_finalize(&mut self, fx: &[Effect]) {
        for eff in fx {
            let r: Result<(), tp_ble::BleError> = match eff {
                Effect::SetTarget(w) => {
                    self.last_target = Some(*w);
                    self.in_free_ride = false;
                    if self.erg_enabled {
                        self.trainer.set_target_power(*w).await
                    } else {
                        Ok(()) // ERG off: track the target, don't drive it
                    }
                }
                Effect::EnterFreeRide => {
                    self.in_free_ride = true;
                    self.last_target = None;
                    self.write_event(RideEventKind::FreerideEnter, None);
                    self.trainer.set_flat_road_simulation().await
                }
                Effect::TrainerStart => self.trainer.start_or_resume_training().await,
                Effect::TrainerStop => self.trainer.pause_training().await,
                Effect::TrainerReset => self.trainer.reset_trainer().await,
                Effect::LapBoundary { seg_idx } => {
                    self.write_event(RideEventKind::Lap, Some(*seg_idx));
                    Ok(())
                }
                Effect::ShowText(t) => {
                    let _ = self.app.emit(
                        "text_event",
                        serde_json::json!({
                            "message": t.message, "duration_s": t.duration_s,
                        }),
                    );
                    Ok(())
                }
                Effect::WorkoutComplete => Ok(()),
            };
            if let Err(e) = r {
                self.trainer_error(&e.to_string()).await;
                break;
            }
        }
    }

    /// Trainer command failed mid-ride: auto-pause and tell the user
    /// (SPEC §5.4). The trainer owner reconnects in the background.
    async fn trainer_error(&mut self, msg: &str) {
        warn!("trainer error: {msg}");
        if self.engine.phase() == Phase::Riding {
            self.write_event(RideEventKind::Pause, None);
            let fx = self.engine.handle(Input::Pause);
            // Effects here are trainer stop ops that will likely also fail —
            // apply best-effort without recursing into trainer_error.
            for eff in fx {
                if let Effect::TrainerStop = eff {
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

    fn write_event(&mut self, kind: RideEventKind, seg: Option<usize>) {
        let e = RideEvent {
            t_ms: self.t_ms(),
            kind,
            seg,
        };
        if let Some(j) = self.journal.as_mut() {
            if let Err(err) = j.write_event(&e) {
                error!("journal write: {err}");
            }
        }
    }

    fn write_sample(&mut self) {
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
            power: self.latest_power_w,
            cadence: self.latest_cadence_rpm,
            hr: self.latest_heart_rate_bpm,
            target: if self.in_free_ride { None } else { self.last_target },
        };
        if let Some(j) = self.journal.as_mut() {
            if let Err(err) = j.write_sample(&s) {
                error!("journal write: {err}");
            }
        }
    }

    fn push_state(&self) {
        let elapsed_s = (self.engine.active_ms() / 1000) as u32;
        let ride_s = (self.engine.ridden_ms() / 1000) as u32;
        let workout = self.engine.workout();
        let (seg_idx, seg_remaining_s) = match workout.segment_at(elapsed_s) {
            Some((i, into)) => (Some(i), workout.segments[i].duration_s() - into),
            None => (None, 0),
        };
        // Live totals, computed exactly as §6 computes them post-ride: NP over
        // the 1 Hz series, TSS off moving time, kJ = Σ power × 1 s. NP is only
        // meaningful once some power has arrived.
        let ftp = self.ftp;
        let np = (self.power_n > 0).then(|| normalized_power(&self.live_power));
        let avg_hr = (self.hr_n > 0).then(|| (self.hr_sum / u64::from(self.hr_n)) as u16);
        let ps = PlayerState {
            phase: phase_str(self.engine.phase()).into(),
            workout_id: self.workout_id.clone(),
            workout_name: workout.name.clone(),
            workout_duration_s: workout.duration_s(),
            seg_idx,
            seg_remaining_s,
            elapsed_s,
            ride_s,
            intensity: self.engine.intensity(),
            target: if self.in_free_ride { None } else { self.last_target },
            avg_power: (self.power_n > 0)
                .then(|| (self.power_sum / u64::from(self.power_n)) as u16),
            np,
            tss: np.filter(|_| ftp > 0).map(|np| tss(ride_s, np, ftp)),
            // EF = NP / average HR. Needs both, and an HRM is optional.
            ef: match (np, avg_hr) {
                (Some(np), Some(hr)) if hr > 0 => Some(f64::from(np) / f64::from(hr)),
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

    /// End-of-ride pipeline: close journal → replay → laps/totals → FIT →
    /// DB row → summary. SPEC §6–§7. The journal survives any failure here.
    async fn finalize(&mut self) -> Result<RideSummary, AppError> {
        self.write_event(RideEventKind::End, None);
        if let Some(j) = self.journal.take() {
            let f = j.into_inner();
            let _ = f.sync_all();
        }

        let data = replay(BufReader::new(File::open(&self.journal_path)?))?;
        let laps = compute_laps(&data);
        let state = self.app.state::<AppState>();
        let settings = state.settings();
        let totals = session_totals(&data, data.header.ftp);

        let fit_bytes = tp_core::fit::encode_activity(&tp_core::fit::FitRide {
            header: &data.header,
            samples: &data.samples,
            events: &data.events,
            laps: &laps,
            totals: &totals,
            record_distance: settings.record_distance,
        })?;
        let fit_path = state.rides_dir().join(format!("{}.fit", self.ride_id));
        std::fs::write(&fit_path, &fit_bytes)?;

        // Optional user export folder: copy with a friendly name (SPEC §8
        // export_dir setting). Failure is non-fatal — the canonical copy in
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
                &self.ride_id[..8]
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

        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO rides (id, workout_id, workout_name, started_at, elapsed_s,
                   timer_s, avg_power, max_power, np, if_, tss, avg_hr, max_hr, avg_cadence,
                   kj, ftp_used, intensity_final, fit_path, journal_path, completed_pct)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
                rusqlite::params![
                    self.ride_id,
                    self.workout_id,
                    data.header.workout_name,
                    self.header_started_ms as i64,
                    totals.elapsed_s,
                    totals.timer_s,
                    totals.avg_power,
                    totals.max_power,
                    totals.np,
                    totals.if_,
                    totals.tss,
                    totals.avg_hr,
                    totals.max_hr,
                    totals.avg_cadence,
                    totals.kj,
                    data.header.ftp,
                    self.engine.intensity(),
                    fit_path.to_string_lossy(),
                    self.journal_path,
                    completed_pct,
                ],
            )?;
        }

        Ok(RideSummary {
            ride_id: self.ride_id.clone(),
            workout_name: data.header.workout_name.clone(),
            started_at: data.header.started_unix_ms,
            elapsed_s: totals.elapsed_s,
            timer_s: totals.timer_s,
            avg_power: totals.avg_power,
            max_power: totals.max_power,
            np: totals.np,
            if_: totals.if_,
            tss: totals.tss,
            avg_hr: totals.avg_hr,
            max_hr: totals.max_hr,
            kj: totals.kj,
            completed_pct,
            fit_path: fit_path.to_string_lossy().into_owned(),
            laps: laps
                .iter()
                .map(|l| LapRow {
                    start_s: (l.start_ms / 1000) as u32,
                    duration_s: ((l.end_ms - l.start_ms) / 1000) as u32,
                    avg_power: l.avg_power,
                    max_power: l.max_power,
                    avg_hr: l.avg_hr,
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
