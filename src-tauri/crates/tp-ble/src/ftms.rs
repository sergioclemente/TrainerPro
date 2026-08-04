//! FTMS trainer driver over btleplug. SPEC.md §4.2.

use std::sync::Arc;

use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::{CharPropFlags, Characteristic, Peripheral as _, WriteType};
use btleplug::platform::Peripheral;
use futures::StreamExt;
use tokio::sync::{broadcast, mpsc, watch, Mutex};
use tokio::time::{timeout, Duration};
use tracing::{debug, warn};

use tp_core::consts::{CP_RETRIES, CP_TIMEOUT_MS};

use crate::codec;
use crate::traits::{BleError, DeviceStatus, Trainer, TrainerData};

/// Consecutive malformed Indoor Bike Data packets treated as a link failure.
const MAX_MALFORMED: u32 = 10;

pub struct FtmsTrainer {
    peripheral: Peripheral,
    cp: Characteristic,
    name: String,
    telemetry_tx: broadcast::Sender<TrainerData>,
    status_tx: Arc<watch::Sender<DeviceStatus>>,
    /// One control-point op in flight at a time (SPEC §4.2). The mutex guards
    /// the response receiver; holding it across write+await serializes ops.
    cp_resp: Mutex<mpsc::Receiver<(u8, u8)>>,
}

impl FtmsTrainer {
    /// Full connect sequence per SPEC §4.2: discover, capability-check,
    /// subscribe, request control. Fails with a specific error at each step.
    pub async fn connect(peripheral: Peripheral) -> Result<Self, BleError> {
        debug!("ftms connect: gatt connect to {:?}", peripheral.id());
        peripheral
            .connect()
            .await
            .map_err(|e| BleError::Transport(format!("gatt connect: {e}")))?;
        peripheral
            .discover_services()
            .await
            .map_err(|e| BleError::Transport(format!("service discovery: {e}")))?;

        debug!("ftms connect: services discovered");
        let chars = peripheral.characteristics();
        let find = |u16id: u16| -> Option<Characteristic> {
            chars.iter().find(|c| c.uuid == uuid_from_u16(u16id)).cloned()
        };
        let ibd = find(codec::CHR_INDOOR_BIKE_DATA)
            .ok_or_else(|| BleError::Incompatible("no Indoor Bike Data (FTMS)".into()))?;
        let cp = find(codec::CHR_FTMS_CONTROL_POINT)
            .ok_or_else(|| BleError::Incompatible("no FTMS Control Point".into()))?;

        // Capability check: Fitness Machine Feature, target-power bit in the
        // Target Setting Features uint32 (bytes 4..8).
        if let Some(feat) = find(codec::CHR_FITNESS_MACHINE_FEATURE) {
            match peripheral.read(&feat).await {
                Ok(v) if v.len() >= 8 => {
                    let target = u32::from_le_bytes([v[4], v[5], v[6], v[7]]);
                    if target & codec::TARGET_SETTING_POWER_BIT == 0 {
                        return Err(BleError::Incompatible(
                            "trainer does not support power targets".into(),
                        ));
                    }
                }
                Ok(_) | Err(_) => debug!("feature read unavailable; proceeding"),
            }
        }

        let name = peripheral
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|p| p.local_name)
            .unwrap_or_else(|| "FTMS trainer".into());

        // Subscriptions before Request Control (CP responses arrive as
        // indications on the same characteristic).
        for c in [&cp, &ibd] {
            debug!("subscribing to {}", c.uuid);
            peripheral
                .subscribe(c)
                .await
                .map_err(|e| BleError::Transport(format!("subscribe {}: {e}", c.uuid)))?;
        }
        if let Some(st) = find(codec::CHR_FTMS_STATUS) {
            if st.properties.contains(CharPropFlags::NOTIFY) {
                let _ = peripheral.subscribe(&st).await;
            }
        }

        let (telemetry_tx, _) = broadcast::channel(32);
        let (status_tx, _) = watch::channel(DeviceStatus::Connecting);
        let status_tx = Arc::new(status_tx);
        let (cp_tx, cp_rx) = mpsc::channel(8);

