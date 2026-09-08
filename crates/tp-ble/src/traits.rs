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
pub struct TrainerMeasurement {
    pub power_w: Option<u16>,
    pub cadence_rpm: Option<f32>,
    pub speed_kmh: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeartRateMeasurement {
    pub bpm: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
}

/// A controllable smart trainer in ERG mode. Methods take `&self`; drivers
/// serialize control-point access internally (one op in flight, SPEC §4.2).
#[async_trait]
pub trait TrainerConnection: Send + Sync {
    /// Close the link and stop this connection's background tasks.
    async fn disconnect(&mut self) -> Result<(), BleError>;
    /// One-off transport check used to gate ride start. Ongoing connection
    /// changes are delivered through `subscribe_status()` rather than polled.
    async fn probe_connection(&self) -> Result<bool, BleError>;
    async fn set_target_power(&self, watts: u16) -> Result<(), BleError>;
    /// FreeRide segments: simulation mode, grade 0 %. SPEC §0.
    async fn set_flat_road_simulation(&self) -> Result<(), BleError>;
    async fn start_or_resume_training(&self) -> Result<(), BleError>;
    async fn pause_training(&self) -> Result<(), BleError>;
    async fn reset_trainer(&self) -> Result<(), BleError>;
    fn subscribe_measurements(&self) -> broadcast::Receiver<TrainerMeasurement>;
    fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus>;
    /// Human-readable device name for device_info / UI.
    fn name(&self) -> &str;
}

#[async_trait]
pub trait HeartRateConnection: Send + Sync {
    /// Close the link and stop this connection's background tasks.
    async fn disconnect(&mut self) -> Result<(), BleError>;
    fn subscribe_measurements(&self) -> broadcast::Receiver<HeartRateMeasurement>;
    fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus>;
    fn name(&self) -> &str;
}
