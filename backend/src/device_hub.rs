//! Device coordination: shared Bluetooth discovery plus long-lived trainer
//! and heart-rate owners. Reconnect policy belongs to those role-specific owners.

use std::sync::Arc;

use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Manager};
use tracing::warn;

#[cfg(feature = "simulator")]
use tp_ble::ScanResult;
use tp_ble::{
    BleError, ConnectionPriority, DeviceManager, HeartRateConnection, Role, SimHrm, SimTrainer,
    TrainerConnection,
};

use crate::app_state::AppState;
use crate::device_owner::DeviceStatus;
use crate::heart_rate_monitor::{HeartRateConnector, HeartRateMonitor};
use crate::trainer::{Trainer, TrainerConnector};

const SCAN_DURATION_S: u64 = 10;
const DEVICE_MEASUREMENT_INTERVAL_S: u64 = 1;
const STARTUP_RECONNECT_DELAY_MS: u64 = 1_500;

struct TrainerConnections {
    manager: Arc<DeviceManager>,
}

#[async_trait]
impl TrainerConnector for TrainerConnections {
    async fn connect(
        &self,
        platform_id: &str,
        priority: ConnectionPriority,
    ) -> Result<Box<dyn TrainerConnection>, BleError> {
        if platform_id == SimTrainer::ID {
            #[cfg(not(feature = "simulator"))]
            return Err(BleError::NotFound);
            #[cfg(feature = "simulator")]
            {
                self.manager.quiesce_scan(priority).await;
                return Ok(Box::new(SimTrainer::new()));
            }
        }
        Ok(Box::new(
            self.manager.connect_trainer(platform_id, priority).await?,
        ))
    }
}

struct HeartRateConnections {
    manager: Arc<DeviceManager>,
    #[cfg(feature = "simulator")]
    trainer: Trainer,
}

#[async_trait]
impl HeartRateConnector for HeartRateConnections {
    async fn connect(
        &self,
        platform_id: &str,
        priority: ConnectionPriority,
    ) -> Result<Box<dyn HeartRateConnection>, BleError> {
        if platform_id == SimHrm::ID {
            #[cfg(not(feature = "simulator"))]
            return Err(BleError::NotFound);
            #[cfg(feature = "simulator")]
            {
                self.manager.quiesce_scan(priority).await;
                return Ok(Box::new(SimHrm::new(self.trainer.measurement_stream())));
            }
        }
        Ok(Box::new(
            self.manager.connect_hrm(platform_id, priority).await?,
        ))
    }
}

#[derive(Clone)]
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
            #[cfg(feature = "simulator")]
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
    pub name: Option<String>,
}

fn status_str(status: DeviceStatus) -> String {
    match status {
        DeviceStatus::Disconnected => "disconnected".into(),
        DeviceStatus::Connecting => "connecting".into(),
        DeviceStatus::Connected => "connected".into(),
        DeviceStatus::Reconnecting => "reconnecting".into(),
    }
}

