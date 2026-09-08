use tp_ble::{ConnectionStatus, HeartRateConnection, SimHrm};

#[tokio::test(start_paused = true)]
async fn fault_injection_updates_existing_status_stream() {
    let hrm = SimHrm::new(futures::stream::pending());
    let mut status = hrm.subscribe_status();

    hrm.inject_disconnect();
    status.changed().await.unwrap();
    assert_eq!(*status.borrow(), ConnectionStatus::Disconnected);
    hrm.restore_connection();
    status.changed().await.unwrap();
    assert_eq!(*status.borrow(), ConnectionStatus::Connected);
}
