//! Hardware-free controller with explicit press/release and disconnect injection.
use crate::controller::{ButtonEdges, INPUT_CAPACITY};
use crate::{
    BleError, ConnectionStatus, ControllerButton, ControllerInputEvent, ControllerInputStream,
    ControllerProfile, StandaloneControllerConnection,
};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, watch};

#[derive(Clone)]
pub struct SimController {
    profile: ControllerProfile,
    events: broadcast::Sender<ControllerInputEvent>,
    status: watch::Sender<ConnectionStatus>,
    buttons: Arc<Mutex<ButtonEdges>>,
}

impl SimController {
    pub const ID: &'static str = "sim-controller";
    pub fn new(profile: ControllerProfile) -> Self {
        Self {
            profile,
            events: broadcast::channel(INPUT_CAPACITY).0,
            status: watch::channel(ConnectionStatus::Connected).0,
            buttons: Arc::new(Mutex::new(ButtonEdges::default())),
        }
    }
    pub fn inject_button(&self, button: ControllerButton, pressed: bool) {
        if *self.status.borrow() != ConnectionStatus::Connected {
            return;
        }
        if let Some(event) = self.buttons.lock().unwrap().update(button, pressed) {
            let _ = self.events.send(event);
        }
    }
    pub fn inject_disconnect(&self) {
        self.status.send_replace(ConnectionStatus::Disconnected);
        let _ = self.events.send(ControllerInputEvent::Cancel);
    }
}

#[async_trait::async_trait]
impl StandaloneControllerConnection for SimController {
    async fn disconnect(&mut self) -> Result<(), BleError> {
        self.inject_disconnect();
        Ok(())
    }
    fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus> {
        self.status.subscribe()
    }
    fn controller_input(&self) -> ControllerInputStream {
        ControllerInputStream {
            profile: self.profile,
            events: self.events.subscribe(),
        }
    }
    fn name(&self) -> &str {
        match self.profile {
            ControllerProfile::WahooVirtualBike => "Simulated Wahoo controls",
            ControllerProfile::ZwiftRide => "Simulated Zwift Ride",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn duplicates_and_faults_use_existing_subscriptions() {
        let controller = SimController::new(ControllerProfile::ZwiftRide);
        let mut status = controller.subscribe_status();
        let mut input = controller.controller_input().events;
        controller.inject_button(ControllerButton::Y, true);
        controller.inject_button(ControllerButton::Y, true);
        assert!(matches!(
            input.recv().await.unwrap(),
            ControllerInputEvent::Button { pressed: true, .. }
        ));
        assert!(input.try_recv().is_err());
        controller.inject_disconnect();
        status.changed().await.unwrap();
        assert_eq!(*status.borrow(), ConnectionStatus::Disconnected);
        assert_eq!(input.recv().await.unwrap(), ControllerInputEvent::Cancel);
    }
}