fn emit_device_status(app: &AppHandle, role: Role, status: DeviceStatus, name: Option<String>) {
    let _ = app.emit(
        "device_status",
        DeviceStatusPayload {
            role,
            status: status_str(status),
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

    /// One time-boxed scan discovers trainer and heart-rate advertisements.
    pub async fn scan(&self, app: AppHandle) -> Result<(), BleError> {
        #[cfg(feature = "simulator")]
        {
            let simulators = [
                ScanResult {
                    platform_id: SimTrainer::ID.into(),
                    name: "Simulated KICKR".into(),
                    rssi: None,
                    role: Role::Trainer,
                },
                ScanResult {
                    platform_id: SimHrm::ID.into(),
                    name: "Simulated HRM".into(),
                    rssi: None,
                    role: Role::Hrm,
                },
            ];
            for simulator in simulators {
                let _ = app.emit("scan_result", &simulator);
            }
        }

        let app_for_result = app.clone();
        self.manager
            .scan(SCAN_DURATION_S, move |result| {
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

    /// Resume the saved selection and keep retrying until it connects or the
    /// user disconnects, forgets it, or selects a replacement.
    pub async fn maintain(&self, role: Role, platform_id: &str) -> Result<(), BleError> {
        let generation = match role {
            Role::Trainer => self.trainer.state().generation,
            Role::Hrm => self.heart_rate_monitor.state().generation,
        };
        self.maintain_if_generation(
            role,
            platform_id,
            generation,
            ConnectionPriority::Foreground,
        )
        .await
    }

    async fn maintain_if_generation(
        &self,
        role: Role,
        platform_id: &str,
        generation: u64,
        priority: ConnectionPriority,
    ) -> Result<(), BleError> {
        match role {
            Role::Trainer => {
                self.trainer
                    .maintain_if_generation(platform_id, generation, priority)
                    .await
            }
            Role::Hrm => {
                self.heart_rate_monitor
                    .maintain_if_generation(platform_id, generation, priority)
                    .await
            }
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
    tauri::async_runtime::spawn(async move {
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
    tauri::async_runtime::spawn(async move {
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
    tauri::async_runtime::spawn(async move {
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
    tauri::async_runtime::spawn(async move {
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

/// Discover both saved physical devices in one scan. Hand each role to its
/// owner as soon as it appears so setup can start while discovery continues.
/// Missing devices retry privately.
pub fn spawn_startup_reconnect(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(STARTUP_RECONNECT_DELAY_MS)).await;
        let state = app.state::<AppState>();
        let saved_rows: Vec<(String, String)> = {
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
        let saved: Vec<(Role, String)> = saved_rows
            .into_iter()
            .filter_map(|(role, platform_id)| match role.as_str() {
                "trainer" => Some((Role::Trainer, platform_id)),
                "hrm" => Some((Role::Hrm, platform_id)),
                _ => None,
            })
            .collect();

        let trainer_state = state.hub.trainer().state();
        let hrm_state = state.hub.heart_rate_monitor().state();

        // Simulators are an explicit QA choice and never auto-connect. Startup
        // also yields a role once the user has acted on it.
        let physical: Vec<(Role, String)> = saved
            .into_iter()
            .filter(|(role, platform_id)| {
                let owner_state = match role {
                    Role::Trainer => &trainer_state,
                    Role::Hrm => &hrm_state,
                };
                owner_state.generation == 0
                    && owner_state.platform_id.is_none()
                    && !matches!(
                        (role, platform_id.as_str()),
                        (Role::Trainer, SimTrainer::ID) | (Role::Hrm, SimHrm::ID)
                    )
            })
            .collect();
        let trainer_generation = trainer_state.generation;
        let hrm_generation = hrm_state.generation;
        let callback_targets = physical.clone();
        let callback_hub = state.hub.clone();
        let found = match state
            .hub
            .manager
            .discover_saved(&physical, move |role| {
                let Some((_, platform_id)) = callback_targets
                    .iter()
                    .find(|(target_role, _)| *target_role == role)
                else {
                    return;
                };
                let generation = match role {
                    Role::Trainer => trainer_generation,
                    Role::Hrm => hrm_generation,
                };
                let platform_id = platform_id.clone();
                let hub = callback_hub.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = hub
                        .maintain_if_generation(
                            role,
                            &platform_id,
                            generation,
                            ConnectionPriority::Startup,
                        )
                        .await
                    {
                        warn!("could not start saved-device recovery: {error}");
                    }
                });
            })
            .await
        {
            Ok(found) => found,
            Err(error) => {
                warn!("startup saved-device discovery failed: {error}");
                Vec::new()
            }
        };

        for (role, platform_id) in physical {
            if found.contains(&role) {
                continue;
            }
            let generation = match role {
                Role::Trainer => trainer_generation,
                Role::Hrm => hrm_generation,
            };
            let result = state
                .hub
                .maintain_if_generation(
                    role,
                    &platform_id,
                    generation,
                    ConnectionPriority::Background,
                )
                .await;
            if let Err(error) = result {
                warn!("could not start saved-device recovery: {error}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners_can_start_without_an_entered_tokio_runtime() {
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
