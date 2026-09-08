//! Long-lived heart-rate monitor owner. Connection replacement is private to this module.

use crate::device::{
    self, ConnectionAttempt, DeviceState, DeviceStatus, Measurement, Reply, COMMAND_CAPACITY,
    MEASUREMENT_CAPACITY,
};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::Instant;
use tp_ble::{BleError, ConnectionStatus, HeartRateConnection, HeartRateMeasurement};

#[async_trait::async_trait]
pub trait HeartRateConnector: Send + Sync {
    async fn connect(&self, platform_id: &str) -> Result<Box<dyn HeartRateConnection>, BleError>;
}

/// One selected device at a time, with stable subscriptions across reconnects.
/// Clones refer to the same owner; only its worker owns the physical connection.
#[derive(Clone)]
pub struct HeartRateMonitor {
    commands: mpsc::Sender<Request>,
    state_rx: watch::Receiver<DeviceState>,
    measurements: broadcast::Sender<Measurement<HeartRateMeasurement>>,
}

enum Request {
    Connect(String, Option<u64>, Reply<String>),
    Disconnect(Reply<()>),
}

impl HeartRateMonitor {
    pub fn new(connector: Arc<dyn HeartRateConnector>) -> Self {
        let (commands, requests) = mpsc::channel(COMMAND_CAPACITY);
        let (state_tx, state_rx) = watch::channel(DeviceState::default());
        let (measurements, _) = broadcast::channel(MEASUREMENT_CAPACITY);
        tokio::spawn(run(connector, requests, state_tx, measurements.clone()));
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
    pub fn subscribe_measurements(&self) -> broadcast::Receiver<Measurement<HeartRateMeasurement>> {
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
    connection: &mut Option<Box<dyn HeartRateConnection>>,
    status: &mut Option<watch::Receiver<ConnectionStatus>>,
    measurements: &mut Option<broadcast::Receiver<HeartRateMeasurement>>,
) -> Result<(), BleError> {
    *status = None;
    *measurements = None;
    match connection.take() {
        Some(mut connection) => connection.disconnect().await,
        None => Ok(()),
    }
}

async fn run(
    connector: Arc<dyn HeartRateConnector>,
    mut requests: mpsc::Receiver<Request>,
    status_tx: watch::Sender<DeviceState>,
    measurements_tx: broadcast::Sender<Measurement<HeartRateMeasurement>>,
) {
    let mut state = DeviceState::default();
    let mut connection: Option<Box<dyn HeartRateConnection>> = None;
    let mut connection_status = None;
    let mut connection_measurements = None;
    let mut pending: Option<ConnectionAttempt<dyn HeartRateConnection>> = None;
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
                        tracing::warn!("heart_rate_monitor connection failed: {error}");
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
                        tracing::warn!("heart-rate cleanup after link loss failed: {error}");
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
                            tracing::warn!("heart-rate cleanup after measurement stream closed failed: {error}");
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;

    const TEST_TIMEOUT: Duration = Duration::from_secs(1);

    #[derive(Clone)]
    struct FakeConnectionControl {
        status: watch::Sender<ConnectionStatus>,
        measurements: broadcast::Sender<HeartRateMeasurement>,
        disconnects: Arc<AtomicUsize>,
    }

    impl FakeConnectionControl {
        fn new() -> Self {
            let (status, _) = watch::channel(ConnectionStatus::Connected);
            let (measurements, _) = broadcast::channel(MEASUREMENT_CAPACITY);
            Self {
                status,
                measurements,
                disconnects: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    struct FakeConnection {
        control: FakeConnectionControl,
    }

    #[async_trait::async_trait]
    impl HeartRateConnection for FakeConnection {
        async fn disconnect(&mut self) -> Result<(), BleError> {
            self.control.disconnects.fetch_add(1, Ordering::SeqCst);
            self.control
                .status
                .send_replace(ConnectionStatus::Disconnected);
            Ok(())
        }

        fn subscribe_measurements(&self) -> broadcast::Receiver<HeartRateMeasurement> {
            self.control.measurements.subscribe()
        }

        fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus> {
            self.control.status.subscribe()
        }

        fn name(&self) -> &str {
            "Fake HRM"
        }
    }

    #[derive(Default)]
    struct FakeConnector {
        connections: Mutex<Vec<FakeConnectionControl>>,
    }

    #[async_trait::async_trait]
    impl HeartRateConnector for FakeConnector {
        async fn connect(
            &self,
            _platform_id: &str,
        ) -> Result<Box<dyn HeartRateConnection>, BleError> {
            let control = FakeConnectionControl::new();
            self.connections.lock().unwrap().push(control.clone());
            Ok(Box::new(FakeConnection { control }))
        }
    }

    impl FakeConnector {
        fn connection(&self, index: usize) -> FakeConnectionControl {
            self.connections.lock().unwrap()[index].clone()
        }
    }

    #[tokio::test]
    async fn existing_subscriptions_survive_connection_replacement() {
        let connector = Arc::new(FakeConnector::default());
        let monitor = HeartRateMonitor::new(connector.clone());
        let mut owner_state_rx = monitor.subscribe_state();
        let mut owner_measurements = monitor.subscribe_measurements();

        monitor.connect("hrm-1").await.unwrap();
        let first_generation = monitor.state().generation;
        let first = connector.connection(0);

        // The owner state receiver exists before the injected link fault.
        first.status.send_replace(ConnectionStatus::Disconnected);
        let reconnected = tokio::time::timeout(TEST_TIMEOUT, async {
            loop {
                let state = owner_state_rx.borrow_and_update().clone();
                if state.is_connected() && state.generation > first_generation {
                    break state;
                }
                owner_state_rx.changed().await.unwrap();
            }
        })
        .await
        .expect("heart-rate reconnect did not complete");

        assert_eq!(first.disconnects.load(Ordering::SeqCst), 1);
        let second = connector.connection(1);
        second
            .measurements
            .send(HeartRateMeasurement { bpm: 147 })
            .unwrap();
        let sample = tokio::time::timeout(TEST_TIMEOUT, owner_measurements.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sample.generation, reconnected.generation);
        assert_eq!(sample.value.bpm, 147);
        assert!(monitor.state().accepts(&sample));
    }
}
