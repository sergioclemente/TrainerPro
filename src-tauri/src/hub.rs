//! Device hub: owns the connected trainer/HRM slots and the reconnect
//! policy (SPEC.md §4.4 — schedule in tp_core::consts). Simulator devices
//! are always offered in scans so the app runs without hardware.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{OnceCell, RwLock};
use tracing::{info, warn};

use tp_ble::sim::{SimHrm, SimTrainer, SIM_HRM_ID, SIM_TRAINER_ID};
use tp_ble::{
    BleError, DeviceManager, DeviceStatus, HeartRateMonitor, Role, ScanResult, Trainer,
};
use tp_core::consts::{RECONNECT_SCHEDULE_S, RECONNECT_STEADY_S};

use crate::state::AppState;

#[derive(Default)]
pub struct DeviceHub {
    manager: OnceCell<DeviceManager>,
    pub trainer: RwLock<Option<Arc<dyn Trainer>>>,
    pub hrm: RwLock<Option<Arc<dyn HeartRateMonitor>>>,
    /// Control block of the in-flight scan, if any. Connecting while a scan
    /// polls the same peripheral races btleplug's CoreBluetooth futures
    /// (observed panic: "We should still have a future at this point!"), so
    /// connects cancel the scan and wait for it to wind down first.
    scan_ctl: std::sync::Mutex<Option<Arc<ScanCtl>>>,
    /// Per-role connect-in-progress guard: a double-click used to launch two
    /// concurrent connects to the same peripheral, racing btleplug's
    /// CoreBluetooth discovery futures (observed panic, internal.rs:282).
    busy: [std::sync::atomic::AtomicBool; 2],
}

struct ScanCtl {
    stop: Arc<std::sync::atomic::AtomicBool>,
    running: std::sync::atomic::AtomicBool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceStatusPayload {
    pub role: Role,
    pub status: String,
    pub attempt: Option<u32>,
    pub name: Option<String>,
}

fn status_str(s: DeviceStatus) -> (String, Option<u32>) {
    match s {
        DeviceStatus::Disconnected => ("disconnected".into(), None),
        DeviceStatus::Connecting => ("connecting".into(), None),
        DeviceStatus::Connected => ("connected".into(), None),
        DeviceStatus::Reconnecting { attempt } => ("reconnecting".into(), Some(attempt)),
    }
}

pub fn emit_device_status(
    app: &AppHandle,
    role: Role,
    status: DeviceStatus,
    name: Option<String>,
) {
    let (status, attempt) = status_str(status);
    let _ = app.emit("device_status", DeviceStatusPayload { role, status, attempt, name });
}

impl DeviceHub {
    async fn manager(&self) -> Result<&DeviceManager, BleError> {
        self.manager.get_or_try_init(DeviceManager::new).await
    }

