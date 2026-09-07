//! Values shared by the two device owners. Connectivity has one public source:
//! the owner's state stream; connection status is an internal input to it.

use std::{future::Future, pin::Pin};
use tokio::sync::{broadcast, oneshot, watch};
use tokio::time::{Duration, Instant};
use tp_ble::{BleError, ConnectionStatus};
use tp_core::consts::{RECONNECT_SCHEDULE_S, RECONNECT_STEADY_S};

pub const COMMAND_CAPACITY: usize = 16;
pub const MEASUREMENT_CAPACITY: usize = 32;
pub type Reply<T> = oneshot::Sender<Result<T, BleError>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting { attempt: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceState {
    pub status: DeviceStatus,
    /// Changes when a connection is invalidated, even if an observer misses
    /// the intermediate disconnected state during a fast reconnect.
    pub generation: u64,
    pub platform_id: Option<String>,
    pub name: Option<String>,
}

impl Default for DeviceState {
    fn default() -> Self {
        Self {
            status: DeviceStatus::Disconnected,
            generation: 0,
            platform_id: None,
            name: None,
        }
    }
}

impl DeviceState {
    pub fn is_connected(&self) -> bool {
        self.status == DeviceStatus::Connected
    }

    pub fn accepts<T>(&self, sample: &Measurement<T>) -> bool {
        self.is_connected() && self.generation == sample.generation
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Measurement<T> {
    pub generation: u64,
    pub value: T,
}

/// Keep an in-flight transport future alive through cancellation of user
/// intent. On completion the owner either installs it or closes the stale
/// connection; dropping a CoreBluetooth operation mid-flight is avoided.
pub struct ConnectionAttempt<C: ?Sized> {
    pub generation: u64,
    pub future: Pin<Box<dyn Future<Output = Result<Box<C>, BleError>> + Send>>,
}

pub async fn finish_attempt<C: ?Sized>(
    attempt: &mut Option<ConnectionAttempt<C>>,
) -> (u64, Result<Box<C>, BleError>) {
    match attempt {
        Some(attempt) => (attempt.generation, attempt.future.as_mut().await),
        None => std::future::pending().await,
    }
}

pub async fn connection_status(
    rx: &mut Option<watch::Receiver<ConnectionStatus>>,
) -> ConnectionStatus {
    match rx {
        Some(rx) => {
            if *rx.borrow() == ConnectionStatus::Disconnected {
                return ConnectionStatus::Disconnected;
            }
            match rx.changed().await {
                Ok(()) => *rx.borrow_and_update(),
                Err(_) => ConnectionStatus::Disconnected,
            }
        }
        None => std::future::pending().await,
    }
}

pub async fn connection_measurement<T: Clone>(
    rx: &mut Option<broadcast::Receiver<T>>,
) -> Result<T, broadcast::error::RecvError> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

pub async fn wait_for_retry(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

pub fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(
        RECONNECT_SCHEDULE_S
            .get(attempt as usize)
            .copied()
            .unwrap_or(RECONNECT_STEADY_S),
    )
}
