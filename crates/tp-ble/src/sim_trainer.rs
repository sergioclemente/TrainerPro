//! Simulated trainer. SPEC.md §4.5. Implements the same trait as the real
//! driver, with fault injection for development and CI.

use std::sync::{Arc, Mutex};

use tokio::sync::{broadcast, watch};
use tokio::time::Duration;

use crate::connection_tasks::ConnectionTasks;
use crate::traits::{BleError, ConnectionStatus, TrainerConnection, TrainerMeasurement};

const TICK_MS: u64 = 250;
const MEASUREMENT_CHANNEL_CAPACITY: usize = 32;
/// First-order power response time constant (SPEC §4.5).
const POWER_TAU_S: f64 = 1.5;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Erg,
    FreeRide,
}

#[derive(Debug)]
struct SimState {
    mode: Mode,
    target_w: f64,
    power_w: f64,
    running: bool,
    /// Fault injection: pretend the link dropped.
    dropped: bool,
    /// Fault injection: refuse the next control op.
    refuse_next: bool,
    rng: u64,
}

impl SimState {
    fn noise(&mut self, sd: f64) -> f64 {
        // xorshift64* → approx N(0,1) via sum of uniforms; deterministic,
        // dependency-free, good enough for a rider model.
        let mut sum = 0.0;
        for _ in 0..12 {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 7;
            self.rng ^= self.rng << 17;
            sum += (self.rng >> 11) as f64 / (1u64 << 53) as f64;
        }
        (sum - 6.0) * sd
    }
}

pub struct SimTrainer {
    tasks: ConnectionTasks,
    state: Arc<Mutex<SimState>>,
    measurements_tx: broadcast::Sender<TrainerMeasurement>,
    status_tx: watch::Sender<ConnectionStatus>,
}

impl Default for SimTrainer {
    fn default() -> Self {
        Self::new()
    }
}

impl SimTrainer {
    pub const ID: &'static str = "sim-trainer";

    pub fn new() -> Self {
        let mut tasks = ConnectionTasks::default();
        let state = Arc::new(Mutex::new(SimState {
            mode: Mode::Erg,
            target_w: 0.0,
            power_w: 0.0,
            running: false,
            dropped: false,
            refuse_next: false,
            rng: 0x9E37_79B9_7F4A_7C15,
        }));
        let (measurements_tx, _) = broadcast::channel(MEASUREMENT_CHANNEL_CAPACITY);
        let (status_tx, _) = watch::channel(ConnectionStatus::Connected);

        {
            let state = state.clone();
            let measurements_tx = measurements_tx.clone();
            tasks.push(tokio::spawn(async move {
                let dt = TICK_MS as f64 / 1000.0;
                let alpha = 1.0 - (-dt / POWER_TAU_S).exp();
                let mut interval = tokio::time::interval(Duration::from_millis(TICK_MS));
                loop {
                    interval.tick().await;
                    let data = {
                        let mut s = state.lock().unwrap();
                        if s.dropped {
                            continue;
                        }
                        let goal = match s.mode {
                            Mode::Erg => s.target_w,
                            // FreeRide: rider noodles around 130 W.
                            Mode::FreeRide => 130.0,
                        };
                        let goal = if s.running { goal } else { 0.0 };
                        s.power_w += (goal - s.power_w) * alpha;
                        let noise = s.noise(5.0);
                        let p = (s.power_w + noise).max(0.0);
                        let cadence = if goal > 0.0 {
                            (88.0 + s.noise(1.5)) as f32
                        } else {
                            0.0
                        };
                        TrainerMeasurement {
                            power_w: Some(p.round() as u16),
                            cadence_rpm: Some(cadence),
                            speed_kmh: Some((p / 10.0) as f32),
                        }
                    };
                    let _ = measurements_tx.send(data);
                }
            }));
        }
        SimTrainer {
            tasks,
            state,
            measurements_tx,
            status_tx,
        }
    }

    fn check_link(&self) -> Result<(), BleError> {
        let mut s = self.state.lock().unwrap();
        if s.dropped {
            return Err(BleError::Disconnected);
        }
        if s.refuse_next {
            s.refuse_next = false;
            return Err(BleError::ControlRefused(0x05));
        }
        Ok(())
    }

    // --- fault injection (tests / dev tools) ---
    pub fn inject_disconnect(&self) {
        self.state.lock().unwrap().dropped = true;
        self.status_tx.send_replace(ConnectionStatus::Disconnected);
    }
    pub fn restore_connection(&self) {
        self.state.lock().unwrap().dropped = false;
        self.status_tx.send_replace(ConnectionStatus::Connected);
    }
    pub fn refuse_next_op(&self) {
        self.state.lock().unwrap().refuse_next = true;
    }
}

#[async_trait::async_trait]
impl TrainerConnection for SimTrainer {
    async fn disconnect(&mut self) -> Result<(), BleError> {
        self.inject_disconnect();
        self.tasks.shutdown().await;
        Ok(())
    }
    async fn probe_connection(&self) -> Result<bool, BleError> {
        Ok(!self.state.lock().unwrap().dropped)
    }
    async fn set_target_power(&self, watts: u16) -> Result<(), BleError> {
        self.check_link()?;
        let mut s = self.state.lock().unwrap();
        s.mode = Mode::Erg;
        s.target_w = f64::from(watts);
        Ok(())
    }
    async fn set_flat_road_simulation(&self) -> Result<(), BleError> {
        self.check_link()?;
        self.state.lock().unwrap().mode = Mode::FreeRide;
        Ok(())
    }
    async fn start_or_resume_training(&self) -> Result<(), BleError> {
        self.check_link()?;
        self.state.lock().unwrap().running = true;
        Ok(())
    }
    async fn pause_training(&self) -> Result<(), BleError> {
        self.check_link()?;
        self.state.lock().unwrap().running = false;
        Ok(())
    }
    async fn reset_trainer(&self) -> Result<(), BleError> {
        self.check_link()?;
        let mut s = self.state.lock().unwrap();
        s.running = false;
        s.target_w = 0.0;
        s.mode = Mode::Erg;
        Ok(())
    }
    fn subscribe_measurements(&self) -> broadcast::Receiver<TrainerMeasurement> {
        self.measurements_tx.subscribe()
    }
    fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus> {
        self.status_tx.subscribe()
    }
    fn name(&self) -> &str {
        "Simulated KICKR"
    }
}
