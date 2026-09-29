//! Direct BLE input for bonded Zwift Ride controllers. No trainer control.
use crate::connection_tasks::ConnectionTasks;
use crate::controller::{ButtonEdges, INPUT_CAPACITY};
use crate::zwift_ride_protocol as codec;
use crate::{
    BleError, ConnectionStatus, ControllerInputEvent, ControllerInputStream, ControllerProfile,
    StandaloneControllerConnection,
};
use btleplug::api::{
    bleuuid::uuid_from_u16, Central as _, CentralEvent, Peripheral as _, WriteType,
};
use btleplug::platform::{Adapter, Peripheral};
use futures::StreamExt;
use tokio::sync::{broadcast, watch};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

const FIRMWARE_REVISION: u16 = 0x2a26;
const SUPPORTED_FIRMWARE_MAX: [u32; 3] = [1, 2, 0];
const INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ZwiftRideConnection {
    peripheral: Peripheral,
    name: String,
    tasks: ConnectionTasks,
    events: broadcast::Sender<ControllerInputEvent>,
    status: watch::Sender<ConnectionStatus>,
}

fn transport(error: impl std::fmt::Display) -> BleError {
    BleError::Transport(error.to_string())
}

fn firmware_supported(value: &[u8]) -> bool {
    let text = String::from_utf8_lossy(value);
    let parts: Vec<_> = text
        .trim()
        .split('.')
        .take(SUPPORTED_FIRMWARE_MAX.len())
        .map(str::parse::<u32>)
        .collect();
    match parts.as_slice() {
        [Ok(a), Ok(b), Ok(c)] => [*a, *b, *c] <= SUPPORTED_FIRMWARE_MAX,
        _ => true, // Unknown firmware still has to pass protocol initialization.
    }
}