    /// Time-boxed scan; sim entries first, then real devices as found.
    /// Emits `scan_result` events; returns when the scan window closes.
    pub async fn scan(&self, app: AppHandle, role: Role) -> Result<(), BleError> {
        let sim = match role {
            Role::Trainer => ScanResult {
                platform_id: SIM_TRAINER_ID.into(),
                name: "Simulated KICKR".into(),
                rssi: None,
                role,
            },
            Role::Hrm => ScanResult {
                platform_id: SIM_HRM_ID.into(),
                name: "Simulated HRM".into(),
                rssi: None,
                role,
            },
        };
        let _ = app.emit("scan_result", &sim);
        let manager = self.manager().await?;

        // Register this scan's control block (cancelling any previous scan).
        let ctl = Arc::new(ScanCtl {
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            running: std::sync::atomic::AtomicBool::new(true),
        });
        {
            let mut slot = self.scan_ctl.lock().unwrap();
            if let Some(prev) = slot.take() {
                prev.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            *slot = Some(ctl.clone());
        }

        let app2 = app.clone();
        let result = manager
            .scan(role, 10, ctl.stop.clone(), move |r| {
                let _ = app2.emit("scan_result", &r);
            })
            .await;
        ctl.running.store(false, std::sync::atomic::Ordering::Relaxed);
        result
    }

    /// Cancel any in-flight scan and wait (bounded) for its loop to exit, so
    /// no scan-poll interleaves with the upcoming connect.
    async fn quiesce_scan(&self) {
        let ctl = self.scan_ctl.lock().unwrap().take();
        if let Some(ctl) = ctl {
            ctl.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            for _ in 0..40 {
                if !ctl.running.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }

    pub async fn connect(
        &self,
        app: &AppHandle,
        role: Role,
        platform_id: &str,
    ) -> Result<String, BleError> {
        use std::sync::atomic::Ordering;
        let idx = match role {
            Role::Trainer => 0,
            Role::Hrm => 1,
        };
        if self.busy[idx].swap(true, Ordering::SeqCst) {
            return Err(BleError::Transport(
                "a connection attempt is already in progress".into(),
            ));
        }
        let result = self.connect_inner(app, role, platform_id).await;
        self.busy[idx].store(false, Ordering::SeqCst);
        if result.is_err() {
            emit_device_status(app, role, DeviceStatus::Disconnected, None);
        }
        result
    }

    async fn connect_inner(
        &self,
        app: &AppHandle,
        role: Role,
        platform_id: &str,
    ) -> Result<String, BleError> {
        emit_device_status(app, role, DeviceStatus::Connecting, None);
        self.quiesce_scan().await;
        let name = match role {
            Role::Trainer => {
                let trainer: Arc<dyn Trainer> = if platform_id == SIM_TRAINER_ID {
                    Arc::new(SimTrainer::new())
                } else {
                    Arc::new(self.manager().await?.connect_trainer(platform_id).await?)
                };
                let name = trainer.name();
                spawn_status_forwarder(app.clone(), role, trainer.status(), name.clone());
                spawn_trainer_reading_forwarder(app.clone(), trainer.telemetry());
                if platform_id != SIM_TRAINER_ID {
                    spawn_reconnector(app.clone(), platform_id.to_string(), trainer.status());
                }
                *self.trainer.write().await = Some(trainer);
                name
            }
            Role::Hrm => {
                let hrm: Arc<dyn HeartRateMonitor> = if platform_id == SIM_HRM_ID {
                    // Model effort off the connected trainer when present.
                    let rx = match self.trainer.read().await.as_ref() {
                        Some(t) => t.telemetry(),
                        None => tokio::sync::broadcast::channel(1).0.subscribe(),
                    };
                    Arc::new(SimHrm::new(rx))
                } else {
                    Arc::new(self.manager().await?.connect_hrm(platform_id).await?)
                };
                let name = hrm.name();
                spawn_status_forwarder(app.clone(), role, hrm.status(), name.clone());
                spawn_hrm_reading_forwarder(app.clone(), hrm.heart_rate());
                *self.hrm.write().await = Some(hrm);
                name
            }
        };
        emit_device_status(app, role, DeviceStatus::Connected, Some(name.clone()));
        Ok(name)
    }

    pub async fn disconnect(&self, app: &AppHandle, role: Role) {
        match role {
            Role::Trainer => *self.trainer.write().await = None,
            Role::Hrm => *self.hrm.write().await = None,
        }
        emit_device_status(app, role, DeviceStatus::Disconnected, None);
    }
}

/// Live reading ticker for the Devices screen: latest trainer power/cadence
/// at 1 Hz while connected; a final null reading on disconnect. Exits when
/// the driver (broadcast sender) is dropped.
fn spawn_trainer_reading_forwarder(
    app: AppHandle,
    mut rx: tokio::sync::broadcast::Receiver<tp_ble::TrainerData>,
) {
    use tokio::sync::broadcast::error::RecvError;
    tokio::spawn(async move {
        let mut latest: Option<tp_ble::TrainerData> = None;
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                r = rx.recv() => match r {
                    Ok(d) => latest = Some(d),
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                },
                _ = tick.tick() => {
                    if let Some(d) = &latest {
                        let _ = app.emit("device_reading", serde_json::json!({
                            "role": "trainer",
                            "power": d.power_w,
                            "cadence": d.cadence_rpm.map(|c| c.round() as u16),
                        }));
                    }
                }
            }
        }
        let _ = app.emit("device_reading", serde_json::json!({
            "role": "trainer", "power": null, "cadence": null,
        }));
    });
}

fn spawn_hrm_reading_forwarder(
    app: AppHandle,
    mut rx: tokio::sync::broadcast::Receiver<tp_ble::HrData>,
) {
    use tokio::sync::broadcast::error::RecvError;
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(d) => {
                    let _ = app.emit("device_reading", serde_json::json!({
                        "role": "hrm", "hr": d.bpm,
                    }));
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
        let _ = app.emit("device_reading", serde_json::json!({ "role": "hrm", "hr": null }));
    });
}

/// SPEC §4.4: on app start, silently reconnect saved devices. Trainer
/// first (the sim HRM models effort off trainer telemetry), two attempts
/// each; connect_trainer/_hrm already scan-until-found internally. Gives up
/// quietly — the Devices screen still offers manual Connect.
pub fn spawn_startup_reconnect(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Let the BLE adapter and webview settle before scanning.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let state = app.state::<AppState>();
        let mut saved: Vec<(String, String)> = {
            let conn = state.db.lock().unwrap();
            let Ok(mut stmt) = conn.prepare("SELECT role, platform_id FROM devices") else {
                return;
            };
            match stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
            {
                Ok(v) => v,
                Err(_) => return,
            }
        };
        saved.sort_by_key(|(role, _)| if role == "trainer" { 0 } else { 1 });
        for (role_s, pid) in saved {
            let role = match role_s.as_str() {
                "trainer" => Role::Trainer,
                "hrm" => Role::Hrm,
                _ => continue,
            };
            let already = match role {
                Role::Trainer => state.hub.trainer.read().await.is_some(),
                Role::Hrm => state.hub.hrm.read().await.is_some(),
            };
            if already {
                continue;
            }
            for attempt in 1..=2u32 {
                match state.hub.connect(&app, role, &pid).await {
                    Ok(name) => {
                        info!("startup reconnect: {role_s} \"{name}\" connected");
                        break;
                    }
                    Err(e) => {
                        warn!("startup reconnect {role_s} attempt {attempt} failed: {e}");
                        if attempt < 2 {
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        }
                    }
                }
            }
        }
    });
}

