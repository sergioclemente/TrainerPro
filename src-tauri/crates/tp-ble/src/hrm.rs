//! BLE heart-rate strap driver. SPEC.md §4.3.

use std::sync::Arc;

use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::{Central as _, CentralEvent, Peripheral as _};
use btleplug::platform::{Adapter, Peripheral};
use futures::StreamExt;
use tokio::sync::{broadcast, watch};

use crate::codec;
use crate::traits::{BleError, DeviceStatus, HeartRateMonitor, HrData};

pub struct HrmDevice {
    name: String,
    hr_tx: broadcast::Sender<HrData>,
    status_tx: Arc<watch::Sender<DeviceStatus>>,
    // Held to keep the connection alive for the device's lifetime.
    _peripheral: Peripheral,
}

impl HrmDevice {
    pub async fn connect(adapter: Adapter, peripheral: Peripheral) -> Result<Self, BleError> {
        let peripheral_id = peripheral.id();
        let mut adapter_events = adapter
            .events()
            .await
            .map_err(|e| BleError::Adapter(format!("adapter events: {e}")))?;
        peripheral
            .connect()
            .await
            .map_err(|e| BleError::Transport(e.to_string()))?;
        peripheral
            .discover_services()
            .await
            .map_err(|e| BleError::Transport(e.to_string()))?;

        let hrm_char = peripheral
            .characteristics()
            .iter()
            .find(|c| c.uuid == uuid_from_u16(codec::CHR_HEART_RATE_MEASUREMENT))
            .cloned()
            .ok_or_else(|| BleError::Incompatible("no Heart Rate Measurement".into()))?;
        peripheral
            .subscribe(&hrm_char)
            .await
            .map_err(|e| BleError::Transport(e.to_string()))?;

        let name = peripheral
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|p| p.local_name)
            .unwrap_or_else(|| "HR monitor".into());

        let (hr_tx, _) = broadcast::channel(32);
        let (status_tx, _) = watch::channel(DeviceStatus::Connected);
        let status_tx = Arc::new(status_tx);

        let stream = peripheral
            .notifications()
            .await
            .map_err(|e| BleError::Transport(e.to_string()))?;
        {
            let hr_tx = hr_tx.clone();
            let status_tx = status_tx.clone();
            let mut status_rx = status_tx.subscribe();
            tokio::spawn(async move {
                let mut stream = stream;
                loop {
                    let n = tokio::select! {
                        n = stream.next() => match n {
                            Some(n) => n,
                            None => break,
                        },
                        changed = status_rx.changed() => {
                            if changed.is_err()
                                || *status_rx.borrow() == DeviceStatus::Disconnected
                            {
                                break;
                            }
                            continue;
                        }
                    };
                    if n.uuid == uuid_from_u16(codec::CHR_HEART_RATE_MEASUREMENT) {
                        if let Ok(Some(bpm)) = codec::parse_heart_rate(&n.value) {
                            let _ = hr_tx.send(HrData { bpm });
                        }
                    }
                }
                status_tx.send_replace(DeviceStatus::Disconnected);
            });
        }

        {
            let status_tx = status_tx.clone();
            tokio::spawn(async move {
                while let Some(event) = adapter_events.next().await {
                    if matches!(
                        event,
                        CentralEvent::DeviceDisconnected(ref id) if id == &peripheral_id
                    ) {
                        status_tx.send_replace(DeviceStatus::Disconnected);
                        return;
                    }
                }
                status_tx.send_replace(DeviceStatus::Disconnected);
            });
        }

        Ok(HrmDevice { name, hr_tx, status_tx, _peripheral: peripheral })
    }
}

impl HeartRateMonitor for HrmDevice {
    fn heart_rate(&self) -> broadcast::Receiver<HrData> {
        self.hr_tx.subscribe()
    }
    fn status(&self) -> watch::Receiver<DeviceStatus> {
        self.status_tx.subscribe()
    }
    fn name(&self) -> String {
        self.name.clone()
    }
}
