//! BLE heart-rate strap driver. SPEC.md §4.3.

use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::{Central as _, CentralEvent, Peripheral as _};
use btleplug::platform::{Adapter, Peripheral};
use futures::StreamExt;
use tokio::sync::{broadcast, watch};

use crate::codec;
use crate::connection_tasks::ConnectionTasks;
use crate::traits::{BleError, ConnectionStatus, HeartRateConnection, HeartRateMeasurement};

const MEASUREMENT_CHANNEL_CAPACITY: usize = 32;

/// Heart-rate measurements for one BLE connection. The instance can outlive its
/// link; connectivity comes from `subscribe_status()`. Reconnect creates a new instance.
pub struct BleHeartRateConnection {
    name: String,
    tasks: ConnectionTasks,
    hr_tx: broadcast::Sender<HeartRateMeasurement>,
    status_tx: watch::Sender<ConnectionStatus>,
    // Retained peripheral handle for this connection.
    peripheral: Peripheral,
}

impl BleHeartRateConnection {
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

        let (hr_tx, _) = broadcast::channel(MEASUREMENT_CHANNEL_CAPACITY);
        let (status_tx, _) = watch::channel(ConnectionStatus::Connected);

        let stream = peripheral
            .notifications()
            .await
            .map_err(|e| BleError::Transport(e.to_string()))?;
        {
            let hr_tx = hr_tx.clone();
            let status_tx = status_tx.clone();
            let mut status_rx = status_tx.subscribe();
            tasks.push(tokio::spawn(async move {
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
                    if n.uuid == uuid_from_u16(codec::CHR_HEART_RATE_MEASUREMENT) {
                        if let Ok(Some(bpm)) = codec::parse_heart_rate(&n.value) {
                            let _ = hr_tx.send(HeartRateMeasurement { bpm });
                        }
                    }
                }
                status_tx.send_replace(ConnectionStatus::Disconnected);
            }));
        }

        {
            let status_tx = status_tx.clone();
            tasks.push(tokio::spawn(async move {
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

        Ok(BleHeartRateConnection {
            name,
            tasks,
            hr_tx,
            status_tx,
            peripheral,
        })
    }
}

#[async_trait::async_trait]
impl HeartRateConnection for BleHeartRateConnection {
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
    fn subscribe_measurements(&self) -> broadcast::Receiver<HeartRateMeasurement> {
        self.hr_tx.subscribe()
    }
    fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus> {
        self.status_tx.subscribe()
    }
    fn name(&self) -> &str {
        &self.name
    }
}
