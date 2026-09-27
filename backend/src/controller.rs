//! One selected controller source. Trainer input is borrowed; standalone input owns its link.

use crate::device_owner::{
    self as device, ConnectionAttempt, DeviceInput, DeviceState, DeviceStatus, Reply,
    COMMAND_CAPACITY,
};
use crate::trainer::Trainer;
use std::sync::Arc;
use tp_ble::controller::INPUT_CAPACITY;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerSource {
    TrainerControls,
    PairedController,
    Disabled,
}

use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::Instant;
use tp_ble::{
    BleError, ConnectionPriority, ConnectionStatus, ControllerInputEvent,
    StandaloneControllerConnection,
};

#[async_trait::async_trait]
pub trait ControllerConnector: Send + Sync {
    async fn connect(
        &self,
        platform_id: &str,
        priority: ConnectionPriority,
    ) -> Result<Box<dyn StandaloneControllerConnection>, BleError>;
}

/// One selected device at a time, with stable subscriptions across reconnects.
/// Clones refer to the same owner; only its worker owns the physical connection.
#[derive(Clone)]
pub struct Controller {
    commands: mpsc::Sender<Request>,
    state_rx: watch::Receiver<DeviceState>,
    inputs: broadcast::Sender<DeviceInput>,
}

enum Request {
    Connect {
        platform_id: String,
        expected_generation: Option<u64>,
        priority: ConnectionPriority,
        reply: Option<Reply<String>>,
    },
    Disconnect(Reply<()>),
    Source(ControllerSource, Reply<()>),
}

impl Controller {
    pub fn new(
        connector: Arc<dyn ControllerConnector>,
        trainer: Trainer,
        source: ControllerSource,
    ) -> Self {
        let (commands, requests) = mpsc::channel(COMMAND_CAPACITY);
        let (state_tx, state_rx) = watch::channel(DeviceState {
            controller_source: Some(source),
            ..DeviceState::default()
        });
        let (inputs, _) = broadcast::channel(INPUT_CAPACITY);
        tauri::async_runtime::spawn(run(
            connector,
            trainer,
            source,
            requests,
            state_tx,
            inputs.clone(),
        ));
        Self {
            commands,
            state_rx,
            inputs,
        }
    }

    pub fn state(&self) -> DeviceState {
        self.state_rx.borrow().clone()
    }
    pub fn subscribe_state(&self) -> watch::Receiver<DeviceState> {
        self.state_rx.clone()
    }
    pub fn subscribe_inputs(&self) -> broadcast::Receiver<DeviceInput> {
        self.inputs.subscribe()
    }

    pub async fn connect(&self, platform_id: &str) -> Result<String, BleError> {
        self.request(|reply| Request::Connect {
            platform_id: platform_id.into(),
            expected_generation: None,
            priority: ConnectionPriority::Foreground,
            reply: Some(reply),
        })
        .await
    }

    /// Select a saved device and keep trying until it connects or a later user
    /// action invalidates this generation.
    pub async fn maintain_if_generation(
        &self,
        platform_id: &str,
        generation: u64,
        initial_priority: ConnectionPriority,
    ) -> Result<(), BleError> {
        self.commands
            .send(Request::Connect {
                platform_id: platform_id.into(),
                expected_generation: Some(generation),
                priority: initial_priority,
                reply: None,
            })
            .await
            .map_err(|_| BleError::Disconnected)
    }

    /// Cancels retries immediately. Completion also waits for any in-flight
    /// attempt to finish and closes its result before acknowledging disconnect.
    pub async fn select_source(&self, source: ControllerSource) -> Result<(), BleError> {
        if source == ControllerSource::PairedController {
            return Err(BleError::Incompatible(
                "Select a paired controller first".into(),
            ));
        }
        self.request(|reply| Request::Source(source, reply)).await
    }

    pub async fn disconnect(&self) -> Result<(), BleError> {
        self.request(Request::Disconnect).await
    }