        // Notification pump: telemetry out, CP responses to the op waiter,
        // stream end = disconnect.
        let stream = peripheral
            .notifications()
            .await
            .map_err(|e| BleError::Transport(format!("notification stream: {e}")))?;
        {
            let telemetry_tx = telemetry_tx.clone();
            let status_tx = status_tx.clone();
            tokio::spawn(async move {
                let mut malformed = 0u32;
                let mut stream = stream;
                while let Some(n) = stream.next().await {
                    if n.uuid == uuid_from_u16(codec::CHR_INDOOR_BIKE_DATA) {
                        match codec::parse_indoor_bike_data(&n.value) {
                            Ok(d) => {
                                malformed = 0;
                                let _ = telemetry_tx.send(d);
                            }
                            Err(e) => {
                                malformed += 1;
                                warn!("malformed IBD packet ({e}), {malformed} consecutive");
                                if malformed >= MAX_MALFORMED {
                                    break;
                                }
                            }
                        }
                    } else if n.uuid == uuid_from_u16(codec::CHR_FTMS_CONTROL_POINT) {
                        if let Ok(resp) = codec::parse_cp_response(&n.value) {
                            let _ = cp_tx.send(resp).await;
                        }
                    }
                    // FTMS Status (0x2ADA) is informational; ignored in v1.
                }
                status_tx.send_replace(DeviceStatus::Disconnected);
            });
        }

        let trainer = FtmsTrainer {
            peripheral,
            cp,
            name,
            telemetry_tx,
            status_tx,
            cp_resp: Mutex::new(cp_rx),
        };

        debug!("ftms connect: requesting control");
        trainer.cp_op(codec::request_control(), codec::CP_REQUEST_CONTROL).await?;
        trainer.status_tx.send_replace(DeviceStatus::Connected);
        Ok(trainer)
    }

    /// Write a CP op and await its response indication; retry once on
    /// timeout (SPEC §4.2 / consts CP_TIMEOUT_MS, CP_RETRIES).
    async fn cp_op(&self, frame: Vec<u8>, op: u8) -> Result<(), BleError> {
        let mut rx = self.cp_resp.lock().await;
        for attempt in 0..=CP_RETRIES {
            // Drain stale responses from any earlier timed-out op.
            while rx.try_recv().is_ok() {}
            self.peripheral
                .write(&self.cp, &frame, WriteType::WithResponse)
                .await
                .map_err(|e| BleError::Transport(format!("cp write {:#04x}: {e}", op)))?;
            match timeout(Duration::from_millis(CP_TIMEOUT_MS), rx.recv()).await {
                Ok(Some((resp_op, result))) => {
                    if resp_op != op {
                        warn!("CP response op mismatch: sent {op:#04x} got {resp_op:#04x}");
                        continue;
                    }
                    return if result == codec::CP_RESULT_SUCCESS {
                        Ok(())
                    } else {
                        Err(BleError::ControlRefused(result))
                    };
                }
                Ok(None) => return Err(BleError::Disconnected),
                Err(_) if attempt < CP_RETRIES => continue,
                Err(_) => return Err(BleError::Timeout),
            }
        }
        Err(BleError::Timeout)
    }
}

#[async_trait::async_trait]
impl Trainer for FtmsTrainer {
    async fn set_target_power(&self, watts: u16) -> Result<(), BleError> {
        self.cp_op(codec::set_target_power(watts), codec::CP_SET_TARGET_POWER).await
    }
    async fn set_sim_grade_zero(&self) -> Result<(), BleError> {
        self.cp_op(codec::sim_grade_zero(), codec::CP_SET_INDOOR_BIKE_SIM).await
    }
    async fn start(&self) -> Result<(), BleError> {
        self.cp_op(codec::start_resume(), codec::CP_START_RESUME).await
    }
    async fn stop(&self) -> Result<(), BleError> {
        self.cp_op(codec::pause(), codec::CP_STOP_PAUSE).await
    }
    async fn reset(&self) -> Result<(), BleError> {
        self.cp_op(codec::reset(), codec::CP_RESET).await
    }
    fn telemetry(&self) -> broadcast::Receiver<TrainerData> {
        self.telemetry_tx.subscribe()
    }
    fn status(&self) -> watch::Receiver<DeviceStatus> {
        self.status_tx.subscribe()
    }
    fn name(&self) -> String {
        self.name.clone()
    }
}
