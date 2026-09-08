//! Device coordination: shared Bluetooth discovery plus long-lived trainer
//! and heart-rate owners. Reconnect policy belongs to those role-specific owners.

use std::sync::Arc;

use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{info, warn};

use tp_ble::{
    BleError, DeviceManager, HeartRateConnection, Role, ScanResult, SimHrm, SimTrainer,
    TrainerConnection,
};

use crate::device::DeviceStatus;
use crate::heart_rate_monitor::{HeartRateConnector, HeartRateMonitor};
use crate::state::AppState;
use crate::trainer::{Trainer, TrainerConnector};

const SCAN_DURATION_S: u64 = 10;
const DEVICE_MEASUREMENT_INTERVAL_S: u64 = 1;
const STARTUP_RECONNECT_DELAY_MS: u64 = 1_500;
const STARTUP_RECONNECT_ATTEMPTS: u32 = 2;
const STARTUP_RECONNECT_RETRY_S: u64 = 5;

struct TrainerConnections {
    manager: Arc<DeviceManager>,
}

#[async_trait]
impl TrainerConnector for TrainerConnections {
    async fn connect(&self, platform_id: &str) -> Result<Box<dyn TrainerConnection>, BleError> {
        if platform_id == SimTrainer::ID {
            self.manager.cancel_scan().await;
            return Ok(Box::new(SimTrainer::new()));
        }
        Ok(Box::new(self.manager.connect_trainer(platform_id).await?))
    }
}

struct HeartRateConnections {
    manager: Arc<DeviceManager>,
    trainer: Trainer,
}

#[async_trait]
impl HeartRateConnector for HeartRateConnections {
    async fn connect(&self, platform_id: &str) -> Result<Box<dyn HeartRateConnection>, BleError> {
        if platform_id == SimHrm::ID {
            self.manager.cancel_scan().await;
            return Ok(Box::new(SimHrm::new(self.trainer.measurement_stream())));
        }
        Ok(Box::new(self.manager.connect_hrm(platform_id).await?))
    }
}

pub struct DeviceHub {
    manager: Arc<DeviceManager>,
    trainer: Trainer,
    heart_rate_monitor: HeartRateMonitor,
}