    async fn request<T>(&self, request: impl FnOnce(Reply<T>) -> Request) -> Result<T, BleError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(request(reply))
            .await
            .map_err(|_| BleError::Disconnected)?;
        result.await.map_err(|_| BleError::Disconnected)?
    }
}

async fn retire(
    connection: &mut Option<Box<dyn StandaloneControllerConnection>>,
    status: &mut Option<watch::Receiver<ConnectionStatus>>,
    inputs: &mut Option<broadcast::Receiver<ControllerInputEvent>>,
) -> Result<(), BleError> {
    *status = None;
    *inputs = None;
    match connection.take() {
        Some(mut connection) => connection.disconnect().await,
        None => Ok(()),
    }
}

async fn run(
    connector: Arc<dyn ControllerConnector>,
    trainer: Trainer,
    source: ControllerSource,
    mut requests: mpsc::Receiver<Request>,
    status_tx: watch::Sender<DeviceState>,
    inputs_tx: broadcast::Sender<DeviceInput>,
) {
    let mut trainer_state = trainer.subscribe_state();
    let mut trainer_inputs = trainer.subscribe_inputs();
    let mut state = DeviceState {
        controller_source: Some(source),
        ..DeviceState::default()
    };
    if source == ControllerSource::TrainerControls {
        derive_trainer_controls_state(&mut state, &trainer_state.borrow());
    }
    status_tx.send_replace(state.clone());
    let mut connection: Option<Box<dyn StandaloneControllerConnection>> = None;
    let mut connection_status = None;
    let mut connection_inputs = None;
    let mut pending: Option<ConnectionAttempt<dyn StandaloneControllerConnection>> = None;
    let mut connect_reply: Option<Reply<String>> = None;
    let mut disconnect_replies: Vec<Reply<()>> = Vec::new();
    let mut retry_at = None;
    // None means a one-shot explicit attempt; Some tracks automatic recovery.
    let mut retry_attempt: Option<u32> = None;
    let mut next_priority = ConnectionPriority::Foreground;

    loop {
        tokio::select! {
            biased;
            changed = trainer_state.changed() => {
                if changed.is_err() { return; }
                let trainer = trainer_state.borrow_and_update().clone();
                if state.controller_source == Some(ControllerSource::TrainerControls) {
                    state.generation += 1;
                    derive_trainer_controls_state(&mut state, &trainer);
                    status_tx.send_replace(state.clone());
                }
            }
            result = trainer_inputs.recv() => {
                if state.controller_source != Some(ControllerSource::TrainerControls) { continue; }
                match result {
                    Ok(input) if state.is_connected() && trainer_state.borrow().generation == input.generation && trainer_state.borrow().is_connected() => {
                        let _ = inputs_tx.send(DeviceInput { generation: state.generation, ..input });
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        state.generation += 1;
                        status_tx.send_replace(state.clone());
                        if let Some(profile) = state.controller_profile {
                            let _ = inputs_tx.send(DeviceInput { generation: state.generation, profile, event: ControllerInputEvent::Cancel });
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                    _ => {},
                }
            }
            request = requests.recv() => {
                let Some(request) = request else {
                    state.generation += 1;
                    state.error = None;
                    state.controller_profile = None;
                    state.status = DeviceStatus::Disconnected;
                    status_tx.send_replace(state);
                    let _ = retire(&mut connection, &mut connection_status, &mut connection_inputs).await;
                    if pending.is_some() {
                        if let (_, Ok(mut connection)) = device::finish_attempt(&mut pending).await {
                            let _ = connection.disconnect().await;
                        }
                    }
                    return;
                };
                match request {
                    Request::Connect {
                        platform_id,
                        expected_generation,
                        priority,
                        reply,
                    } => {
                        if expected_generation.is_some_and(|generation| generation != state.generation) {
                            if let Some(reply) = reply { let _ = reply.send(Err(BleError::Disconnected)); }
                            continue;
                        }
                        if let Some(reply) = connect_reply.take() { let _ = reply.send(Err(BleError::Disconnected)); }
                        state.generation += 1;
                    state.error = None;
                    state.controller_profile = None;
                        state.controller_source = Some(ControllerSource::PairedController);
                        state.platform_id = Some(platform_id);
                        state.name = None;
                        state.status = DeviceStatus::Connecting;
                        status_tx.send_replace(state.clone());
                        if let Err(error) = retire(&mut connection, &mut connection_status, &mut connection_inputs).await {
                            state.status = DeviceStatus::Disconnected;
                            status_tx.send_replace(state.clone());
                            if let Some(reply) = reply { let _ = reply.send(Err(error)); }
                            retry_at = None;
                        } else {
                            retry_attempt = reply.is_none().then_some(0);
                            connect_reply = reply;
                            next_priority = priority;
                            retry_at = Some(Instant::now());
                        }
                    }
                    Request::Disconnect(reply) => {
                        state.generation += 1;
                    state.error = None;
                    state.controller_profile = None;
                        if state.controller_source == Some(ControllerSource::TrainerControls) { state.controller_source = Some(ControllerSource::Disabled); }
                        state.platform_id = None;
                        state.name = None;
                        state.status = DeviceStatus::Disconnected;
                        status_tx.send_replace(state.clone());
                        retry_at = None;
                        retry_attempt = None;
                        if let Some(reply) = connect_reply.take() { let _ = reply.send(Err(BleError::Disconnected)); }
                        let result = retire(&mut connection, &mut connection_status, &mut connection_inputs).await;
                        if pending.is_some() && result.is_ok() { disconnect_replies.push(reply); }
                        else { let _ = reply.send(result); }
                    }

                    Request::Source(source, reply) => {
                        state.generation += 1;
                        state.controller_source = Some(source);
                        retry_at = None;
                        retry_attempt = None;
                        if let Some(reply) = connect_reply.take() { let _ = reply.send(Err(BleError::Disconnected)); }
                        let result = retire(&mut connection, &mut connection_status, &mut connection_inputs).await;
                        if source == ControllerSource::TrainerControls {
                            derive_trainer_controls_state(&mut state, &trainer_state.borrow());
                        } else {
                            state.platform_id = None;
                            state.name = None;
                            state.controller_profile = None;
                            state.error = None;
                            state.status = DeviceStatus::Disconnected;
                        }
                        trainer_inputs = trainer.subscribe_inputs();
                        status_tx.send_replace(state.clone());
                        if pending.is_some() && result.is_ok() { disconnect_replies.push(reply); }
                        else { let _ = reply.send(result); }
                    }
                }
            }
            _ = device::wait_for_retry(retry_at), if pending.is_none() => {
                retry_at = None;
                if let Some(platform_id) = state.platform_id.clone() {
                    if retry_attempt.is_some() {
                        state.status = DeviceStatus::Reconnecting;
                        status_tx.send_replace(state.clone());
                    }
                    let connector = connector.clone();
                    let priority = next_priority;
                    pending = Some(ConnectionAttempt {
                        generation: state.generation,
                        future: Box::pin(async move {
                            connector.connect(&platform_id, priority).await
                        }),
                    });
                }
            }
            (generation, result) = device::finish_attempt(&mut pending) => {
                pending = None;
                if generation != state.generation {
                    let cleanup = match result {
                        Ok(mut connection) => connection.disconnect().await,
                        Err(_) => Ok(()),
                    };
                    for reply in disconnect_replies.drain(..) { let _ = reply.send(cleanup.clone()); }
                    if let Err(error) = cleanup {
                        // Do not open another link after a failed retirement.
                        retry_at = None;
                        state.status = DeviceStatus::Disconnected;
                        status_tx.send_replace(state.clone());
                        if let Some(reply) = connect_reply.take() { let _ = reply.send(Err(error)); }
                    }
                    continue;
                }
                let result = match result {
                    Ok(mut connection) if *connection.subscribe_status().borrow() != ConnectionStatus::Connected => {
                        let _ = connection.disconnect().await;
                        Err(BleError::Disconnected)
                    }
                    result => result,
                };
                match result {
                    Ok(new_connection) => {
                        let name = new_connection.name().to_owned();
                        state.name = Some(name.clone());
                        state.error = None;
                        connection_status = Some(new_connection.subscribe_status());
                        let input = new_connection.controller_input();
                        state.controller_profile = Some(input.profile);
                        connection_inputs = Some(input.events);
                        connection = Some(new_connection);
                        state.status = DeviceStatus::Connected;
                        status_tx.send_replace(state.clone());
                        retry_attempt = None;
                        if let Some(reply) = connect_reply.take() { let _ = reply.send(Ok(name)); }
                    }
                    Err(error) => {
                        tracing::warn!("controller connection failed: {error}");
                        state.error = Some(error.to_string());
                        if matches!(error, BleError::Incompatible(_)) {
                            retry_attempt = None;
                            retry_at = None;
                        }
                        if let Some(attempt) = retry_attempt {
                            retry_attempt = Some(attempt + 1);
                            next_priority = ConnectionPriority::Background;
                            retry_at = Some(Instant::now() + device::retry_delay(attempt + 1));
                        } else {
                            state.status = DeviceStatus::Disconnected;
                            status_tx.send_replace(state.clone());
                            if let Some(reply) = connect_reply.take() { let _ = reply.send(Err(error)); }
                        }
                    }
                }
            }
            status = device::connection_status(&mut connection_status) => {
                if status == ConnectionStatus::Disconnected {
                    state.generation += 1;
                    state.error = None;
                    state.controller_profile = None;
                    state.status = DeviceStatus::Reconnecting;
                    status_tx.send_replace(state.clone());
                    // The status stream is authoritative: this link is already
                    // gone. Do not wait on a redundant CoreBluetooth disconnect,
                    // which can remain pending and prevent retry attempt 1.
                    connection_status = None;
                    connection_inputs = None;
                    connection = None;
                    retry_attempt = Some(0);
                    next_priority = ConnectionPriority::Background;
                    retry_at = Some(Instant::now() + device::retry_delay(0));
                }
            }
            result = device::connection_measurement(&mut connection_inputs) => {
                match result {
                    Ok(value) => {
                        if connection_status.as_ref().is_some_and(|rx| *rx.borrow() == ConnectionStatus::Connected) {
                            if let Some(profile) = state.controller_profile {
                                let _ = inputs_tx.send(DeviceInput { generation: state.generation, profile, event: value });
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        state.generation += 1;
                        status_tx.send_replace(state.clone());
                        if let Some(profile) = state.controller_profile {
                            let _ = inputs_tx.send(DeviceInput { generation: state.generation, profile, event: ControllerInputEvent::Cancel });
                        }
                    },
                    Err(broadcast::error::RecvError::Closed) => {
                        // A closed input stream is also a failed connection.
                        state.generation += 1;
                    state.error = None;
                    state.controller_profile = None;
                        state.status = DeviceStatus::Reconnecting;
                        status_tx.send_replace(state.clone());
                        if let Err(error) = retire(
                            &mut connection,
                            &mut connection_status,
                            &mut connection_inputs,
                        ).await {
                            tracing::warn!("controller cleanup after input stream closed failed: {error}");
                        }
                        retry_attempt = Some(0);
                        next_priority = ConnectionPriority::Background;
                        retry_at = Some(Instant::now() + device::retry_delay(0));
                    }
                }
            }
        }
    }
}

fn derive_trainer_controls_state(state: &mut DeviceState, trainer: &DeviceState) {
    state.platform_id = trainer.platform_id.clone();
    state.name = trainer.name.clone();
    state.controller_profile = trainer.controller_profile;
    state.error = None;
    state.status = if trainer.is_connected() && trainer.controller_profile.is_none() {
        DeviceStatus::Disconnected
    } else {
        trainer.status
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trainer::TrainerConnector;
    use std::collections::VecDeque;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };
    use tokio::time::{timeout, Duration};
    use tp_ble::{
        ControllerButton, ControllerInputStream, ControllerProfile, SimController,
        TrainerConnection, TrainerMeasurement,
    };

    const TEST_TIMEOUT: Duration = Duration::from_secs(3);

    #[derive(Clone)]
    struct Bike {
        status: watch::Sender<ConnectionStatus>,
        input: SimController,
        measurements: broadcast::Sender<TrainerMeasurement>,
        disconnects: Arc<AtomicUsize>,
    }
    impl Bike {
        fn new() -> Self {
            Self {
                status: watch::channel(ConnectionStatus::Connected).0,
                input: SimController::new(ControllerProfile::WahooVirtualBike),
                measurements: broadcast::channel(INPUT_CAPACITY).0,
                disconnects: Arc::new(AtomicUsize::new(0)),
            }
        }
    }
    #[async_trait::async_trait]
    impl TrainerConnection for Bike {
        async fn disconnect(&mut self) -> Result<(), BleError> {
            self.disconnects.fetch_add(1, Ordering::SeqCst);
            self.status.send_replace(ConnectionStatus::Disconnected);
            Ok(())
        }
        async fn probe_connection(&self) -> Result<bool, BleError> {
            Ok(*self.status.borrow() == ConnectionStatus::Connected)
        }
        async fn set_target_power(&self, _: u16) -> Result<(), BleError> {
            Ok(())
        }
        async fn set_flat_road_simulation(&self) -> Result<(), BleError> {
            Ok(())
        }
        async fn start_or_resume_training(&self) -> Result<(), BleError> {
            Ok(())
        }
        async fn pause_training(&self) -> Result<(), BleError> {
            Ok(())
        }
        async fn reset_trainer(&self) -> Result<(), BleError> {
            Ok(())
        }
        fn controller_input(&self) -> Option<ControllerInputStream> {
            Some(self.input.controller_input())
        }
        fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus> {
            self.status.subscribe()
        }
        fn subscribe_measurements(&self) -> broadcast::Receiver<TrainerMeasurement> {
            self.measurements.subscribe()
        }
        fn name(&self) -> &str {
            "Test BIKE SHIFT"
        }
    }
    #[async_trait::async_trait]
    impl TrainerConnector for Bike {
        async fn connect(
            &self,
            _: &str,
            _: ConnectionPriority,
        ) -> Result<Box<dyn TrainerConnection>, BleError> {
            Ok(Box::new(self.clone()))
        }
    }

    struct Rides {
        attempts: AtomicUsize,
        results: Mutex<VecDeque<Result<SimController, BleError>>>,
    }
    #[async_trait::async_trait]
    impl ControllerConnector for Rides {
        async fn connect(
            &self,
            _: &str,
            _: ConnectionPriority,
        ) -> Result<Box<dyn StandaloneControllerConnection>, BleError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            self.results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(BleError::NotFound))
                .map(|connection| Box::new(connection) as Box<dyn StandaloneControllerConnection>)
        }
    }
    fn rides(results: Vec<Result<SimController, BleError>>) -> Arc<Rides> {
        Arc::new(Rides {
            attempts: AtomicUsize::new(0),
            results: Mutex::new(results.into()),
        })
    }
    async fn status(rx: &mut watch::Receiver<DeviceState>, expected: DeviceStatus) -> DeviceState {
        timeout(TEST_TIMEOUT, async {
            loop {
                let value = rx.borrow_and_update().clone();
                if value.status == expected {
                    return value;
                }
                rx.changed().await.unwrap();
            }
        })
        .await
        .expect("device transition")
    }

    #[tokio::test]
    async fn disabling_borrowed_input_preserves_trainer_and_stable_subscriptions() {
        let bike = Bike::new();
        let trainer = Trainer::new(Arc::new(bike.clone()));
        let factory = rides(vec![]);
        let controller = Controller::new(
            factory.clone(),
            trainer.clone(),
            ControllerSource::TrainerControls,
        );
        let mut state = controller.subscribe_state();
        let mut inputs = controller.subscribe_inputs();
        trainer.connect("bike").await.unwrap();
        let connected = status(&mut state, DeviceStatus::Connected).await;
        bike.input.inject_button(ControllerButton::RightSteer, true);
        let event = timeout(TEST_TIMEOUT, inputs.recv()).await.unwrap().unwrap();
        assert_eq!(event.generation, connected.generation);
        controller
            .select_source(ControllerSource::Disabled)
            .await
            .unwrap();
        let disabled = status(&mut state, DeviceStatus::Disconnected).await;
        assert!(disabled.generation > connected.generation);
        assert!(trainer.is_connected());
        assert_eq!(bike.disconnects.load(Ordering::SeqCst), 0);
        assert_eq!(factory.attempts.load(Ordering::SeqCst), 0);
        controller
            .select_source(ControllerSource::TrainerControls)
            .await
            .unwrap();
        status(&mut state, DeviceStatus::Connected).await;
        bike.status.send_replace(ConnectionStatus::Disconnected);
        let lost = status(&mut state, DeviceStatus::Reconnecting).await;
        assert!(lost.generation > disabled.generation);
        controller
            .select_source(ControllerSource::Disabled)
            .await
            .unwrap();
        trainer.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn standalone_loss_does_not_touch_trainer_and_recovery_keeps_subscription() {
        let bike = Bike::new();
        let trainer = Trainer::new(Arc::new(bike.clone()));
        trainer.connect("bike").await.unwrap();
        let first = SimController::new(ControllerProfile::ZwiftRide);
        let second = SimController::new(ControllerProfile::ZwiftRide);
        let factory = rides(vec![Ok(first.clone()), Ok(second.clone())]);
        let controller = Controller::new(factory, trainer.clone(), ControllerSource::Disabled);
        let mut state = controller.subscribe_state();
        let mut inputs = controller.subscribe_inputs();
        controller.connect("ride").await.unwrap();
        let connected = status(&mut state, DeviceStatus::Connected).await;
        first.inject_disconnect();
        let lost = status(&mut state, DeviceStatus::Reconnecting).await;
        assert!(lost.generation > connected.generation);
        assert!(trainer.is_connected());
        assert_eq!(bike.disconnects.load(Ordering::SeqCst), 0);
        let recovered = status(&mut state, DeviceStatus::Connected).await;
        second.inject_button(ControllerButton::Y, true);
        timeout(TEST_TIMEOUT, async {
            loop {
                let input = inputs.recv().await.unwrap();
                if input.generation == recovered.generation
                    && matches!(
                        input.event,
                        ControllerInputEvent::Button { pressed: true, .. }
                    )
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        controller.disconnect().await.unwrap();
        trainer.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn unsupported_saved_firmware_stops_recovery_and_reports_error() {
        let trainer = Trainer::new(Arc::new(Bike::new()));
        let factory = rides(vec![Err(BleError::Incompatible(
            "unsupported firmware".into(),
        ))]);
        let controller =
            Controller::new(factory.clone(), trainer, ControllerSource::PairedController);
        let mut state = controller.subscribe_state();
        controller
            .maintain_if_generation("ride", 0, ConnectionPriority::Background)
            .await
            .unwrap();
        timeout(TEST_TIMEOUT, async {
            loop {
                if state.borrow_and_update().error.is_some() {
                    break;
                }
                state.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(state.borrow().status, DeviceStatus::Disconnected);
        assert!(state
            .borrow()
            .error
            .as_ref()
            .unwrap()
            .contains("unsupported firmware"));
        assert_eq!(factory.attempts.load(Ordering::SeqCst), 1);
        controller.disconnect().await.unwrap();
    }
}
