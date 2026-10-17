use serde_json::json;
use tp_integrations::garmin::{GarminClient, GarminError, Tokens};

#[test]
fn persisted_tokens_keep_the_existing_json_contract() {
    // The backend vault stores this shape directly, including older credentials
    // whose expiry is absent. No application types are needed to restore it.
    let saved = json!({
        "access_token": "synthetic-access",
        "refresh_token": "synthetic-refresh",
        "client_id": "synthetic-client",
        "expires_at_unix_s": 0
    });
    let tokens: Tokens = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(serde_json::to_value(&tokens).unwrap(), saved);
    assert!(tokens.needs_refresh());

    let mut legacy = saved;
    legacy.as_object_mut().unwrap().remove("expires_at_unix_s");
    let tokens: Tokens = serde_json::from_value(legacy.clone()).unwrap();
    assert!(tokens.needs_refresh());
    legacy["expires_at_unix_s"] = serde_json::Value::Null;
    assert_eq!(serde_json::to_value(&tokens).unwrap(), legacy);
}

#[tokio::test]
async fn invalid_fit_is_rejected_before_network_io() {
    let tokens: Tokens = serde_json::from_value(json!({
        "access_token": "synthetic-access",
        "refresh_token": "synthetic-refresh",
        "client_id": "synthetic-client"
    }))
    .unwrap();
    let invalid_fit = b"not a FIT file";
    assert_eq!(
        GarminClient::new()
            .unwrap()
            .upload(&tokens, invalid_fit.to_vec(), "other-app.fit")
            .await,
        Err(GarminError::InvalidFit)
    );
}

#[test]
fn browser_callback_recognition_requires_the_exact_credential_destination() {
    assert!(GarminClient::is_browser_sign_in_callback(
        "https://sso.garmin.com/sso/embed?ticket=ST-synthetic"
    ));
    for input in [
        "ST-synthetic",
        "https://sso.garmin.com/sso/embed",
        "https://sso.garmin.com.attacker.test/sso/embed?ticket=ST-synthetic",
        "https://sso.garmin.com/sso/embed?ticket=ST-one&ticket=ST-two",
        "http://sso.garmin.com/sso/embed?ticket=ST-synthetic",
        "https://sso.garmin.com/sso/embed?ticket=ST-synthetic#fragment",
    ] {
        assert!(!GarminClient::is_browser_sign_in_callback(input));
    }
}

#[test]
fn schedule_occurrence_identity_is_independent_of_workout_identity() {
    use tp_integrations::garmin::workouts::WorkoutSchedule;
    let first = WorkoutSchedule::from_payload(
        &json!({"workoutScheduleId":51,"workout":{"workoutId":41},"calendarDate":"2026-10-25"}),
    )
    .unwrap();
    let second = WorkoutSchedule::from_payload(
        &json!({"workoutScheduleId":"52","workout":{"workoutId":"41"},"calendarDate":"2026-10-26"}),
    )
    .unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.workout_id, second.workout_id);
    assert!(WorkoutSchedule::from_payload(
        &json!({"workoutScheduleId":51,"workout":{"workoutId":41},"calendarDate":"2026-02-30"})
    )
    .is_err());
}
