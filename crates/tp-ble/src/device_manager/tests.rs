use std::time::Duration;

use super::*;
use tokio::sync::{oneshot, Notify};

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

#[tokio::test]
async fn connection_cancels_scan_and_waits_for_completion() {
    let operations = Arc::new(AdapterOperations::default());
    let (scan_guard, session) = operations.begin_scan().await;
    let (connected, mut connected_rx) = oneshot::channel();

    tokio::spawn({
        let operations = operations.clone();
        async move {
            let _connection_guard = operations
                .begin_connection(ConnectionPriority::Foreground)
                .await;
            let _ = connected.send(());
        }
    });

    tokio::time::timeout(TEST_TIMEOUT, async {
        while !session.stop.load(Ordering::Relaxed) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("connection did not cancel scan");

    operations.finish_scan(&session);
    assert!(matches!(
        connected_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    drop(scan_guard);
    tokio::time::timeout(TEST_TIMEOUT, connected_rx)
        .await
        .expect("connection did not acquire adapter operation")
        .expect("connection task ended");
}

#[tokio::test]
async fn resolved_startup_connection_does_not_wait_for_saved_device_scan() {
    let operations = AdapterOperations::default();
    let (_scan_guard, _session) = operations.begin_scan().await;

    tokio::time::timeout(
        TEST_TIMEOUT,
        operations.begin_connection(ConnectionPriority::Startup),
    )
    .await
    .expect("resolved startup connection waited for saved-device scan");
}

#[tokio::test]
async fn background_connection_waits_for_scan_without_cancelling_it() {
    let operations = Arc::new(AdapterOperations::default());
    let (scan_guard, session) = operations.begin_scan().await;
    let (connected, mut connected_rx) = oneshot::channel();

    tokio::spawn({
        let operations = operations.clone();
        async move {
            let _connection_guard = operations
                .begin_connection(ConnectionPriority::Background)
                .await;
            let _ = connected.send(());
        }
    });

    tokio::task::yield_now().await;
    assert!(!session.stop.load(Ordering::Relaxed));
    assert!(matches!(
        connected_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));

    operations.finish_scan(&session);
    drop(scan_guard);
    tokio::time::timeout(TEST_TIMEOUT, connected_rx)
        .await
        .expect("background connection did not follow scan")
        .expect("background connection task ended");
}

#[tokio::test]
async fn background_connection_cancels_preemptible_hrm_discovery() {
    let operations = Arc::new(AdapterOperations::default());
    let (scan_guard, session) = operations
        .begin_connection_discovery(ConnectionPriority::Background, true)
        .await;
    let (connected, mut connected_rx) = oneshot::channel();

    tokio::spawn({
        let operations = operations.clone();
        async move {
            let _connection_guard = operations
                .begin_connection(ConnectionPriority::Background)
                .await;
            let _ = connected.send(());
        }
    });

    tokio::time::timeout(TEST_TIMEOUT, async {
        while !session.stop.load(Ordering::Relaxed) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("trainer recovery did not cancel background HRM discovery");

    operations.finish_scan(&session);
    assert!(matches!(
        connected_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    drop(scan_guard);
    tokio::time::timeout(TEST_TIMEOUT, connected_rx)
        .await
        .expect("trainer recovery did not acquire adapter operation")
        .expect("connection task ended");
}

#[tokio::test]
async fn scan_requested_after_connection_waits_behind_it() {
    let operations = Arc::new(AdapterOperations::default());
    let connection_guard = operations
        .begin_connection(ConnectionPriority::Foreground)
        .await;
    let (scanning, mut scanning_rx) = oneshot::channel();

    tokio::spawn({
        let operations = operations.clone();
        async move {
            let (scan_guard, session) = operations.begin_scan().await;
            let _ = scanning.send(());
            operations.finish_scan(&session);
            drop(scan_guard);
        }
    });

    tokio::task::yield_now().await;
    assert!(matches!(
        scanning_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    drop(connection_guard);
    tokio::time::timeout(TEST_TIMEOUT, scanning_rx)
        .await
        .expect("scan did not acquire adapter operation")
        .expect("scan task ended");
}

#[tokio::test]
async fn resolved_connection_setups_can_overlap() {
    let operations = Arc::new(AdapterOperations::default());
    let first_connection = operations
        .begin_connection(ConnectionPriority::Foreground)
        .await;
    let (second_started, second_started_rx) = oneshot::channel();

    tokio::spawn({
        let operations = operations.clone();
        async move {
            let _second_connection = operations
                .begin_connection(ConnectionPriority::Foreground)
                .await;
            let _ = second_started.send(());
        }
    });

    tokio::time::timeout(TEST_TIMEOUT, second_started_rx)
        .await
        .expect("second connection setup was serialized")
        .expect("second connection task ended");
    drop(first_connection);
}

#[tokio::test]
async fn admitted_connection_stays_ahead_of_later_scan() {
    let operations = Arc::new(AdapterOperations::default());
    let (active_scan_guard, active_session) = operations.begin_scan().await;
    let release_connection = Arc::new(Notify::new());
    let (connected, connected_rx) = oneshot::channel();

    tokio::spawn({
        let operations = operations.clone();
        let release_connection = release_connection.clone();
        async move {
            let _connection_guard = operations
                .begin_connection(ConnectionPriority::Foreground)
                .await;
            let _ = connected.send(());
            release_connection.notified().await;
        }
    });

    tokio::time::timeout(TEST_TIMEOUT, async {
        while !active_session.stop.load(Ordering::Relaxed) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("connection was not admitted");

    let (scanning, mut scanning_rx) = oneshot::channel();
    tokio::spawn({
        let operations = operations.clone();
        async move {
            let (scan_guard, session) = operations.begin_scan().await;
            let _ = scanning.send(());
            operations.finish_scan(&session);
            drop(scan_guard);
        }
    });

    operations.finish_scan(&active_session);
    drop(active_scan_guard);
    tokio::time::timeout(TEST_TIMEOUT, connected_rx)
        .await
        .expect("connection did not acquire adapter operation")
        .expect("connection task ended");
    assert!(matches!(
        scanning_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));

    release_connection.notify_one();
    tokio::time::timeout(TEST_TIMEOUT, scanning_rx)
        .await
        .expect("later scan did not acquire adapter operation")
        .expect("scan task ended");
}
