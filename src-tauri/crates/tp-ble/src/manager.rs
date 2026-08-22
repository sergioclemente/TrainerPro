//! Scan and connect. SPEC.md §4.4. Reconnect *policy* (backoff schedule,
//! auto-pause) lives in the tp-app runtime; this module only finds devices
//! and produces connected driver instances.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use serde::Serialize;
use tokio::time::{sleep, Duration};
use tracing::debug;

use crate::codec;
use crate::ftms::FtmsTrainer;
use crate::hrm::HrmDevice;
use crate::traits::BleError;

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

pub struct DeviceManager {
    adapter: Adapter,
}

impl DeviceManager {
    pub async fn new() -> Result<Self, BleError> {
        let manager = Manager::new().await.map_err(|e| BleError::Adapter(e.to_string()))?;
        let adapter = manager
            .adapters()
            .await
            .map_err(|e| BleError::Adapter(e.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| BleError::Adapter("no bluetooth adapter".into()))?;
        Ok(DeviceManager { adapter })
    }

    /// Scan for `secs`, invoking `on_found` for each newly discovered device
    /// advertising the role's service. Deduped by platform id. `stop` ends
    /// the scan early — REQUIRED before connecting (SPEC §4.4): CoreBluetooth
    /// races btleplug's operation futures when scan-poll property reads
    /// interleave with a connect on the same peripheral.
    pub async fn scan(
        &self,
        role: Role,
        secs: u64,
        stop: Arc<AtomicBool>,
        on_found: impl Fn(ScanResult) + Send,
    ) -> Result<(), BleError> {
        let filter = ScanFilter { services: vec![uuid_from_u16(role.service_u16())] };
        self.adapter
            .start_scan(filter)
            .await
            .map_err(|e| BleError::Adapter(e.to_string()))?;
        let mut seen: Vec<String> = Vec::new();
        for _ in 0..(secs * 2) {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            sleep(Duration::from_millis(500)).await;
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let peripherals = self
                .adapter
                .peripherals()
                .await
                .map_err(|e| BleError::Adapter(e.to_string()))?;
            for p in peripherals {
                let id = format!("{:?}", p.id());
                if seen.contains(&id) {
                    continue;
                }
                let Some(props) = p.properties().await.ok().flatten() else { continue };
                // CoreBluetooth honors the scan filter, but double-check the
                // advertised services when present.
                if !props.services.is_empty()
                    && !props.services.contains(&uuid_from_u16(role.service_u16()))
                {
                    continue;
                }
                let name = props.local_name.unwrap_or_else(|| "(unnamed)".into());
                debug!("scan hit: {name} ({id})");
                seen.push(id.clone());
                on_found(ScanResult { platform_id: id, name, rssi: props.rssi, role });
            }
        }
        let _ = self.adapter.stop_scan().await;
        Ok(())
    }

    async fn find(&self, platform_id: &str) -> Result<Peripheral, BleError> {
        // Stored ids are `format!("{:?}", PeripheralId)` — stable per machine
        // on macOS (opaque CoreBluetooth UUIDs, SPEC §4.4).
        let peripherals = self
            .adapter
            .peripherals()
            .await
            .map_err(|e| BleError::Adapter(e.to_string()))?;
        peripherals
            .into_iter()
            .find(|p| format!("{:?}", p.id()) == platform_id)
            .ok_or(BleError::NotFound)
    }

    /// Find + full FTMS connect sequence. Saved devices are not resolvable
    /// after app restart until the adapter has seen an advertisement, so
    /// when the cache misses we scan *until the target appears* (bounded).
    pub async fn connect_trainer(&self, platform_id: &str) -> Result<FtmsTrainer, BleError> {
        let p = self.find_with_scan(Role::Trainer, platform_id, 12).await?;
        FtmsTrainer::connect(self.adapter.clone(), p).await
    }

    pub async fn connect_hrm(&self, platform_id: &str) -> Result<HrmDevice, BleError> {
        let p = self.find_with_scan(Role::Hrm, platform_id, 12).await?;
        HrmDevice::connect(self.adapter.clone(), p).await
    }

    /// Resolve a platform id, scanning until it shows up or `secs` elapse.
    /// The scan is fully stopped before returning so the caller's connect
    /// never overlaps scan traffic (CoreBluetooth race, SPEC §4.4).
    async fn find_with_scan(
        &self,
        role: Role,
        platform_id: &str,
        secs: u64,
    ) -> Result<Peripheral, BleError> {
        if let Ok(p) = self.find(platform_id).await {
            return Ok(p);
        }
        let filter = ScanFilter { services: vec![uuid_from_u16(role.service_u16())] };
        self.adapter
            .start_scan(filter)
            .await
            .map_err(|e| BleError::Adapter(e.to_string()))?;
        let mut found = Err(BleError::NotFound);
        for _ in 0..(secs * 2) {
            sleep(Duration::from_millis(500)).await;
            if let Ok(p) = self.find(platform_id).await {
                found = Ok(p);
                break;
            }
        }
        let _ = self.adapter.stop_scan().await;
        // Give CoreBluetooth a beat to quiesce scan callbacks before connect.
        sleep(Duration::from_millis(300)).await;
        found
    }
}
