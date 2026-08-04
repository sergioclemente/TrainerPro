//! Hardware-agnostic device contracts. Everything above this layer (player
//! runtime, UI) sees only these traits — real FTMS hardware and the simulator
//! are interchangeable. SPEC.md §4.1.

use async_trait::async_trait;
use tokio::sync::{broadcast, watch};

#[derive(Debug, Clone, thiserror::Error)]
pub enum BleError {
    #[error("bluetooth adapter unavailable: {0}")]
    Adapter(String),
    #[error("device not found")]
    NotFound,
    #[error("device incompatible: {0}")]
    Incompatible(String),
    #[error("trainer refused control (result code {0:#04x})")]
    ControlRefused(u8),
    #[error("control point timeout")]
    Timeout,
    #[error("device disconnected")]
    Disconnected,
    #[error("ble transport: {0}")]
    Transport(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrainerData {
    pub power_w: Option<u16>,
    pub cadence_rpm: Option<f32>,
    pub speed_kmh: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HrData {
    pub bpm: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting { attempt: u32 },
}

/// A controllable smart trainer in ERG mode. Methods take `&self`; drivers
/// serialize control-point access internally (one op in flight, SPEC §4.2).
#[async_trait]
pub trait Trainer: Send + Sync {
    async fn set_target_power(&self, watts: u16) -> Result<(), BleError>;
    /// FreeRide segments: simulation mode, grade 0 %. SPEC §0.
    async fn set_sim_grade_zero(&self) -> Result<(), BleError>;
    async fn start(&self) -> Result<(), BleError>;
    async fn stop(&self) -> Result<(), BleError>;
    async fn reset(&self) -> Result<(), BleError>;
    fn telemetry(&self) -> broadcast::Receiver<TrainerData>;
    fn status(&self) -> watch::Receiver<DeviceStatus>;
    /// Human-readable device name for device_info / UI.
    fn name(&self) -> String;
}

#[async_trait]
pub trait HeartRateMonitor: Send + Sync {
    fn heart_rate(&self) -> broadcast::Receiver<HrData>;
    fn status(&self) -> watch::Receiver<DeviceStatus>;
    fn name(&self) -> String;
}
