use super::*;
use crate::database::{provider_connections::ConnectedProvider, workout_definitions};
use tp_core::workout_spec::WorkoutSpec;

fn setup() -> Connection {
    let conn = crate::database::test_connection();
    for (id, provider, account) in [
        ("garmin-one", "garmin", "123"),
        ("intervals", "intervals_icu", "i123"),
    ] {
        provider_connections::connect(
            &conn,
            &ConnectedProvider {
                id,
                provider,
                external_account_id: account,
                display_name: None,
                time_zone: None,
                connected_at_unix_ms: 1,
            },
        )
        .unwrap();
    }
    conn
}

fn event(id: &str, date: &str) -> (WorkoutSchedule, Value) {
    (
        WorkoutSchedule {
            id: id.into(),
            workout_id: "41".into(),
            date_local: date.into(),
        },
        serde_json::from_str(include_str!(
            "../../../testdata/providers/garmin/scheduled-cycling.json"
        ))
        .unwrap(),
    )
}

fn sync(
    conn: &mut Connection,
    connection: &str,
    events: &[(WorkoutSchedule, Value)],
    time: i64,
) -> PlanSyncReport {
    persist_calendar_window(conn, connection, "2030-01-01", "2030-01-31", events, time).unwrap()
}

fn row(
    conn: &Connection,
    connection: &str,
    id: &str,
) -> scheduled_workouts::ScheduledWorkoutWithDefinitionRow {
    scheduled_workouts::get_by_provider_event(conn, connection, id)
        .unwrap()
        .unwrap()
}

#[test]
fn repeated_templates_keep_distinct_occurrences_and_updates_preserve_local_identity() {
    let mut conn = setup();
    let events = [event("51", "2030-01-02"), event("52", "2030-01-03")];
    assert_eq!(sync(&mut conn, "garmin-one", &events, 10).inserted, 2);
    let original = row(&conn, "garmin-one", "51");
    assert_ne!(
        original.schedule.id,
        row(&conn, "garmin-one", "52").schedule.id
    );
    assert_eq!(sync(&mut conn, "garmin-one", &events, 20).unchanged, 2);
    let mut moved = events;
    moved[0].0.date_local = "2030-01-04".into();
    moved[0].1["workoutName"] = serde_json::json!("Updated by Garmin");
    assert_eq!(sync(&mut conn, "garmin-one", &moved, 30).updated, 1);
    let updated = row(&conn, "garmin-one", "51");
    assert_eq!(updated.schedule.id, original.schedule.id);
    assert_eq!(updated.definition.id, original.definition.id);
    assert_eq!(updated.schedule.scheduled_date_local, "2030-01-04");
    assert_eq!(updated.schedule.scheduled_time_local, None);
    let cached = WorkoutSpec::from_json(&updated.definition.tpw_json).unwrap();
    assert_eq!(cached.title, "Updated by Garmin");
    assert_eq!(
        cached.compile().unwrap().target_power_w_at(60, 200, 1.0),
        Some(190)
    );
    assert!(workout_definitions::list_local(&conn).unwrap().is_empty());
}

#[test]
fn removals_are_window_and_account_scoped_and_reappearance_restores_identity() {
    let mut conn = setup();
    sync(&mut conn, "garmin-one", &[event("51", "2030-01-02")], 10);
    provider_connections::mark_disconnected(&conn, "garmin-one", 11).unwrap();
    provider_connections::connect(
        &conn,
        &ConnectedProvider {
            id: "garmin-two",
            provider: "garmin",
            external_account_id: "456",
            display_name: None,
            time_zone: None,
            connected_at_unix_ms: 12,
        },
    )
    .unwrap();
    sync(&mut conn, "garmin-two", &[event("51", "2030-01-02")], 13);
    provider_connections::mark_disconnected(&conn, "garmin-two", 14).unwrap();
    provider_connections::connect(
        &conn,
        &ConnectedProvider {
            id: "garmin-one",
            provider: "garmin",
            external_account_id: "123",
            display_name: None,
            time_zone: None,
            connected_at_unix_ms: 15,
        },
    )
    .unwrap();
    let outside = event("53", "2030-02-01");
    persist_calendar_window(
        &mut conn,
        "garmin-one",
        "2030-02-01",
        "2030-02-02",
        &[outside],
        10,
    )
    .unwrap();
    let original = row(&conn, "garmin-one", "51");
    assert_eq!(sync(&mut conn, "garmin-one", &[], 20).removed, 1);
    assert_eq!(
        row(&conn, "garmin-one", "51").schedule.removed_at_unix_ms,
        Some(20)
    );
    assert_eq!(
        row(&conn, "garmin-two", "51").schedule.removed_at_unix_ms,
        None
    );
    assert_eq!(
        row(&conn, "garmin-one", "53").schedule.removed_at_unix_ms,
        None
    );
    sync(&mut conn, "garmin-one", &[event("51", "2030-01-02")], 30);
    assert_eq!(
        row(&conn, "garmin-one", "51").schedule.id,
        original.schedule.id
    );
    assert_eq!(
        row(&conn, "garmin-one", "51").schedule.removed_at_unix_ms,
        None
    );
}