/// Forward driver status changes to the UI event stream.
fn spawn_status_forwarder(
    app: AppHandle,
    role: Role,
    mut status: tokio::sync::watch::Receiver<DeviceStatus>,
    name: String,
) {
    tokio::spawn(async move {
        while status.changed().await.is_ok() {
            let s = *status.borrow();
            emit_device_status(&app, role, s, Some(name.clone()));
            if s == DeviceStatus::Disconnected {
                break;
            }
        }
    });
}

/// Trainer auto-reconnect per SPEC §4.4: on drop, retry on the schedule
/// (0/1/2/5/10 s then every 15 s) until the slot is replaced, the user
/// disconnects, or a new device connects. The player runtime auto-pauses
/// independently by watching the same status channel.
fn spawn_reconnector(
    app: AppHandle,
    platform_id: String,
    mut status: tokio::sync::watch::Receiver<DeviceStatus>,
) {
    tokio::spawn(async move {
        // Wait for the drop.
        loop {
            if *status.borrow() == DeviceStatus::Disconnected {
                break;
            }
            if status.changed().await.is_err() {
                break;
            }
        }
        let state = app.state::<AppState>();
        let mut attempt: u32 = 0;
        loop {
            // Stop if the user cleared or replaced the slot meanwhile.
            {
                let slot = state.hub.trainer.read().await;
                match slot.as_ref() {
                    None => return,
                    Some(t) if *t.status().borrow() == DeviceStatus::Connected => return,
                    Some(_) => {}
                }
            }
            let delay = RECONNECT_SCHEDULE_S
                .get(attempt as usize)
                .copied()
                .unwrap_or(RECONNECT_STEADY_S);
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            attempt += 1;
            emit_device_status(
                &app,
                Role::Trainer,
                DeviceStatus::Reconnecting { attempt },
                None,
            );
            match state.hub.connect(&app, Role::Trainer, &platform_id).await {
                Ok(name) => {
                    info!("trainer reconnected: {name}");
                    return; // connect() spawned a fresh reconnector.
                }
                Err(e) => warn!("reconnect attempt {attempt} failed: {e}"),
            }
        }
    });
}
