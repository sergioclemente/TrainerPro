//! BLE central coordination, discovery, and connection setup. SPEC.md §4.4.
//! Public and targeted scans are serialized; startup can initialize a resolved
//! peripheral while its shared saved-device scan continues. Resolved GATT
//! setups may proceed concurrently.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager as BtleManager, Peripheral};
use serde::Serialize;
use tokio::sync::{watch, Mutex, OnceCell, RwLock, RwLockReadGuard, RwLockWriteGuard};
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
const DISCOVERABLE_ROLES: [Role; 2] = [Role::Trainer, Role::Hrm];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Trainer,
    Hrm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionPriority {
    Foreground,
    Background,
    /// The peripheral was resolved by the active saved-device scan, so its
    /// GATT setup may begin without stopping or waiting for that scan.
    Startup,
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
    preemptible_by_background_connection: bool,
}

impl ScanSession {
    fn new(preemptible_by_background_connection: bool) -> Self {
        let (done, _) = watch::channel(false);
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            done,
            preemptible_by_background_connection,
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

/// Admission orders newly requested operations. Public and targeted scans take
/// exclusive operation access; ordinary connection setup takes shared access.
/// A resolved startup connection bypasses the operation lock so the one shared
/// saved-device scan can keep discovering the other role.
#[derive(Default)]
struct AdapterOperations {
    admission: Mutex<()>,
    operation: RwLock<()>,
    scan: StdMutex<Option<Arc<ScanSession>>>,
}

impl AdapterOperations {
    async fn quiesce_scan_locked(&self, priority: ConnectionPriority) {
        let active = { self.scan.lock().unwrap().clone() };
        if let Some(active) = active {
            if priority == ConnectionPriority::Foreground
                || active.preemptible_by_background_connection
            {
                active.cancel();
            }
            active.wait_until_complete().await;
        }
    }

    async fn begin_scan(&self) -> (RwLockWriteGuard<'_, ()>, Arc<ScanSession>) {
        let admission = self.admission.lock().await;
        self.quiesce_scan_locked(ConnectionPriority::Foreground)
            .await;
        let operation = self.operation.write().await;
        let session = Arc::new(ScanSession::new(false));
        *self.scan.lock().unwrap() = Some(session.clone());
        drop(admission);
        (operation, session)
    }

    async fn begin_connection(
        &self,
        priority: ConnectionPriority,
    ) -> Option<RwLockReadGuard<'_, ()>> {
        if priority == ConnectionPriority::Startup {
            return None;
        }
        let admission = self.admission.lock().await;
        self.quiesce_scan_locked(priority).await;
        let operation = self.operation.read().await;
        drop(admission);
        Some(operation)
    }

    async fn begin_connection_discovery(
        &self,
        priority: ConnectionPriority,
        preemptible_by_background_connection: bool,
    ) -> (RwLockWriteGuard<'_, ()>, Arc<ScanSession>) {
        let admission = self.admission.lock().await;
        self.quiesce_scan_locked(priority).await;
        let operation = self.operation.write().await;
        let session = Arc::new(ScanSession::new(preemptible_by_background_connection));
        *self.scan.lock().unwrap() = Some(session.clone());
        drop(admission);
        (operation, session)
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

/// Owns the BLE adapter and coordinates scans with connection setup. Setup for
/// already-resolved peripherals may overlap the shared startup scan and run
/// concurrently. Adapter initialization remains lazy so the application can
/// start without Bluetooth.
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

    /// A foreground action cancels public discovery; automatic recovery waits
    /// for it. Simulated connections use this boundary without adapter setup.
    pub async fn quiesce_scan(&self, priority: ConnectionPriority) {
        let _operation = self.operations.begin_connection(priority).await;
    }

    /// Time-boxed public discovery. A later connection request cancels this
    /// scan and waits for its adapter work to finish before connecting.
    pub async fn scan(
        &self,
        secs: u64,
        on_found: impl Fn(ScanResult) + Send,
    ) -> Result<(), BleError> {
        let (_operation, session) = self.operations.begin_scan().await;
        let _active_scan = ActiveScan {
            operations: &self.operations,
            session: session.clone(),
        };
        let adapter = self.adapter().await?;
        self.scan_adapter(adapter, secs, session.stop.clone(), on_found)
            .await
    }

    /// Discover configured physical devices in one scan and announce each role
    /// as soon as its peripheral enters the adapter cache. Startup connections
    /// may initialize from those announcements while discovery continues.
    pub async fn discover_saved(
        &self,
        targets: &[(Role, String)],
        mut on_found: impl FnMut(Role) + Send,
    ) -> Result<Vec<Role>, BleError> {
        if targets.is_empty() {
            return Ok(Vec::new());
        }

        let (_operation, session) = self.operations.begin_scan().await;
        let _active_scan = ActiveScan {
            operations: &self.operations,
            session: session.clone(),
        };
        let adapter = self.adapter().await?;
        let mut found = Self::found_target_roles(adapter, targets).await?;
        let mut announced = Vec::new();
        for role in found.iter().copied() {
            on_found(role);
            announced.push(role);
        }
        if found.len() == targets.len() {
            return Ok(found);
        }

        let mut services = Vec::new();
        for (role, _) in targets {
            let service = uuid_from_u16(role.service_u16());
            if !services.contains(&service) {
                services.push(service);
            }
        }
        adapter
            .start_scan(ScanFilter { services })
            .await
            .map_err(|e| BleError::Adapter(e.to_string()))?;

        let scan_result = async {
            for _ in 0..(CONNECT_SCAN_DURATION_S * SCAN_POLLS_PER_SECOND) {
                if session.stop.load(Ordering::Relaxed) {
                    break;
                }
                sleep(Duration::from_millis(SCAN_POLL_INTERVAL_MS)).await;
                if session.stop.load(Ordering::Relaxed) {
                    break;
                }
                found = Self::found_target_roles(adapter, targets).await?;
                let newly_found: Vec<Role> = found
                    .iter()
                    .copied()
                    .filter(|role| !announced.contains(role))
                    .collect();
                for role in newly_found {
                    on_found(role);
                    announced.push(role);
                }
                if found.len() == targets.len() {
                    break;
                }
            }
            Ok::<_, BleError>(found)
        }
        .await;
        let _ = adapter.stop_scan().await;
        sleep(Duration::from_millis(SCAN_SETTLE_MS)).await;
        scan_result
    }

    async fn scan_adapter(
        &self,
        adapter: &Adapter,
        secs: u64,
        stop: Arc<AtomicBool>,
        on_found: impl Fn(ScanResult) + Send,
    ) -> Result<(), BleError> {
        let filter = ScanFilter {
            services: DISCOVERABLE_ROLES
                .iter()
                .map(|role| uuid_from_u16(role.service_u16()))
                .collect(),
        };
        adapter
            .start_scan(filter)
            .await
            .map_err(|e| BleError::Adapter(e.to_string()))?;
        let result = async {
            let mut seen: Vec<(String, Role)> = Vec::new();
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
                    let Some(properties) = peripheral.properties().await.ok().flatten() else {
                        continue;
                    };
                    let name = properties.local_name.unwrap_or_else(|| "(unnamed)".into());
                    for role in DISCOVERABLE_ROLES {
                        if !properties
                            .services
                            .contains(&uuid_from_u16(role.service_u16()))
                        {
                            continue;
                        }
                        if seen.contains(&(id.clone(), role)) {
                            continue;
                        }
                        debug!("scan hit: {name} ({id}) as {role:?}");
                        seen.push((id.clone(), role));
                        on_found(ScanResult {
                            platform_id: id.clone(),
                            name: name.clone(),
                            rssi: properties.rssi,
                            role,
                        });
                    }
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

    async fn found_target_roles(
        adapter: &Adapter,
        targets: &[(Role, String)],
    ) -> Result<Vec<Role>, BleError> {
        let peripherals = adapter
            .peripherals()
            .await
            .map_err(|e| BleError::Adapter(e.to_string()))?;
        let ids: Vec<String> = peripherals
            .iter()
            .map(|peripheral| format!("{:?}", peripheral.id()))
            .collect();
        Ok(targets
            .iter()
            .filter_map(|(role, platform_id)| ids.contains(platform_id).then_some(*role))
            .collect())
    }

    /// Resolve the trainer, then keep shared setup access while its GATT
    /// connection initializes. A startup-resolved peripheral may initialize
    /// while the shared saved-device scan continues.
    pub async fn connect_trainer(
        &self,
        platform_id: &str,
        priority: ConnectionPriority,
    ) -> Result<FtmsTrainerConnection, BleError> {
        let operation = self.operations.begin_connection(priority).await;
        let adapter = self.adapter().await?;
        let (peripheral, _setup) = match Self::find(adapter, platform_id).await {
            Ok(peripheral) => (peripheral, operation),
            Err(_) => {
                if priority == ConnectionPriority::Startup {
                    return Err(BleError::NotFound);
                }
                drop(operation);
                let (discovery, session) = self
                    .operations
                    .begin_connection_discovery(priority, false)
                    .await;
                let active_scan = ActiveScan {
                    operations: &self.operations,
                    session: session.clone(),
                };
                let peripheral = self
                    .find_with_scan(
                        adapter,
                        Role::Trainer,
                        platform_id,
                        CONNECT_SCAN_DURATION_S,
                        session.stop.clone(),
                    )
                    .await?;
                drop(active_scan);
                (peripheral, Some(RwLockWriteGuard::downgrade(discovery)))
            }
        };
        FtmsTrainerConnection::connect(adapter.clone(), peripheral).await
    }

    /// Resolve the HRM, then keep shared setup access while its GATT connection
    /// initializes. A startup-resolved peripheral may initialize while the
    /// shared saved-device scan continues.
    pub async fn connect_hrm(
        &self,
        platform_id: &str,
        priority: ConnectionPriority,
    ) -> Result<BleHeartRateConnection, BleError> {
        let operation = self.operations.begin_connection(priority).await;
        let adapter = self.adapter().await?;
        let (peripheral, _setup) = match Self::find(adapter, platform_id).await {
            Ok(peripheral) => (peripheral, operation),
            Err(_) => {
                if priority == ConnectionPriority::Startup {
                    return Err(BleError::NotFound);
                }
                drop(operation);
                let (discovery, session) = self
                    .operations
                    .begin_connection_discovery(
                        priority,
                        priority == ConnectionPriority::Background,
                    )
                    .await;
                let active_scan = ActiveScan {
                    operations: &self.operations,
                    session: session.clone(),
                };
                let peripheral = self
                    .find_with_scan(
                        adapter,
                        Role::Hrm,
                        platform_id,
                        CONNECT_SCAN_DURATION_S,
                        session.stop.clone(),
                    )
                    .await?;
                drop(active_scan);
                (peripheral, Some(RwLockWriteGuard::downgrade(discovery)))
            }
        };
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
        stop: Arc<AtomicBool>,
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
            if stop.load(Ordering::Relaxed) {
                break;
            }
            sleep(Duration::from_millis(SCAN_POLL_INTERVAL_MS)).await;
            if stop.load(Ordering::Relaxed) {
                break;
            }
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