#[test]
fn unsupported_edits_keep_last_good_cache_and_do_not_block_valid_events() {
    let mut conn = setup();
    sync(&mut conn, "garmin-one", &[event("51", "2030-01-02")], 10);
    let original = row(&conn, "garmin-one", "51");
    let mut unsupported = event("51", "2030-01-02");
    unsupported.1["workoutSegments"][0]["workoutSteps"][0]["endCondition"]["conditionTypeKey"] =
        serde_json::json!("lap.button");
    let report = sync(
        &mut conn,
        "garmin-one",
        &[unsupported, event("52", "2030-01-03")],
        20,
    );
    assert_eq!(report.inserted, 1);
    assert_eq!(report.unsupported, 1);
    assert_eq!(report.issues.len(), 1);
    assert_eq!(row(&conn, "garmin-one", "51"), original);
    assert!(provider_connections::get(&conn, "garmin-one")
        .unwrap()
        .unwrap()
        .last_sync_error
        .is_some());
}

#[test]
fn inconsistent_calendar_never_mutates_cache_or_reconciles_missing_events() {
    let mut conn = setup();
    sync(&mut conn, "garmin-one", &[event("51", "2030-01-02")], 10);
    let original = row(&conn, "garmin-one", "51");
    let mut mismatch = event("52", "2030-01-03");
    mismatch.0.workout_id = "99".into();
    for events in [
        vec![event("52", "2030-01-03"), mismatch],
        vec![event("52", "2030-02-01")],
        vec![event("52", "2030-01-03"), event("52", "2030-01-04")],
    ] {
        assert!(persist_calendar_window(
            &mut conn,
            "garmin-one",
            "2030-01-01",
            "2030-01-31",
            &events,
            20
        )
        .is_err());
        assert_eq!(row(&conn, "garmin-one", "51"), original);
        assert!(
            scheduled_workouts::get_by_provider_event(&conn, "garmin-one", "52")
                .unwrap()
                .is_none()
        );
    }
    assert!(
        persist_calendar_window(&mut conn, "intervals", "2030-01-01", "2030-01-31", &[], 20)
            .is_err()
    );
}

#[test]
fn retirement_preserves_activity_snapshots_local_copies_and_upload_receipts() {
    let mut conn = setup();
    sync(&mut conn, "garmin-one", &[event("51", "2030-01-02")], 10);
    let original = row(&conn, "garmin-one", "51");
    workout_definitions::insert(
        &conn,
        &workout_definitions::NewWorkoutDefinition {
            id: "local-copy",
            tpw_json: &original.definition.tpw_json,
            created_at_ms: 10,
        },
    )
    .unwrap();
    conn.execute("INSERT INTO activities(id, workout_name, started_at_unix_ms, elapsed_s, timer_s, ftp_used_w, final_intensity_multiplier, fit_path, journal_path, completed_pct, scheduled_workout_id) VALUES ('ride','Synthetic Garmin cycling',10,30,30,200,1,'synthetic.fit','synthetic.jsonl',100,?1)", [&original.schedule.id]).unwrap();
    crate::database::activity_uploads::record(
        &conn,
        &crate::database::activity_uploads::ActivityUpload {
            connection_id: "garmin-one".into(),
            activity_id: "ride".into(),
            remote_activity_id: "901".into(),
            uploaded_at_unix_ms: 10,
        },
    )
    .unwrap();
    let tx = conn.transaction().unwrap();
    scheduled_workouts::remove_all_for_connection(&tx, "garmin-one", 20).unwrap();
    provider_connections::mark_disconnected(&tx, "garmin-one", 20).unwrap();
    tx.commit().unwrap();
    assert!(workout_definitions::get(&conn, "local-copy")
        .unwrap()
        .is_some());
    assert!(workout_definitions::get(&conn, &original.definition.id)
        .unwrap()
        .is_some());
    assert_eq!(
        crate::database::activity_uploads::list(&conn, "garmin-one")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT scheduled_workout_id FROM activities WHERE id='ride'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        original.schedule.id
    );
}