impl ZwiftRideConnection {
    pub async fn connect(adapter: Adapter, peripheral: Peripheral) -> Result<Self, BleError> {
        if peripheral
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|properties| {
                properties
                    .manufacturer_data
                    .get(&codec::ZWIFT_COMPANY_ID)
                    .and_then(|data| data.first().copied())
            })
            .is_some_and(|kind| !codec::is_left_device_type(kind))
        {
            return Err(BleError::Incompatible(
                "Select the left Zwift Ride controller; it carries both handles".into(),
            ));
        }
        let result = Self::establish(adapter, peripheral.clone()).await;
        if result.is_err() {
            let _ = peripheral.disconnect().await;
        }
        result
    }
    async fn establish(adapter: Adapter, peripheral: Peripheral) -> Result<Self, BleError> {
        let mut adapter_events = adapter.events().await.map_err(transport)?;
        let peripheral_id = peripheral.id();
        peripheral.connect().await.map_err(transport)?;
        peripheral.discover_services().await.map_err(transport)?;
        let chars = peripheral.characteristics();
        if let Some(firmware) = chars
            .iter()
            .find(|c| c.uuid == uuid_from_u16(FIRMWARE_REVISION))
        {
            if let Ok(value) = peripheral.read(firmware).await {
                if !firmware_supported(&value) {
                    return Err(BleError::Incompatible(format!("Zwift Ride firmware {} is not supported; supported protocol target is 1.2.0", String::from_utf8_lossy(&value))));
                }
            }
        }
        let service = [
            uuid_from_u16(codec::CURRENT_SERVICE_UUID),
            Uuid::from_u128(codec::LEGACY_SERVICE_UUID),
        ]
        .into_iter()
        .find(|service| {
            [
                codec::NOTIFY_CHARACTERISTIC_UUID,
                codec::INDICATE_CHARACTERISTIC_UUID,
                codec::WRITE_CHARACTERISTIC_UUID,
            ]
            .iter()
            .all(|id| {
                chars
                    .iter()
                    .any(|c| c.uuid == Uuid::from_u128(*id) && c.service_uuid == *service)
            })
        })
        .ok_or_else(|| BleError::Incompatible("Zwift Ride input service unavailable".into()))?;
        let find = |id| {
            chars
                .iter()
                .find(|c| c.uuid == Uuid::from_u128(id) && c.service_uuid == service)
                .cloned()
                .ok_or_else(|| {
                    BleError::Incompatible("Zwift Ride input service unavailable".into())
                })
        };
        let notify = find(codec::NOTIFY_CHARACTERISTIC_UUID)?;
        let indicate = find(codec::INDICATE_CHARACTERISTIC_UUID)?;
        let write = find(codec::WRITE_CHARACTERISTIC_UUID)?;
        let mut stream = peripheral.notifications().await.map_err(transport)?;
        peripheral.subscribe(&notify).await.map_err(transport)?;
        peripheral.subscribe(&indicate).await.map_err(transport)?;
        peripheral
            .write(&write, codec::HANDSHAKE_REQUEST, WriteType::WithoutResponse)
            .await
            .map_err(transport)?;
        let initial_bitmap = timeout(INITIALIZATION_TIMEOUT, async {
            loop {
                tokio::select! {
                    event = adapter_events.next() => match event {
                        Some(CentralEvent::DeviceDisconnected(id)) if id == peripheral_id => return Err(BleError::Disconnected),
                        None => return Err(BleError::Disconnected),
                        _ => {},
                    },
                    notification = stream.next() => {
                        let n = notification.ok_or(BleError::Disconnected)?;
                        if (n.uuid == notify.uuid || n.uuid == indicate.uuid) && n.value.starts_with(codec::HANDSHAKE_REQUEST) { return Ok(None); }
                        if n.uuid == notify.uuid && n.value.first() == Some(&codec::BUTTON_STATUS_OPCODE) {
                            if let Ok(Some(bitmap)) = codec::parse_button_bitmap(&n.value[1..]) { return Ok(Some(bitmap)); }
                        }
                    }
                }
            }
        }).await.map_err(|_| BleError::Incompatible("Zwift Ride did not initialize. Wake both handles and close other controller apps; this firmware may be unsupported.".into()))??;

        let name = peripheral
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|p| p.local_name)
            .unwrap_or_else(|| "Zwift Ride".into());
        let (events, _) = broadcast::channel(INPUT_CAPACITY);
        let (status, _) = watch::channel(ConnectionStatus::Connected);
        let mut tasks = ConnectionTasks::default();
        let events_tx = events.clone();
        let status_tx = status.clone();
        tasks.push(tokio::spawn(async move {
            let mut edges = ButtonEdges::default();
            if let Some(bitmap) = initial_bitmap {
                // A key held while connecting is not a fresh press. Seed the
                // state so it must be released before it can trigger an action.
                for (mask, button) in codec::BUTTON_MASKS { edges.update(button, bitmap & mask == 0); }
            }
            loop {
                tokio::select! {
                    event = adapter_events.next() => match event {
                        Some(CentralEvent::DeviceDisconnected(id)) if id == peripheral_id => break,
                        None => break,
                        _ => {},
                    },
                    notification = stream.next() => {
                        let Some(n) = notification else { break; };
                        if n.uuid != notify.uuid || n.value.first() != Some(&codec::BUTTON_STATUS_OPCODE) { continue; }
                        match codec::parse_button_bitmap(&n.value[1..]) {
                            Ok(Some(bitmap)) => {
                                for (mask, button) in codec::BUTTON_MASKS {
                                    if let Some(event) = edges.update(button, bitmap & mask == 0) {
                                        let _ = events_tx.send(event);
                                    }
                                }
                            },
                            Ok(None) => {},
                            Err(error) => { tracing::warn!("malformed Ride input: {error}"); let _ = events_tx.send(ControllerInputEvent::Cancel); }
                        }
                    }
                }
            }
            status_tx.send_replace(ConnectionStatus::Disconnected);
            let _ = events_tx.send(ControllerInputEvent::Cancel);
        }));
        Ok(Self {
            peripheral,
            name,
            tasks,
            events,
            status,
        })
    }
}

#[async_trait::async_trait]
impl StandaloneControllerConnection for ZwiftRideConnection {
    async fn disconnect(&mut self) -> Result<(), BleError> {
        self.status.send_replace(ConnectionStatus::Disconnected);
        let _ = self.events.send(ControllerInputEvent::Cancel);
        let result = self.peripheral.disconnect().await.map_err(transport);
        self.tasks.shutdown().await;
        result
    }
    fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus> {
        self.status.subscribe()
    }
    fn controller_input(&self) -> ControllerInputStream {
        ControllerInputStream {
            profile: ControllerProfile::ZwiftRide,
            events: self.events.subscribe(),
        }
    }
    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn firmware_gate() {
        assert!(firmware_supported(b"1.2.0.24"));
        assert!(!firmware_supported(b"1.3.0"));
        assert!(firmware_supported(b"unknown"));
    }
}
