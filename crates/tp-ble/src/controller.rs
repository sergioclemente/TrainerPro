//! Controller input contracts. An input subscription never owns a BLE link.
use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::{broadcast, watch};

use crate::{BleError, ConnectionStatus};

pub const INPUT_CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerProfile {
    WahooVirtualBike,
    ZwiftRide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerButton {
    LeftSteer,
    RightSteer,
    LeftUp,
    LeftDown,
    RightUp,
    RightDown,
    LeftShiftUp,
    LeftShiftDown,
    RightShiftUp,
    RightShiftDown,
    LeftBrake,
    RightBrake,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    A,
    B,
    Y,
    Z,
    LeftPower,
    RightPower,
    LeftOnOff,
    RightOnOff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControllerInputEvent {
    Button {
        button: ControllerButton,
        pressed: bool,
    },
    /// Input continuity was lost. Never interpret this as a button release.
    Cancel,
}

pub struct ControllerInputStream {
    pub profile: ControllerProfile,
    pub events: broadcast::Receiver<ControllerInputEvent>,
}

#[async_trait]
pub trait StandaloneControllerConnection: Send + Sync {
    async fn disconnect(&mut self) -> Result<(), BleError>;
    fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus>;
    fn controller_input(&self) -> ControllerInputStream;
    fn name(&self) -> &str;
}

/// Duplicate notifications are normal. Releases affect only their own button.
#[derive(Default)]
pub(crate) struct ButtonEdges(std::collections::HashSet<ControllerButton>);

impl ButtonEdges {
    pub fn update(
        &mut self,
        button: ControllerButton,
        pressed: bool,
    ) -> Option<ControllerInputEvent> {
        let changed = if pressed {
            self.0.insert(button)
        } else {
            self.0.remove(&button)
        };
        changed.then_some(ControllerInputEvent::Button { button, pressed })
    }
}
