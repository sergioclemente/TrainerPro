//! BLE central coordination, discovery, and connection setup. SPEC.md §4.4.
//! Scan/connect ordering is a transport invariant and is enforced here.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager as BtleManager, Peripheral};
use serde::Serialize;
use tokio::sync::{watch, Mutex, MutexGuard, OnceCell};
use tokio::time::{sleep, Duration};
use tracing::debug;

use crate::ble_heart_rate_connection::BleHeartRateConnection;
use crate::codec;
use crate::ftms_trainer_connection::FtmsTrainerConnection;
use crate::traits::BleError;

const SCAN_POLLS_PER_SECOND: u64 = 2;
const SCAN_POLL_INTERVAL_MS: u64 = 500;
const CONNECT_SCAN_DURATION_S: u64 = 12;
const SCAN_SETTLE_MS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Trainer,
    Hrm,
}

impl Role {
    fn service_u16(self) -> u16 {
        match self {
            Role::Trainer => codec::SVC_FTMS,
            Role::Hrm => codec::SVC_HEART_RATE,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub platform_id: String,
    pub name: String,
    pub rssi: Option<i16>,
    pub role: Role,
}

struct ScanSession {
    stop: Arc<AtomicBool>,
    done: watch::Sender<bool>,
}

impl ScanSession {
    fn new() -> Self {
        let (done, _) = watch::channel(false);
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            done,
        }
    }

    fn cancel(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn complete(&self) {
        self.done.send_replace(true);
    }

    async fn wait_until_complete(&self) {
        let mut done = self.done.subscribe();
        if !*done.borrow() {
            let _ = done.changed().await;
        }
    }
}

/// Admission orders newly requested operations; operation provides exclusive
/// adapter setup access. Established links do not retain either guard.
#[derive(Default)]
struct AdapterOperations {
    admission: Mutex<()>,
    operation: Mutex<()>,
    scan: StdMutex<Option<Arc<ScanSession>>>,
}

impl AdapterOperations {
    async fn cancel_scan_locked(&self) {
        let active = { self.scan.lock().unwrap().clone() };
        if let Some(active) = active {
            active.cancel();
            active.wait_until_complete().await;
        }
    }

    async fn cancel_scan(&self) {
        let _admission = self.admission.lock().await;
        self.cancel_scan_locked().await;
    }

    async fn begin_scan(&self) -> (MutexGuard<'_, ()>, Arc<ScanSession>) {
        let admission = self.admission.lock().await;
        self.cancel_scan_locked().await;
        let operation = self.operation.lock().await;
        let session = Arc::new(ScanSession::new());
        *self.scan.lock().unwrap() = Some(session.clone());
        drop(admission);
        (operation, session)
    }

    async fn begin_connection(&self) -> MutexGuard<'_, ()> {
        let admission = self.admission.lock().await;
        self.cancel_scan_locked().await;
        let operation = self.operation.lock().await;
        drop(admission);
        operation
    }

    fn finish_scan(&self, session: &Arc<ScanSession>) {
        session.complete();
        let mut active = self.scan.lock().unwrap();
        if active
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            *active = None;
        }
    }
}

struct ActiveScan<'a> {
    operations: &'a AdapterOperations,
    session: Arc<ScanSession>,
}

impl Drop for ActiveScan<'_> {
    fn drop(&mut self) {
        self.operations.finish_scan(&self.session);
    }
}

