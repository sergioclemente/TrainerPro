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
            let _connection_guard = operations.begin_connection().await;
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
async fn scan_requested_after_connection_waits_behind_it() {
    let operations = Arc::new(AdapterOperations::default());
    let connection_guard = operations.begin_connection().await;
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
async fn admitted_connection_stays_ahead_of_later_scan() {
    let operations = Arc::new(AdapterOperations::default());
    let (active_scan_guard, active_session) = operations.begin_scan().await;
    let release_connection = Arc::new(Notify::new());
    let (connected, connected_rx) = oneshot::channel();

    tokio::spawn({
        let operations = operations.clone();
        let release_connection = release_connection.clone();
        async move {
            let _connection_guard = operations.begin_connection().await;
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
