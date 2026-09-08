//! FTMS trainer driver over btleplug. SPEC.md §4.2.

use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::{
    Central as _, CentralEvent, CharPropFlags, Characteristic, Peripheral as _, WriteType,
};
use btleplug::platform::{Adapter, Peripheral};
use futures::StreamExt;
use tokio::sync::{broadcast, mpsc, watch, Mutex};
use tokio::time::{timeout, Duration};
use tracing::{debug, warn};

use tp_core::consts::{CP_RETRIES, CP_TIMEOUT_MS};

use crate::codec::{self, ControlPointResponse};
use crate::tasks::ConnectionTasks;
use crate::traits::{BleError, ConnectionStatus, TrainerConnection, TrainerMeasurement};

/// Consecutive malformed Indoor Bike Data packets treated as a link failure.
const MAX_MALFORMED: u32 = 10;
const MEASUREMENT_CHANNEL_CAPACITY: usize = 32;
const CONTROL_RESPONSE_CHANNEL_CAPACITY: usize = 8;

/// FTMS control and measurements for one connection. The instance can outlive its
/// link; connectivity comes from `subscribe_status()`. Reconnect creates a new instance.
pub struct FtmsTrainerConnection {
    peripheral: Peripheral,
    cp: Characteristic,
    name: String,
    tasks: ConnectionTasks,
    measurements_tx: broadcast::Sender<TrainerMeasurement>,
    status_tx: watch::Sender<ConnectionStatus>,
    /// One control-point op in flight at a time (SPEC §4.2). The mutex guards
    /// the response receiver; holding it across write+await serializes ops.
    cp_resp: Mutex<mpsc::Receiver<ControlPointResponse>>,
}

impl FtmsTrainerConnection {
    /// Full connect sequence per SPEC §4.2: discover, capability-check,
    /// subscribe, request control. Fails with a specific error at each step.
    pub async fn connect(adapter: Adapter, peripheral: Peripheral) -> Result<Self, BleError> {
        let result = Self::establish(adapter, peripheral.clone()).await;
        if result.is_err() {
            // Setup can fail after opening the transport.
            let _ = peripheral.disconnect().await;
        }
        result
    }

    async fn establish(adapter: Adapter, peripheral: Peripheral) -> Result<Self, BleError> {
        let mut tasks = ConnectionTasks::default();
        let peripheral_id = peripheral.id();
        // Subscribe before connecting so a short-lived connection cannot
        // lose its disconnect event between setup steps.
        let mut adapter_events = adapter
            .events()
            .await
            .map_err(|e| BleError::Adapter(format!("adapter events: {e}")))?;
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

        let (measurements_tx, _) = broadcast::channel(MEASUREMENT_CHANNEL_CAPACITY);
        let (status_tx, _) = watch::channel(ConnectionStatus::Connecting);
        let (cp_tx, cp_rx) = mpsc::channel(CONTROL_RESPONSE_CHANNEL_CAPACITY);

        // Notification pump: measurements out, CP responses to the op waiter,
        // stream end = disconnect.
        let stream = peripheral
            .notifications()
            .await
            .map_err(|e| BleError::Transport(format!("notification stream: {e}")))?;
        {
            let measurements_tx = measurements_tx.clone();
            let status_tx = status_tx.clone();
            let mut status_rx = status_tx.subscribe();
            tasks.push(tokio::spawn(async move {
                let mut malformed = 0u32;
                let mut stream = stream;
                loop {
                    let n = tokio::select! {
                        n = stream.next() => match n {
                            Some(n) => n,
                            None => break,
                        },
                        changed = status_rx.changed() => {
                            if changed.is_err()
                                || *status_rx.borrow() == ConnectionStatus::Disconnected
                            {
                                break;
                            }
                            continue;
                        }
                    };
                    if n.uuid == uuid_from_u16(codec::CHR_INDOOR_BIKE_DATA) {
                        match codec::parse_indoor_bike_data(&n.value) {
                            Ok(d) => {
                                malformed = 0;
                                let _ = measurements_tx.send(d);
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
                status_tx.send_replace(ConnectionStatus::Disconnected);
            }));
        }

        let mut trainer = FtmsTrainerConnection {
            peripheral,
            cp,
            name,
            tasks,
            measurements_tx,
            status_tx,
            cp_resp: Mutex::new(cp_rx),
        };

        debug!("ftms connect: requesting control");
        trainer
            .cp_op(codec::request_control(), codec::CP_REQUEST_CONTROL)
            .await?;
        trainer.status_tx.send_replace(ConnectionStatus::Connected);

        // On CoreBluetooth, a peripheral notification stream can remain open
        // after link loss. The adapter event is the authoritative disconnect
        // signal; the notification pump remains a secondary fallback.
        {
            let status_tx = trainer.status_tx.clone();
            trainer.tasks.push(tokio::spawn(async move {
                while let Some(event) = adapter_events.next().await {
                    if matches!(
                        event,
                        CentralEvent::DeviceDisconnected(ref id) if id == &peripheral_id
                    ) {
                        status_tx.send_replace(ConnectionStatus::Disconnected);
                        return;
                    }
                }
                status_tx.send_replace(ConnectionStatus::Disconnected);
            }));
        }
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
                Ok(Some(response)) => {
                    if response.request_opcode != op {
                        warn!(
                            "CP response op mismatch: sent {op:#04x} got {:#04x}",
                            response.request_opcode
                        );
                        continue;
                    }
                    return if response.result_code == codec::CP_RESULT_SUCCESS {
                        Ok(())
                    } else {
                        Err(BleError::ControlRefused(response.result_code))
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
impl TrainerConnection for FtmsTrainerConnection {
    async fn disconnect(&mut self) -> Result<(), BleError> {
        self.status_tx.send_replace(ConnectionStatus::Disconnected);
        let result = self
            .peripheral
            .disconnect()
            .await
            .map_err(|e| BleError::Transport(format!("disconnect: {e}")));
        self.tasks.shutdown().await;
        result
    }
    async fn probe_connection(&self) -> Result<bool, BleError> {
        self.peripheral
            .is_connected()
            .await
            .map_err(|e| BleError::Transport(format!("connection check: {e}")))
    }
    async fn set_target_power(&self, watts: u16) -> Result<(), BleError> {
        self.cp_op(codec::set_target_power(watts), codec::CP_SET_TARGET_POWER)
            .await
    }
    async fn set_flat_road_simulation(&self) -> Result<(), BleError> {
        self.cp_op(codec::flat_road_simulation(), codec::CP_SET_INDOOR_BIKE_SIM)
            .await
    }
    async fn start_or_resume_training(&self) -> Result<(), BleError> {
        self.cp_op(codec::start_or_resume_training(), codec::CP_START_RESUME)
            .await
    }
    async fn pause_training(&self) -> Result<(), BleError> {
        self.cp_op(codec::pause_training(), codec::CP_STOP_PAUSE)
            .await
    }
    async fn reset_trainer(&self) -> Result<(), BleError> {
        self.cp_op(codec::reset_trainer(), codec::CP_RESET).await
    }
    fn subscribe_measurements(&self) -> broadcast::Receiver<TrainerMeasurement> {
        self.measurements_tx.subscribe()
    }
    fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus> {
        self.status_tx.subscribe()
    }
    fn name(&self) -> &str {
        &self.name
    }
}
