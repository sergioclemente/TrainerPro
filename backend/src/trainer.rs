//! Long-lived trainer owner. Connection replacement is private to this module.

use crate::device::{
    self, ConnectionAttempt, DeviceState, DeviceStatus, Measurement, Reply, COMMAND_CAPACITY,
    MEASUREMENT_CAPACITY,
};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::Instant;
use tp_ble::{BleError, ConnectionStatus, TrainerConnection, TrainerMeasurement};

#[async_trait::async_trait]
pub trait TrainerConnector: Send + Sync {
    async fn connect(&self, platform_id: &str) -> Result<Box<dyn TrainerConnection>, BleError>;
}

/// One selected device at a time, with stable subscriptions across reconnects.
/// Clones refer to the same owner; only its worker owns the physical connection.
#[derive(Clone)]
pub struct Trainer {
    commands: mpsc::Sender<Request>,
    state_rx: watch::Receiver<DeviceState>,
    measurements: broadcast::Sender<Measurement<TrainerMeasurement>>,
}

enum Request {
    Connect(String, Option<u64>, Reply<String>),
    Disconnect(Reply<()>),
    ProbeConnection(Reply<bool>),
    SetTargetPower(u16, u64, Reply<()>),
    SetFlatRoadSimulation(u64, Reply<()>),
    StartTraining(u64, Reply<()>),
    PauseTraining(u64, Reply<()>),
    ResetTrainer(u64, Reply<()>),
}

impl Trainer {
    pub fn new(connector: Arc<dyn TrainerConnector>) -> Self {
        let (commands, requests) = mpsc::channel(COMMAND_CAPACITY);
        let (state_tx, state_rx) = watch::channel(DeviceState::default());
        let (measurements, _) = broadcast::channel(MEASUREMENT_CAPACITY);
        tauri::async_runtime::spawn(run(
            connector,
            requests,
            state_tx,
            measurements.clone(),
        ));
        Self {
            commands,
            state_rx,
            measurements,
        }
    }

    pub fn state(&self) -> DeviceState {
        self.state_rx.borrow().clone()
    }
    pub fn is_connected(&self) -> bool {
        self.state_rx.borrow().is_connected()
    }
    pub fn subscribe_state(&self) -> watch::Receiver<DeviceState> {
        self.state_rx.clone()
    }
    pub fn subscribe_measurements(&self) -> broadcast::Receiver<Measurement<TrainerMeasurement>> {
        self.measurements.subscribe()
    }

    pub async fn connect(&self, platform_id: &str) -> Result<String, BleError> {
        self.request(|reply| Request::Connect(platform_id.into(), None, reply))
            .await
    }

    /// Startup retries must not supersede a later user selection or disconnect.
    pub async fn connect_if_generation(
        &self,
        platform_id: &str,
        generation: u64,
    ) -> Result<String, BleError> {
        self.request(|reply| Request::Connect(platform_id.into(), Some(generation), reply))
            .await
    }

    /// Cancels retries immediately. Completion also waits for any in-flight
    /// attempt to finish and closes its result before acknowledging disconnect.
    pub async fn disconnect(&self) -> Result<(), BleError> {
        self.request(Request::Disconnect).await
    }

    /// One-off transport check at ride start; ongoing state uses subscribe_state.
    pub async fn probe_connection(&self) -> Result<bool, BleError> {
        self.request(Request::ProbeConnection).await
    }

    pub async fn set_target_power(&self, watts: u16) -> Result<(), BleError> {
        let generation = self.state().generation;
        self.request(|reply| Request::SetTargetPower(watts, generation, reply))
            .await
    }

    pub async fn set_flat_road_simulation(&self) -> Result<(), BleError> {
        let generation = self.state().generation;
        self.request(|reply| Request::SetFlatRoadSimulation(generation, reply))
            .await
    }

    pub async fn start_or_resume_training(&self) -> Result<(), BleError> {
        let generation = self.state().generation;
        self.request(|reply| Request::StartTraining(generation, reply))
            .await
    }

