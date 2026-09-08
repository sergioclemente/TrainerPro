//! Simulated trainer + HRM. SPEC.md §4.5. Carries all development and CI:
//! implements the same traits as the real drivers, with fault injection.
//! Also surfaced in the UI device scan as "Simulator" entries so the full
//! app can be exercised without hardware.

use std::sync::{Arc, Mutex};

use crate::tasks::ConnectionTasks;
use futures::{Stream, StreamExt};

use tokio::sync::{broadcast, watch};
use tokio::time::Duration;

use crate::traits::{
    BleError, ConnectionStatus, HeartRateConnection, HeartRateMeasurement, TrainerConnection,
    TrainerMeasurement,
};

pub const SIM_TRAINER_ID: &str = "sim-trainer";
pub const SIM_HRM_ID: &str = "sim-hrm";

const TICK_MS: u64 = 250;
const HEART_RATE_TICK_S: u64 = 1;
const MEASUREMENT_CHANNEL_CAPACITY: usize = 32;
/// First-order power response time constant (SPEC §4.5).
const POWER_TAU_S: f64 = 1.5;
const HR_TAU_S: f64 = 25.0;

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

/// HR model: 60 bpm base + effort-proportional drive with a slow first-order
/// response (τ 25 s). Reads the trainer's measurements to know the effort.
pub struct SimHrm {
    tasks: ConnectionTasks,
    hr_tx: broadcast::Sender<HeartRateMeasurement>,
    status_tx: watch::Sender<ConnectionStatus>,
}

impl SimHrm {
    pub fn new(
        trainer_measurements: impl Stream<Item = TrainerMeasurement> + Send + 'static,
    ) -> Self {
        let mut tasks = ConnectionTasks::default();
        let mut trainer_measurements = Box::pin(trainer_measurements);
        let (hr_tx, _) = broadcast::channel(MEASUREMENT_CHANNEL_CAPACITY);
        let (status_tx, _) = watch::channel(ConnectionStatus::Connected);
        {
            let hr_tx = hr_tx.clone();
            let status_tx = status_tx.clone();
            tasks.push(tokio::spawn(async move {
                let mut hr = 60.0f64;
                let mut last_power = 0.0f64;
                let mut interval =
                    tokio::time::interval(Duration::from_secs(HEART_RATE_TICK_S));
                loop {
                    tokio::select! {
                        r = trainer_measurements.next() => match r {
                            Some(d) => {
                                if let Some(p) = d.power_w { last_power = f64::from(p); }
                            }
                            None => break,
                        },
                        _ = interval.tick() => {
                            if *status_tx.borrow() == ConnectionStatus::Disconnected {
                                continue;
                            }
                            // A missing sample in the next interval means no
                            // current effort; do not preserve stale trainer
                            // power across a disconnect.
                            let observed_power = last_power;
                            last_power = 0.0;
                            let goal = 60.0 + 110.0 * (observed_power / 250.0).min(1.4);
                            let alpha = 1.0 - (-1.0 / HR_TAU_S).exp();
                            hr += (goal - hr) * alpha;
                            let _ = hr_tx.send(HeartRateMeasurement { bpm: hr.round() as u16 });
                        }
                    }
                }
                status_tx.send_replace(ConnectionStatus::Disconnected);
            }));
        }
        SimHrm {
            tasks,
            hr_tx,
            status_tx,
        }
    }

    pub fn inject_disconnect(&self) {
        self.status_tx.send_replace(ConnectionStatus::Disconnected);
    }

    pub fn restore_connection(&self) {
        self.status_tx.send_replace(ConnectionStatus::Connected);
    }
}

#[async_trait::async_trait]
impl HeartRateConnection for SimHrm {
    async fn disconnect(&mut self) -> Result<(), BleError> {
        self.inject_disconnect();
        self.tasks.shutdown().await;
        Ok(())
    }
    fn subscribe_measurements(&self) -> broadcast::Receiver<HeartRateMeasurement> {
        self.hr_tx.subscribe()
    }
    fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus> {
        self.status_tx.subscribe()
    }
    fn name(&self) -> &str {
        "Simulated HRM"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn sim_converges_to_target() {
        let sim = SimTrainer::new();
        let mut rx = sim.subscribe_measurements();
        sim.start_or_resume_training().await.unwrap();
        sim.set_target_power(200).await.unwrap();
        // τ=1.5 s ⇒ well converged after 10 s of virtual time.
        let mut last = 0u16;
        for _ in 0..40 {
            tokio::time::advance(Duration::from_millis(TICK_MS)).await;
            if let Ok(d) = rx.try_recv() {
                last = d.power_w.unwrap_or(0);
            }
            while rx.try_recv().is_ok() {}
        }
        assert!((170..=230).contains(&last), "power={last} not near 200 W");
    }

    #[tokio::test(start_paused = true)]
    async fn fault_injection_disconnect_and_restore() {
        let sim = SimTrainer::new();
        let mut status = sim.subscribe_status();
        sim.inject_disconnect();
        status.changed().await.unwrap();
        assert!(!sim.probe_connection().await.unwrap());
        assert!(matches!(
            sim.set_target_power(150).await,
            Err(BleError::Disconnected)
        ));
        assert_eq!(*status.borrow(), ConnectionStatus::Disconnected);
        sim.restore_connection();
        status.changed().await.unwrap();
        assert!(sim.probe_connection().await.unwrap());
        assert!(sim.set_target_power(150).await.is_ok());
        assert_eq!(*status.borrow(), ConnectionStatus::Connected);
    }

    #[tokio::test(start_paused = true)]
    async fn hrm_fault_injection_updates_existing_status_stream() {
        let hrm = SimHrm::new(futures::stream::pending());
        let mut status = hrm.subscribe_status();

        hrm.inject_disconnect();
        status.changed().await.unwrap();
        assert_eq!(*status.borrow(), ConnectionStatus::Disconnected);
        hrm.restore_connection();
        status.changed().await.unwrap();
        assert_eq!(*status.borrow(), ConnectionStatus::Connected);
    }

    #[tokio::test(start_paused = true)]
    async fn refuse_next_op_fires_once() {
        let sim = SimTrainer::new();
        sim.refuse_next_op();
        assert!(matches!(
            sim.start_or_resume_training().await,
            Err(BleError::ControlRefused(_))
        ));
        assert!(sim.start_or_resume_training().await.is_ok());
    }
}
