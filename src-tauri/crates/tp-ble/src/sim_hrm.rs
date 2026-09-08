//! Simulated heart-rate monitor. SPEC.md §4.5. Implements the same trait as
//! the real driver and follows simulated trainer measurements.

use futures::{Stream, StreamExt};
use tokio::sync::{broadcast, watch};
use tokio::time::Duration;

use crate::connection_tasks::ConnectionTasks;
use crate::traits::{
    BleError, ConnectionStatus, HeartRateConnection, HeartRateMeasurement, TrainerMeasurement,
};

const HEART_RATE_TICK_S: u64 = 1;
const MEASUREMENT_CHANNEL_CAPACITY: usize = 32;
const HR_TAU_S: f64 = 25.0;

/// HR model: 60 bpm base + effort-proportional drive with a slow first-order
/// response (τ 25 s). Reads the trainer's measurements to know the effort.
pub struct SimHrm {
    tasks: ConnectionTasks,
    hr_tx: broadcast::Sender<HeartRateMeasurement>,
    status_tx: watch::Sender<ConnectionStatus>,
}

impl SimHrm {
    pub const ID: &'static str = "sim-hrm";

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
                let mut interval = tokio::time::interval(Duration::from_secs(HEART_RATE_TICK_S));
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
