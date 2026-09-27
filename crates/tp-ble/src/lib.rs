//! `tp-ble`: BLE device layer for TrainerPro.
//!
//! Layering: `codec` is pure byte parsing/building (unit-tested, no BLE); the
//! concrete connection types implement the `traits` contracts; `DeviceManager`
//! scans and connects; the simulated connections carry development and CI
//! through the same public contracts.

mod ble_heart_rate_connection;
pub mod codec;
mod connection_tasks;
pub mod controller;
mod device_manager;
mod ftms_trainer_connection;
mod sim_controller;
mod sim_hrm;
mod sim_trainer;
pub mod traits;
mod wahoo_virtual_bike_protocol;
mod zwift_ride_connection;
mod zwift_ride_protocol;

pub use ble_heart_rate_connection::BleHeartRateConnection;
pub use controller::{
    ControllerButton, StandaloneControllerConnection, ControllerInputStream, ControllerInputEvent,
    ControllerProfile,
};
pub use device_manager::{ConnectionPriority, DeviceManager, Role, ScanResult};
pub use ftms_trainer_connection::FtmsTrainerConnection;
pub use sim_controller::SimController;
pub use sim_hrm::SimHrm;
pub use sim_trainer::SimTrainer;
pub use traits::{
    BleError, ConnectionStatus, HeartRateConnection, HeartRateMeasurement, TrainerConnection,
    TrainerMeasurement,
};
pub use zwift_ride_connection::ZwiftRideConnection;
