//! BLE heart-rate strap driver. SPEC.md §4.3.

use std::sync::Arc;

use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::Peripheral as _;
use btleplug::platform::Peripheral;
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
    pub async fn connect(peripheral: Peripheral) -> Result<Self, BleError> {
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
            tokio::spawn(async move {
                let mut stream = stream;
                while let Some(n) = stream.next().await {
                    if n.uuid == uuid_from_u16(codec::CHR_HEART_RATE_MEASUREMENT) {
                        if let Ok(Some(bpm)) = codec::parse_heart_rate(&n.value) {
                            let _ = hr_tx.send(HrData { bpm });
                        }
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
