use tp_integrations::intervals_icu::{
    IntervalsApiError, IntervalsCalendarEvent, IntervalsIcuClient,
};

#[test]
fn structured_workouts_are_accessible_without_a_trainerpro_model() {
    let event: IntervalsCalendarEvent = serde_json::from_str(include_str!(
        "../../../testdata/providers/intervals-icu/scheduled-virtual-ride.json"
    ))
    .unwrap();
    assert_eq!(event.id, 1_000_000);
    assert_eq!(event.start_date_local, "2030-01-01T00:00:00");
    assert_eq!(event.event_type, "VirtualRide");
    let doc = event.workout_doc.unwrap();
    assert_eq!(doc.duration, Some(1_740));
    assert_eq!(doc.steps.len(), 5);
    let ramp = &doc.steps[0];
    assert!(ramp.ramp);
    let power = ramp.power.as_ref().unwrap();
    assert_eq!(power.units, "%ftp");
    assert_eq!(power.start, Some(50.0));
    assert_eq!(power.end, Some(70.0));
    let cadence = ramp.cadence.as_ref().unwrap();
    assert_eq!(cadence.units, "rpm");
    assert_eq!(cadence.start, Some(85.0));
    assert_eq!(cadence.end, Some(95.0));
    let repeat = &doc.steps[1];
    assert_eq!(repeat.reps, Some(3));
    assert_eq!(repeat.steps.len(), 2);
    assert_eq!(repeat.steps[0].power.as_ref().unwrap().value, Some(105.0));
    assert!(doc.steps[3].freeride);
}

#[test]
fn unused_provider_metadata_does_not_change_calendar_decoding() {
    // uid/external_id were ignored in production before the extraction. Their
    // presence or shape must not start rejecting otherwise usable events.
    let event: IntervalsCalendarEvent = serde_json::from_value(serde_json::json!({
        "id": 7,
        "uid": {"uninterpreted": true},
        "external_id": 42,
        "start_date_local": "2030-01-01T00:00:00",
        "type": "Run",
        "category": "WORKOUT",
        "name": "Provider workout"
    }))
    .unwrap();
    assert_eq!(event.id, 7);
    assert_eq!(event.event_type, "Run");
    assert!(event.workout_doc.is_none());
}

#[tokio::test]
async fn blank_keys_and_invalid_ranges_fail_before_network_io() {
    let client = IntervalsIcuClient::new("ConsumerApp/1.0").unwrap();
    assert!(matches!(
        client.fetch_athlete(" ").await,
        Err(IntervalsApiError::MissingApiKey)
    ));
    assert!(matches!(
        client
            .fetch_workout_events(" ", "2030-01-01", "2030-01-07")
            .await,
        Err(IntervalsApiError::MissingApiKey)
    ));
    assert!(matches!(
        client
            .fetch_workout_events("synthetic-key", "2030-01-07", "2030-01-01")
            .await,
        Err(IntervalsApiError::InvalidDateRange { .. })
    ));
    assert!(matches!(
        client
            .fetch_workout_events("synthetic-key", "20300101", "2030-01-07")
            .await,
        Err(IntervalsApiError::InvalidDateRange { .. })
    ));
}
