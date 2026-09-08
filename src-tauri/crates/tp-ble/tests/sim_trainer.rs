use std::time::Duration;

use tp_ble::{BleError, ConnectionStatus, SimTrainer, TrainerConnection};

const SIMULATOR_TICK: Duration = Duration::from_millis(250);
const CONVERGENCE_TICKS: usize = 40;

#[tokio::test(start_paused = true)]
async fn sim_converges_to_target() {
    let sim = SimTrainer::new();
    let mut rx = sim.subscribe_measurements();
    sim.start_or_resume_training().await.unwrap();
    sim.set_target_power(200).await.unwrap();
    // τ=1.5 s ⇒ well converged after 10 s of virtual time.
    let mut last = 0u16;
    for _ in 0..CONVERGENCE_TICKS {
        tokio::time::advance(SIMULATOR_TICK).await;
        if let Ok(d) = rx.try_recv() {
            last = d.power_w.unwrap_or(0);
        }
        while rx.try_recv().is_ok() {}
    }
    assert!((170..=230).contains(&last), "power={last} not near 200 W");
}

#[tokio::test(start_paused = true)]
async fn fault_injection_disconnect_and_restore() {
    let sim = SimTrainer::new();
    let mut status = sim.subscribe_status();
    sim.inject_disconnect();
    status.changed().await.unwrap();
    assert!(!sim.probe_connection().await.unwrap());
    assert!(matches!(
        sim.set_target_power(150).await,
        Err(BleError::Disconnected)
    ));
    assert_eq!(*status.borrow(), ConnectionStatus::Disconnected);
    sim.restore_connection();
    status.changed().await.unwrap();
    assert!(sim.probe_connection().await.unwrap());
    assert!(sim.set_target_power(150).await.is_ok());
    assert_eq!(*status.borrow(), ConnectionStatus::Connected);
}

#[tokio::test(start_paused = true)]
async fn refuse_next_op_fires_once() {
    let sim = SimTrainer::new();
    sim.refuse_next_op();
    assert!(matches!(
        sim.start_or_resume_training().await,
        Err(BleError::ControlRefused(_))
    ));
    assert!(sim.start_or_resume_training().await.is_ok());
}