impl Default for DeviceHub {
    fn default() -> Self {
        let manager = Arc::new(DeviceManager::new());
        let trainer = Trainer::new(Arc::new(TrainerConnections {
            manager: manager.clone(),
        }));
        let heart_rate_monitor = HeartRateMonitor::new(Arc::new(HeartRateConnections {
            manager: manager.clone(),
            trainer: trainer.clone(),
        }));
        Self {
            manager,
            trainer,
            heart_rate_monitor,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceStatusPayload {
    pub role: Role,
    pub status: String,
    pub attempt: Option<u32>,
    pub name: Option<String>,
}

fn status_str(status: DeviceStatus) -> (String, Option<u32>) {
    match status {
        DeviceStatus::Disconnected => ("disconnected".into(), None),
        DeviceStatus::Connecting => ("connecting".into(), None),
        DeviceStatus::Connected => ("connected".into(), None),
        DeviceStatus::Reconnecting { attempt } => ("reconnecting".into(), Some(attempt)),
    }
}

fn emit_device_status(app: &AppHandle, role: Role, status: DeviceStatus, name: Option<String>) {
    let (status, attempt) = status_str(status);
    let _ = app.emit(
        "device_status",
        DeviceStatusPayload {
            role,
            status,
            attempt,
            name,
        },
    );
}

impl DeviceHub {
    pub fn trainer(&self) -> &Trainer {
        &self.trainer
    }

    pub fn heart_rate_monitor(&self) -> &HeartRateMonitor {
        &self.heart_rate_monitor
    }

    pub fn trainer_connected(&self) -> bool {
        self.trainer.is_connected()
    }

    pub fn heart_rate_monitor_connected(&self) -> bool {
        self.heart_rate_monitor.is_connected()
    }

    /// Start the application event bridges once. Their subscriptions remain
    /// valid while the owners replace connections underneath them.
    pub fn start_event_forwarders(&self, app: AppHandle) {
        spawn_trainer_state_forwarder(app.clone(), self.trainer.clone());
        spawn_heart_rate_state_forwarder(app.clone(), self.heart_rate_monitor.clone());
        spawn_trainer_measurement_forwarder(app.clone(), self.trainer.clone());
        spawn_heart_rate_measurement_forwarder(app, self.heart_rate_monitor.clone());
    }

    /// Time-boxed scan; simulator entries appear before physical devices.
    pub async fn scan(&self, app: AppHandle, role: Role) -> Result<(), BleError> {
        let sim = match role {
            Role::Trainer => ScanResult {
                platform_id: SimTrainer::ID.into(),
                name: "Simulated KICKR".into(),
                rssi: None,
                role,
            },
            Role::Hrm => ScanResult {
                platform_id: SimHrm::ID.into(),
                name: "Simulated HRM".into(),
                rssi: None,
                role,
            },
        };
        let _ = app.emit("scan_result", &sim);

        let app_for_result = app.clone();
        self.manager
            .scan(role, SCAN_DURATION_S, move |result| {
                let _ = app_for_result.emit("scan_result", &result);
            })
            .await
    }

    pub async fn connect(&self, role: Role, platform_id: &str) -> Result<String, BleError> {
        match role {
            Role::Trainer => self.trainer.connect(platform_id).await,
            Role::Hrm => self.heart_rate_monitor.connect(platform_id).await,
        }
    }

    pub async fn disconnect(&self, role: Role) -> Result<(), BleError> {
        match role {
            Role::Trainer => self.trainer.disconnect().await,
            Role::Hrm => self.heart_rate_monitor.disconnect().await,
        }
    }
}

fn spawn_trainer_state_forwarder(app: AppHandle, trainer: Trainer) {
    let mut state_rx = trainer.subscribe_state();
    tokio::spawn(async move {
        while state_rx.changed().await.is_ok() {
            let state = state_rx.borrow_and_update().clone();
            let is_connected = state.is_connected();
            emit_device_status(&app, Role::Trainer, state.status, state.name);
            if !is_connected {
                let _ = app.emit(
                    "device_measurement",
                    serde_json::json!({
                        "role": "trainer", "power_w": null, "cadence_rpm": null,
                    }),
                );
            }
        }
    });
}

fn spawn_heart_rate_state_forwarder(app: AppHandle, monitor: HeartRateMonitor) {
    let mut state_rx = monitor.subscribe_state();
    tokio::spawn(async move {
        while state_rx.changed().await.is_ok() {
            let state = state_rx.borrow_and_update().clone();
            let is_connected = state.is_connected();
            emit_device_status(&app, Role::Hrm, state.status, state.name);
            if !is_connected {
                let _ = app.emit(
                    "device_measurement",
                    serde_json::json!({
                        "role": "hrm", "heart_rate_bpm": null,
                    }),
                );
            }
        }
    });
}

/// Latest trainer measurement for the Devices screen, sampled at 1 Hz.
fn spawn_trainer_measurement_forwarder(app: AppHandle, trainer: Trainer) {
    let mut measurements = trainer.subscribe_measurements();
    let state = trainer.subscribe_state();
    tokio::spawn(async move {
        let mut latest = None;
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(
            DEVICE_MEASUREMENT_INTERVAL_S,
        ));
        loop {
            tokio::select! {
                result = measurements.recv() => match result {
                    Ok(sample) if state.borrow().accepts(&sample) => latest = Some(sample),
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
                _ = tick.tick() => {
                    if let Some(sample) = latest.filter(|sample| state.borrow().accepts(sample)) {
                        let measurement = sample.value;
                        let _ = app.emit("device_measurement", serde_json::json!({
                            "role": "trainer",
                            "power_w": measurement.power_w,
                            "cadence_rpm": measurement.cadence_rpm.map(|rpm| rpm.round() as u16),
                        }));
                    }
                }
            }
        }
    });
}

fn spawn_heart_rate_measurement_forwarder(app: AppHandle, monitor: HeartRateMonitor) {
    let mut measurements = monitor.subscribe_measurements();
    let state = monitor.subscribe_state();
    tokio::spawn(async move {
        loop {
            match measurements.recv().await {
                Ok(sample) if state.borrow().accepts(&sample) => {
                    let _ = app.emit(
                        "device_measurement",
                        serde_json::json!({
                            "role": "hrm", "heart_rate_bpm": sample.value.bpm,
                        }),
                    );
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

/// On startup, try the saved devices in trainer-then-HRM order. Once an
/// initial connection succeeds, each owner maintains its own reconnect loop.
pub fn spawn_startup_reconnect(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(STARTUP_RECONNECT_DELAY_MS)).await;
        let state = app.state::<AppState>();
        let mut saved: Vec<(String, String)> = {
            let conn = state.db.lock().unwrap();
            let Ok(mut stmt) = conn.prepare("SELECT role, platform_id FROM devices") else {
                return;
            };
            match stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
            {
                Ok(saved) => saved,
                Err(_) => return,
            }
        };
        saved.sort_by_key(|(role, _)| if role == "trainer" { 0 } else { 1 });

        for (role_name, platform_id) in saved {
            let role = match role_name.as_str() {
                "trainer" => Role::Trainer,
                "hrm" => Role::Hrm,
                _ => continue,
            };
            let initial_state = match role {
                Role::Trainer => state.hub.trainer().state(),
                Role::Hrm => state.hub.heart_rate_monitor().state(),
            };
            // Any earlier user action owns this role, including an explicit
            // disconnect. Startup recovery must never supersede it.
            if initial_state.generation != 0 || initial_state.platform_id.is_some() {
                continue;
            }
            let mut expected_generation = initial_state.generation;
            for attempt in 1..=STARTUP_RECONNECT_ATTEMPTS {
                let owner_state = match role {
                    Role::Trainer => state.hub.trainer().state(),
                    Role::Hrm => state.hub.heart_rate_monitor().state(),
                };
                if owner_state.is_connected()
                    || owner_state.generation != expected_generation
                    || (attempt == 1 && owner_state.platform_id.is_some())
                    || (attempt > 1
                        && owner_state.platform_id.as_deref() != Some(platform_id.as_str()))
                {
                    break;
                }
                let result = match role {
                    Role::Trainer => {
                        state
                            .hub
                            .trainer()
                            .connect_if_generation(&platform_id, owner_state.generation)
                            .await
                    }
                    Role::Hrm => {
                        state
                            .hub
                            .heart_rate_monitor()
                            .connect_if_generation(&platform_id, owner_state.generation)
                            .await
                    }
                };
                match result {
                    Ok(name) => {
                        info!("startup reconnect: {role_name} \"{name}\" connected");
                        break;
                    }
                    Err(error) => {
                        warn!("startup reconnect {role_name} attempt {attempt} failed: {error}");
                        if attempt < STARTUP_RECONNECT_ATTEMPTS {
                            let failed_state = match role {
                                Role::Trainer => state.hub.trainer().state(),
                                Role::Hrm => state.hub.heart_rate_monitor().state(),
                            };
                            if failed_state.platform_id.as_deref() != Some(platform_id.as_str()) {
                                break;
                            }
                            expected_generation = failed_state.generation;
                            tokio::time::sleep(std::time::Duration::from_secs(
                                STARTUP_RECONNECT_RETRY_S,
                            ))
                            .await;
                        }
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn owners_start_disconnected() {
        let hub = DeviceHub::default();
        assert!(!hub.trainer_connected());
        assert!(!hub.heart_rate_monitor_connected());
        assert_eq!(hub.trainer().state().status, DeviceStatus::Disconnected);
        assert_eq!(
            hub.heart_rate_monitor().state().status,
            DeviceStatus::Disconnected
        );
    }
}
