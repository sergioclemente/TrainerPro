//! tp-ble: BLE device layer for TrainerPro. SPEC.md §4.
//!
//! Layering: `codec` is pure byte parsing/building (unit-tested, no BLE); the
//! concrete connection types implement the `traits` contracts; `DeviceManager`
//! scans and connects; the simulated connections carry development and CI
//! (SPEC.md §4.5).

mod ble_heart_rate_connection;
pub mod codec;
mod connection_tasks;
mod device_manager;
mod ftms_trainer_connection;
mod sim_hrm;
mod sim_trainer;
pub mod traits;

pub use ble_heart_rate_connection::BleHeartRateConnection;
pub use device_manager::{ConnectionPriority, DeviceManager, Role, ScanResult};
pub use ftms_trainer_connection::FtmsTrainerConnection;
pub use sim_hrm::SimHrm;
pub use sim_trainer::SimTrainer;
pub use traits::{
    BleError, ConnectionStatus, HeartRateConnection, HeartRateMeasurement, TrainerConnection,
    TrainerMeasurement,
};