    pub async fn pause_training(&self) -> Result<(), BleError> {
        let generation = self.state().generation;
        self.request(|reply| Request::PauseTraining(generation, reply))
            .await
    }

    pub async fn reset_trainer(&self) -> Result<(), BleError> {
        let generation = self.state().generation;
        self.request(|reply| Request::ResetTrainer(generation, reply))
            .await
    }

    /// The simulated HRM observes this same stable measurement source.
    pub fn measurement_stream(
        &self,
    ) -> impl futures::Stream<Item = TrainerMeasurement> + Send + 'static {
        let rx = self.subscribe_measurements();
        let state = self.state_rx.clone();
        futures::stream::unfold((rx, state), |(mut rx, state)| async move {
            loop {
                match rx.recv().await {
                    Ok(sample) if state.borrow().accepts(&sample) => {
                        return Some((sample.value, (rx, state)))
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
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
    connection: &mut Option<Box<dyn TrainerConnection>>,
    status: &mut Option<watch::Receiver<ConnectionStatus>>,
    measurements: &mut Option<broadcast::Receiver<TrainerMeasurement>>,
) -> Result<(), BleError> {
    *status = None;
    *measurements = None;
    match connection.take() {
        Some(mut connection) => connection.disconnect().await,
        None => Ok(()),
    }
}

fn connection_for_command<'a>(
    connection: &'a Option<Box<dyn TrainerConnection>>,
    state: &DeviceState,
    generation: u64,
) -> Result<&'a dyn TrainerConnection, BleError> {
    let connection = connection.as_deref().ok_or(BleError::Disconnected)?;
    if state.is_connected()
        && generation == state.generation
        && *connection.subscribe_status().borrow() == ConnectionStatus::Connected
    {
        Ok(connection)
    } else {
        Err(BleError::Disconnected)
    }
}

async fn run(
    connector: Arc<dyn TrainerConnector>,
    mut requests: mpsc::Receiver<Request>,
    status_tx: watch::Sender<DeviceState>,
    measurements_tx: broadcast::Sender<Measurement<TrainerMeasurement>>,
) {
    let mut state = DeviceState::default();
    let mut connection: Option<Box<dyn TrainerConnection>> = None;
    let mut connection_status = None;
    let mut connection_measurements = None;
    let mut pending: Option<ConnectionAttempt<dyn TrainerConnection>> = None;
    let mut connect_reply: Option<Reply<String>> = None;
    let mut disconnect_replies: Vec<Reply<()>> = Vec::new();
    let mut retry_at = None;
    // None means an explicit initial attempt; Some means automatic recovery.
    let mut retry_attempt: Option<u32> = None;

    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else {
                    state.generation += 1;
                    state.status = DeviceStatus::Disconnected;
                    status_tx.send_replace(state);
                    let _ = retire(&mut connection, &mut connection_status, &mut connection_measurements).await;
                    if pending.is_some() {
                        if let (_, Ok(mut connection)) = device::finish_attempt(&mut pending).await {
                            let _ = connection.disconnect().await;
                        }
                    }
                    return;
                };
                match request {
                    Request::Connect(platform_id, expected_generation, reply) => {
                        if expected_generation.is_some_and(|generation| generation != state.generation) {
                            let _ = reply.send(Err(BleError::Disconnected));
                            continue;
                        }
                        if let Some(reply) = connect_reply.take() { let _ = reply.send(Err(BleError::Disconnected)); }
                        state.generation += 1;
                        state.platform_id = Some(platform_id);
                        state.name = None;
                        state.status = DeviceStatus::Connecting;
                        status_tx.send_replace(state.clone());
                        if let Err(error) = retire(&mut connection, &mut connection_status, &mut connection_measurements).await {
                            state.status = DeviceStatus::Disconnected;
                            status_tx.send_replace(state.clone());
                            let _ = reply.send(Err(error));
                            retry_at = None;
                        } else {
                            connect_reply = Some(reply);
                            retry_attempt = None;
                            retry_at = Some(Instant::now());
                        }
                    }
                    Request::Disconnect(reply) => {
                        state.generation += 1;
                        state.platform_id = None;
                        state.name = None;
                        state.status = DeviceStatus::Disconnected;
                        status_tx.send_replace(state.clone());
                        retry_at = None;
                        retry_attempt = None;
                        if let Some(reply) = connect_reply.take() { let _ = reply.send(Err(BleError::Disconnected)); }
                        let result = retire(&mut connection, &mut connection_status, &mut connection_measurements).await;
                        if pending.is_some() && result.is_ok() { disconnect_replies.push(reply); }
                        else { let _ = reply.send(result); }
                    }

                    Request::ProbeConnection(reply) => {
                        let result = match connection.as_ref() {
                            Some(connection) if state.is_connected() => connection.probe_connection().await,
                            _ => Ok(false),
                        };
                        let _ = reply.send(result);
                    }

                    Request::SetTargetPower(watts, generation, reply) => {
                        let result = match connection_for_command(&connection, &state, generation) {
                            Ok(connection) => connection.set_target_power(watts).await,
                            Err(error) => Err(error),
                        };
                        let _ = reply.send(result);
                    }

                    Request::SetFlatRoadSimulation(generation, reply) => {
                        let result = match connection_for_command(&connection, &state, generation) {
                            Ok(connection) => connection.set_flat_road_simulation().await,
                            Err(error) => Err(error),
                        };
                        let _ = reply.send(result);
                    }

                    Request::StartTraining(generation, reply) => {
                        let result = match connection_for_command(&connection, &state, generation) {
                            Ok(connection) => connection.start_or_resume_training().await,
                            Err(error) => Err(error),
                        };
                        let _ = reply.send(result);
                    }

                    Request::PauseTraining(generation, reply) => {
                        let result = match connection_for_command(&connection, &state, generation) {
                            Ok(connection) => connection.pause_training().await,
                            Err(error) => Err(error),
                        };
                        let _ = reply.send(result);
                    }

                    Request::ResetTrainer(generation, reply) => {
                        let result = match connection_for_command(&connection, &state, generation) {
                            Ok(connection) => connection.reset_trainer().await,
                            Err(error) => Err(error),
                        };
                        let _ = reply.send(result);
                    }

                }
            }
            _ = device::wait_for_retry(retry_at), if pending.is_none() => {
                retry_at = None;
                if let Some(platform_id) = state.platform_id.clone() {
                    if let Some(attempt) = retry_attempt {
                        state.status = DeviceStatus::Reconnecting { attempt: attempt + 1 };
                        status_tx.send_replace(state.clone());
                    }
                    let connector = connector.clone();
                    pending = Some(ConnectionAttempt {
                        generation: state.generation,
                        future: Box::pin(async move { connector.connect(&platform_id).await }),
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
                        connection_status = Some(new_connection.subscribe_status());
                        connection_measurements = Some(new_connection.subscribe_measurements());
                        connection = Some(new_connection);
                        state.status = DeviceStatus::Connected;
                        status_tx.send_replace(state.clone());
                        retry_attempt = None;
                        if let Some(reply) = connect_reply.take() { let _ = reply.send(Ok(name)); }
                    }
                    Err(error) => {
                        tracing::warn!("trainer connection failed: {error}");
                        if let Some(attempt) = retry_attempt {
                            retry_attempt = Some(attempt + 1);
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
                    state.status = DeviceStatus::Reconnecting { attempt: 0 };
                    status_tx.send_replace(state.clone());
                    if let Err(error) = retire(
                        &mut connection,
                        &mut connection_status,
                        &mut connection_measurements,
                    ).await {
                        tracing::warn!("trainer cleanup after link loss failed: {error}");
                    }
                    retry_attempt = Some(0);
                    retry_at = Some(Instant::now() + device::retry_delay(0));
                }
            }
            result = device::connection_measurement(&mut connection_measurements) => {
                match result {
                    Ok(value) => {
                        if connection_status.as_ref().is_some_and(|rx| *rx.borrow() == ConnectionStatus::Connected) {
                            let _ = measurements_tx.send(Measurement { generation: state.generation, value });
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {},
                    Err(broadcast::error::RecvError::Closed) => {
                        // A closed measurement stream is also a failed connection.
                        state.generation += 1;
                        state.status = DeviceStatus::Reconnecting { attempt: 0 };
                        status_tx.send_replace(state.clone());
                        if let Err(error) = retire(
                            &mut connection,
                            &mut connection_status,
                            &mut connection_measurements,
                        ).await {
                            tracing::warn!("trainer cleanup after measurement stream closed failed: {error}");
                        }
                        retry_attempt = Some(0);
                        retry_at = Some(Instant::now() + device::retry_delay(0));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;
    use tokio::sync::Notify;

    const TEST_TIMEOUT: Duration = Duration::from_secs(1);

    #[derive(Clone)]
    struct FakeConnectionControl {
        status: watch::Sender<ConnectionStatus>,
        measurements: broadcast::Sender<TrainerMeasurement>,
        disconnects: Arc<AtomicUsize>,
        fail_disconnect: Arc<AtomicBool>,
    }

    impl FakeConnectionControl {
        fn new() -> Self {
            let (status, _) = watch::channel(ConnectionStatus::Connected);
            let (measurements, _) = broadcast::channel(MEASUREMENT_CAPACITY);
            Self {
                status,
                measurements,
                disconnects: Arc::new(AtomicUsize::new(0)),
                fail_disconnect: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    struct FakeConnection {
        control: FakeConnectionControl,
    }

    #[async_trait::async_trait]
    impl TrainerConnection for FakeConnection {
        async fn disconnect(&mut self) -> Result<(), BleError> {
            self.control.disconnects.fetch_add(1, Ordering::SeqCst);
            self.control
                .status
                .send_replace(ConnectionStatus::Disconnected);
            if self.control.fail_disconnect.load(Ordering::SeqCst) {
                Err(BleError::Transport("cleanup failed".into()))
            } else {
                Ok(())
            }
        }

        async fn probe_connection(&self) -> Result<bool, BleError> {
            Ok(*self.control.status.borrow() == ConnectionStatus::Connected)
        }

        async fn set_target_power(&self, _watts: u16) -> Result<(), BleError> {
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

        fn subscribe_measurements(&self) -> broadcast::Receiver<TrainerMeasurement> {
            self.control.measurements.subscribe()
        }

        fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus> {
            self.control.status.subscribe()
        }

        fn name(&self) -> &str {
            "Fake trainer"
        }
    }

    #[derive(Default)]
    struct FakeConnector {
        connections: Mutex<Vec<FakeConnectionControl>>,
    }

    #[async_trait::async_trait]
    impl TrainerConnector for FakeConnector {
        async fn connect(
            &self,
            _platform_id: &str,
        ) -> Result<Box<dyn TrainerConnection>, BleError> {
            let control = FakeConnectionControl::new();
            self.connections.lock().unwrap().push(control.clone());
            Ok(Box::new(FakeConnection { control }))
        }
    }

    impl FakeConnector {
        fn connection(&self, index: usize) -> FakeConnectionControl {
            self.connections.lock().unwrap()[index].clone()
        }

        fn connection_count(&self) -> usize {
            self.connections.lock().unwrap().len()
        }
    }

    #[derive(Default)]
    struct DelayedConnector {
        started: Notify,
        release: Notify,
        connections: Mutex<Vec<FakeConnectionControl>>,
    }

    #[async_trait::async_trait]
    impl TrainerConnector for DelayedConnector {
        async fn connect(
            &self,
            _platform_id: &str,
        ) -> Result<Box<dyn TrainerConnection>, BleError> {
            self.started.notify_one();
            self.release.notified().await;
            let control = FakeConnectionControl::new();
            self.connections.lock().unwrap().push(control.clone());
            Ok(Box::new(FakeConnection { control }))
        }
    }

    async fn wait_until(
        status: &mut watch::Receiver<DeviceState>,
        predicate: impl Fn(&DeviceState) -> bool,
    ) -> DeviceState {
        tokio::time::timeout(TEST_TIMEOUT, async {
            loop {
                let current = status.borrow_and_update().clone();
                if predicate(&current) {
                    return current;
                }
                status.changed().await.unwrap();
            }
        })
        .await
        .expect("device state did not arrive")
    }

    #[tokio::test]
    async fn existing_subscriptions_survive_replacement_when_cleanup_fails() {
        let connector = Arc::new(FakeConnector::default());
        let trainer = Trainer::new(connector.clone());
        let mut owner_state_rx = trainer.subscribe_state();
        let mut owner_measurements = trainer.subscribe_measurements();

        trainer.connect("trainer-1").await.unwrap();
        let first_generation = trainer.state().generation;
        let first = connector.connection(0);
        first
            .measurements
            .send(TrainerMeasurement {
                power_w: Some(180),
                cadence_rpm: Some(90.0),
                speed_kmh: None,
            })
            .unwrap();
        let first_sample = tokio::time::timeout(TEST_TIMEOUT, owner_measurements.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first_sample.generation, first_generation);

        // The owner state receiver exists before the injected link fault.
        first.fail_disconnect.store(true, Ordering::SeqCst);
        first.status.send_replace(ConnectionStatus::Disconnected);
        let reconnected = wait_until(&mut owner_state_rx, |state| {
            state.is_connected() && state.generation > first_generation
        })
        .await;

        assert_eq!(connector.connection_count(), 2);
        assert_eq!(first.disconnects.load(Ordering::SeqCst), 1);
        let second = connector.connection(1);
        second
            .measurements
            .send(TrainerMeasurement {
                power_w: Some(205),
                cadence_rpm: Some(92.0),
                speed_kmh: None,
            })
            .unwrap();
        let second_sample = tokio::time::timeout(TEST_TIMEOUT, owner_measurements.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second_sample.generation, reconnected.generation);
        assert_eq!(second_sample.value.power_w, Some(205));
        assert!(trainer.state().accepts(&second_sample));
    }

    #[tokio::test]
    async fn stale_startup_generation_cannot_override_disconnect() {
        let connector = Arc::new(FakeConnector::default());
        let trainer = Trainer::new(connector.clone());
        let startup_generation = trainer.state().generation;

        trainer.disconnect().await.unwrap();
        assert!(matches!(
            trainer
                .connect_if_generation("saved-trainer", startup_generation)
                .await,
            Err(BleError::Disconnected)
        ));
        assert_eq!(connector.connection_count(), 0);
        assert!(trainer.state().platform_id.is_none());
    }

    #[tokio::test]
    async fn in_flight_connection_is_retired_after_disconnect() {
        let connector = Arc::new(DelayedConnector::default());
        let trainer = Trainer::new(connector.clone());

        let connect = tokio::spawn({
            let trainer = trainer.clone();
            async move { trainer.connect("trainer-1").await }
        });
        connector.started.notified().await;

        let disconnect = tokio::spawn({
            let trainer = trainer.clone();
            async move { trainer.disconnect().await }
        });
        assert!(matches!(
            connect.await.unwrap(),
            Err(BleError::Disconnected)
        ));
        assert!(!disconnect.is_finished());

        connector.release.notify_one();
        disconnect.await.unwrap().unwrap();

        let connections = connector.connections.lock().unwrap();
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].disconnects.load(Ordering::SeqCst), 1);
        assert_eq!(trainer.state().status, DeviceStatus::Disconnected);
        assert!(trainer.state().platform_id.is_none());
    }
}
