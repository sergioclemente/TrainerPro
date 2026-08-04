//! tp-ble: BLE device layer for TrainerPro. SPEC.md §4.
//!
//! Layering: `codec` is pure byte parsing/building (unit-tested, no BLE);
//! `ftms`/`hrm` are btleplug drivers implementing the `traits` contracts;
//! `manager` scans and connects; `sim` is the fault-injectable simulator that
//! carries all development and CI (SPEC.md §4.5).

pub mod codec;
pub mod ftms;
pub mod hrm;
pub mod manager;
pub mod sim;
pub mod traits;

pub use manager::{DeviceManager, Role, ScanResult};
pub use traits::{BleError, DeviceStatus, HeartRateMonitor, HrData, Trainer, TrainerData};
