use tp_ble::{
    ConnectionStatus, ControllerButton, ControllerInputEvent, ControllerProfile, SimController,
    StandaloneControllerConnection,
};

#[tokio::test]
async fn controller_fault_and_duplicate_input_use_existing_subscriptions() {
    let controller = SimController::new(ControllerProfile::ZwiftRide);
    let mut status = controller.subscribe_status();
    let mut input = controller.controller_input().events;

    controller.inject_button(ControllerButton::Y, true);
    controller.inject_button(ControllerButton::Y, true);
    assert_eq!(
        input.recv().await.unwrap(),
        ControllerInputEvent::Button {
            button: ControllerButton::Y,
            pressed: true,
        }
    );
    assert!(input.try_recv().is_err());

    controller.inject_disconnect();
    status.changed().await.unwrap();
    assert_eq!(*status.borrow(), ConnectionStatus::Disconnected);
    assert_eq!(input.recv().await.unwrap(), ControllerInputEvent::Cancel);
}