/// Owns the BLE adapter and serializes discovery/connection setup. Adapter
/// initialization remains lazy so the application can start without Bluetooth.
pub struct DeviceManager {
    adapter: OnceCell<Adapter>,
    operations: AdapterOperations,
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceManager {
    pub fn new() -> Self {
        Self {
            adapter: OnceCell::new(),
            operations: AdapterOperations::default(),
        }
    }

    async fn adapter(&self) -> Result<&Adapter, BleError> {
        self.adapter
            .get_or_try_init(|| async {
                let manager = BtleManager::new()
                    .await
                    .map_err(|e| BleError::Adapter(e.to_string()))?;
                manager
                    .adapters()
                    .await
                    .map_err(|e| BleError::Adapter(e.to_string()))?
                    .into_iter()
                    .next()
                    .ok_or_else(|| BleError::Adapter("no bluetooth adapter".into()))
            })
            .await
    }

    /// Cancel the active public discovery scan and wait until its adapter work
    /// has quiesced. This is also useful when selecting a simulated device.
    pub async fn cancel_scan(&self) {
        self.operations.cancel_scan().await;
    }

    /// Time-boxed public discovery. A later connection request cancels this
    /// scan and waits for its adapter work to finish before connecting.
    pub async fn scan(
        &self,
        role: Role,
        secs: u64,
        on_found: impl Fn(ScanResult) + Send,
    ) -> Result<(), BleError> {
        let (_operation, session) = self.operations.begin_scan().await;
        let _active_scan = ActiveScan {
            operations: &self.operations,
            session: session.clone(),
        };
        let adapter = self.adapter().await?;
        self.scan_adapter(adapter, role, secs, session.stop.clone(), on_found)
            .await
    }

    async fn scan_adapter(
        &self,
        adapter: &Adapter,
        role: Role,
        secs: u64,
        stop: Arc<AtomicBool>,
        on_found: impl Fn(ScanResult) + Send,
    ) -> Result<(), BleError> {
        let filter = ScanFilter {
            services: vec![uuid_from_u16(role.service_u16())],
        };
        adapter
            .start_scan(filter)
            .await
            .map_err(|e| BleError::Adapter(e.to_string()))?;
        let result = async {
            let mut seen: Vec<String> = Vec::new();
            for _ in 0..(secs * SCAN_POLLS_PER_SECOND) {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                sleep(Duration::from_millis(SCAN_POLL_INTERVAL_MS)).await;
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let peripherals = adapter
                    .peripherals()
                    .await
                    .map_err(|e| BleError::Adapter(e.to_string()))?;
                for peripheral in peripherals {
                    let id = format!("{:?}", peripheral.id());
                    if seen.contains(&id) {
                        continue;
                    }
                    let Some(properties) = peripheral.properties().await.ok().flatten() else {
                        continue;
                    };
                    if !properties.services.is_empty()
                        && !properties
                            .services
                            .contains(&uuid_from_u16(role.service_u16()))
                    {
                        continue;
                    }
                    let name = properties.local_name.unwrap_or_else(|| "(unnamed)".into());
                    debug!("scan hit: {name} ({id})");
                    seen.push(id.clone());
                    on_found(ScanResult {
                        platform_id: id,
                        name,
                        rssi: properties.rssi,
                        role,
                    });
                }
            }
            Ok(())
        }
        .await;
        let _ = adapter.stop_scan().await;
        result
    }

    async fn find(adapter: &Adapter, platform_id: &str) -> Result<Peripheral, BleError> {
        let peripherals = adapter
            .peripherals()
            .await
            .map_err(|e| BleError::Adapter(e.to_string()))?;
        peripherals
            .into_iter()
            .find(|peripheral| format!("{:?}", peripheral.id()) == platform_id)
            .ok_or(BleError::NotFound)
    }

    /// Serialize only trainer connection setup. The returned connection does
    /// not retain the adapter-operation guard.
    pub async fn connect_trainer(
        &self,
        platform_id: &str,
    ) -> Result<FtmsTrainerConnection, BleError> {
        let _operation = self.operations.begin_connection().await;
        let adapter = self.adapter().await?;
        let peripheral = self
            .find_with_scan(adapter, Role::Trainer, platform_id, CONNECT_SCAN_DURATION_S)
            .await?;
        FtmsTrainerConnection::connect(adapter.clone(), peripheral).await
    }

    /// Serialize only HRM connection setup. Existing links continue operating.
    pub async fn connect_hrm(&self, platform_id: &str) -> Result<BleHeartRateConnection, BleError> {
        let _operation = self.operations.begin_connection().await;
        let adapter = self.adapter().await?;
        let peripheral = self
            .find_with_scan(adapter, Role::Hrm, platform_id, CONNECT_SCAN_DURATION_S)
            .await?;
        BleHeartRateConnection::connect(adapter.clone(), peripheral).await
    }

    /// Resolve a platform id, scanning until it appears or the bound expires.
    /// This targeted scan runs inside the connection's exclusive setup guard.
    async fn find_with_scan(
        &self,
        adapter: &Adapter,
        role: Role,
        platform_id: &str,
        secs: u64,
    ) -> Result<Peripheral, BleError> {
        if let Ok(peripheral) = Self::find(adapter, platform_id).await {
            return Ok(peripheral);
        }
        let filter = ScanFilter {
            services: vec![uuid_from_u16(role.service_u16())],
        };
        adapter
            .start_scan(filter)
            .await
            .map_err(|e| BleError::Adapter(e.to_string()))?;
        let mut found = Err(BleError::NotFound);
        for _ in 0..(secs * SCAN_POLLS_PER_SECOND) {
            sleep(Duration::from_millis(SCAN_POLL_INTERVAL_MS)).await;
            if let Ok(peripheral) = Self::find(adapter, platform_id).await {
                found = Ok(peripheral);
                break;
            }
        }
        let _ = adapter.stop_scan().await;
        sleep(Duration::from_millis(SCAN_SETTLE_MS)).await;
        found
    }
}

#[cfg(test)]
mod tests;
