//! Transactional inbound schedule sync for the concrete Intervals.icu client.

#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the sync boundary lands before the connection command that invokes it"
    )
)]

use std::collections::HashSet;

use rusqlite::Connection;

use crate::database::{provider_connections, scheduled_workouts};
use crate::intervals_icu::{
    is_iso_date, IntervalsCalendarEvent, IntervalsWorkoutError, PROVIDER_ID,
};

const ALL_DAY_START_TIME_LOCAL: &str = "00:00:00";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IntervalsSyncIssue {
    pub external_event_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IntervalsSyncSummary {
    pub inserted: u32,
    pub updated: u32,
    pub unchanged: u32,
    pub removed: u32,
    pub unsupported: u32,
    pub issues: Vec<IntervalsSyncIssue>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum IntervalsSyncError {
    #[error("provider connection {connection_id} was not found")]
    ConnectionNotFound { connection_id: String },
    #[error("provider connection {connection_id} belongs to {provider:?}, not Intervals.icu")]
    WrongProvider {
        connection_id: String,
        provider: String,
    },
    #[error("invalid completed sync window {oldest_date_local:?} through {newest_date_local:?}")]
    InvalidDateRange {
        oldest_date_local: String,
        newest_date_local: String,
    },
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
}

struct PreparedSchedule {
    external_event_id: String,
    external_event_id_i64: i64,
    external_revision: Option<String>,
    tpw_json: String,
    scheduled_date_local: String,
    scheduled_time_local: Option<String>,
    scheduled_time_zone: Option<String>,
}

/// Persist one complete calendar response. Provider mapping failures are
/// isolated to their event and remain observable in the summary; database
/// changes and deletion reconciliation commit atomically.
pub(crate) fn persist_calendar_window(
    conn: &mut Connection,
    provider_connection_id: &str,
    oldest_date_local: &str,
    newest_date_local: &str,
    events: &[IntervalsCalendarEvent],
    synced_at_unix_ms: i64,
) -> Result<IntervalsSyncSummary, IntervalsSyncError> {
    if !is_iso_date(oldest_date_local)
        || !is_iso_date(newest_date_local)
        || oldest_date_local > newest_date_local
    {
        return Err(IntervalsSyncError::InvalidDateRange {
            oldest_date_local: oldest_date_local.to_string(),
            newest_date_local: newest_date_local.to_string(),
        });
    }

    let connection = provider_connections::get(conn, provider_connection_id)?.ok_or_else(|| {
        IntervalsSyncError::ConnectionNotFound {
            connection_id: provider_connection_id.to_string(),
        }
    })?;
    if connection.provider != PROVIDER_ID {
        return Err(IntervalsSyncError::WrongProvider {
            connection_id: provider_connection_id.to_string(),
            provider: connection.provider,
        });
    }

    let mut present_external_event_ids = HashSet::new();
    let mut prepared = Vec::new();
    let mut summary = IntervalsSyncSummary {
        inserted: 0,
        updated: 0,
        unchanged: 0,
        removed: 0,
        unsupported: 0,
        issues: Vec::new(),
    };

    for event in events {
        let external_event_id = event.id.to_string();
        let definition = match event.to_workout_definition() {
            Ok(definition) => definition,
            Err(
                IntervalsWorkoutError::UnsupportedSport { .. }
                | IntervalsWorkoutError::NotWorkout { .. },
            ) => {
                summary.unsupported += 1;
                continue;
            }
            Err(error) => {
                present_external_event_ids.insert(external_event_id.clone());
                summary.issues.push(IntervalsSyncIssue {
                    external_event_id,
                    message: error.to_string(),
                });
                continue;
            }
        };
        let (scheduled_date_local, scheduled_time_local, scheduled_time_zone) =
            match schedule_placement(&event.start_date_local, &connection.time_zone) {
                Some(placement) => placement,
                None => {
                    present_external_event_ids.insert(external_event_id.clone());
                    summary.issues.push(IntervalsSyncIssue {
                        external_event_id,
                        message: format!(
                            "invalid Intervals.icu start_date_local {:?}",
                            event.start_date_local
                        ),
                    });
                    continue;
                }
            };
        let tpw_json = match definition.to_json_pretty() {
            Ok(tpw_json) => tpw_json,
            Err(error) => {
                present_external_event_ids.insert(external_event_id.clone());
                summary.issues.push(IntervalsSyncIssue {
                    external_event_id,
                    message: error.to_string(),
                });
                continue;
            }
        };
        present_external_event_ids.insert(external_event_id.clone());
        prepared.push(PreparedSchedule {
            external_event_id,
            external_event_id_i64: event.id,
            external_revision: event.updated.clone(),
            tpw_json,
            scheduled_date_local,
            scheduled_time_local,
            scheduled_time_zone,
        });
    }

    let transaction = conn.transaction()?;
    for schedule in &prepared {
        let schedule_id = uuid::Uuid::new_v4().to_string();
        let definition_id = uuid::Uuid::new_v4().to_string();
        let change = scheduled_workouts::upsert_synced(
            &transaction,
            &scheduled_workouts::SyncedScheduledWorkout {
                schedule_id_for_insert: &schedule_id,
                definition_id_for_insert: &definition_id,
                provider_connection_id,
                external_event_id: &schedule.external_event_id,
                external_event_id_i64: Some(schedule.external_event_id_i64),
                definition_origin: PROVIDER_ID,
                external_revision: schedule.external_revision.as_deref(),
                tpw_json: &schedule.tpw_json,
                scheduled_date_local: &schedule.scheduled_date_local,
                scheduled_time_local: schedule.scheduled_time_local.as_deref(),
                scheduled_time_zone: schedule.scheduled_time_zone.as_deref(),
                synced_at_unix_ms,
            },
        )?;
        match change {
            scheduled_workouts::SyncedScheduleChange::Inserted => summary.inserted += 1,
            scheduled_workouts::SyncedScheduleChange::Updated => summary.updated += 1,
            scheduled_workouts::SyncedScheduleChange::Unchanged => summary.unchanged += 1,
        }
    }
    summary.removed = scheduled_workouts::remove_missing_in_window(
        &transaction,
        provider_connection_id,
        oldest_date_local,
        newest_date_local,
        &present_external_event_ids,
        synced_at_unix_ms,
    )?;
    if summary.issues.is_empty() {
        provider_connections::record_sync_success(
            &transaction,
            provider_connection_id,
            synced_at_unix_ms,
        )?;
    } else {
        let message = format!(
            "{} Intervals.icu workout(s) could not be synchronized",
            summary.issues.len()
        );
        provider_connections::record_sync_failure(
            &transaction,
            provider_connection_id,
            &message,
            synced_at_unix_ms,
        )?;
    }
    transaction.commit()?;
    Ok(summary)
}

fn schedule_placement(
    start_date_local: &str,
    time_zone: &str,
) -> Option<(String, Option<String>, Option<String>)> {
    let (date, time) = start_date_local.split_once('T')?;
    if !is_iso_date(date) || !is_local_time(time) {
        return None;
    }
    if time == ALL_DAY_START_TIME_LOCAL {
        Some((date.to_string(), None, None))
    } else {
        Some((
            date.to_string(),
            Some(time.to_string()),
            Some(time_zone.to_string()),
        ))
    }
}

fn is_local_time(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 8 || bytes[2] != b':' || bytes[5] != b':' {
        return false;
    }
    if !bytes
        .iter()
        .enumerate()
        .all(|(index, byte)| matches!(index, 2 | 5) || byte.is_ascii_digit())
    {
        return false;
    }
    let hour = value[0..2].parse::<u8>().ok();
    let minute = value[3..5].parse::<u8>().ok();
    let second = value[6..8].parse::<u8>().ok();
    matches!(
        (hour, minute, second),
        (Some(0..=23), Some(0..=59), Some(0..=59))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::provider_connections::ConnectedProvider;
    use tp_core::workout_definition::WorkoutDefinition;

    fn connection(conn: &Connection, id: &str, external_account_id: &str) {
        provider_connections::connect(
            conn,
            &ConnectedProvider {
                id,
                provider: PROVIDER_ID,
                external_account_id,
                display_name: Some("Fixture Athlete"),
                time_zone: "Europe/Zurich",
                connected_at_unix_ms: 1,
            },
        )
        .unwrap();
    }

    fn fixture() -> IntervalsCalendarEvent {
        serde_json::from_str(include_str!(
            "../../testdata/providers/intervals-icu/scheduled-virtual-ride.json"
        ))
        .unwrap()
    }

    fn synced_row(
        conn: &Connection,
        connection_id: &str,
    ) -> scheduled_workouts::ScheduledWorkoutWithDefinitionRow {
        scheduled_workouts::get_by_provider_event(conn, connection_id, "1000000")
            .unwrap()
            .unwrap()
    }

    #[test]
    fn repeated_sync_preserves_ids_and_does_not_rewrite_unchanged_rows() {
        let mut conn = crate::database::test_connection();
        connection(&conn, "connection", "i123");
        let event = fixture();

        let first = persist_calendar_window(
            &mut conn,
            "connection",
            "2030-01-01",
            "2030-01-07",
            &[event],
            10,
        )
        .unwrap();
        assert_eq!(first.inserted, 1);
        assert_eq!(first.issues, Vec::new());
        let original = synced_row(&conn, "connection");
        assert_eq!(original.schedule.scheduled_date_local, "2030-01-01");
        assert_eq!(original.schedule.scheduled_time_local, None);
        assert_eq!(original.schedule.scheduled_time_zone, None);
        assert_eq!(original.schedule.last_synced_at_unix_ms, Some(10));

        let second = persist_calendar_window(
            &mut conn,
            "connection",
            "2030-01-01",
            "2030-01-07",
            &[fixture()],
            20,
        )
        .unwrap();
        assert_eq!(second.unchanged, 1);
        let repeated = synced_row(&conn, "connection");
        assert_eq!(repeated.schedule.id, original.schedule.id);
        assert_eq!(repeated.definition.id, original.definition.id);
        assert_eq!(repeated.schedule.last_synced_at_unix_ms, Some(10));
        assert_eq!(
            provider_connections::get(&conn, "connection")
                .unwrap()
                .unwrap()
                .last_sync_succeeded_at_unix_ms,
            Some(20)
        );
    }

    #[test]
    fn remote_edit_updates_definition_and_timed_placement_in_place() {
        let mut conn = crate::database::test_connection();
        connection(&conn, "connection", "i123");
        persist_calendar_window(
            &mut conn,
            "connection",
            "2030-01-01",
            "2030-01-07",
            &[fixture()],
            10,
        )
        .unwrap();
        let original = synced_row(&conn, "connection");

        let mut edited = fixture();
        edited.name = "Edited by provider".into();
        edited.start_date_local = "2030-01-02T07:30:00".into();
        edited.updated = Some("2030-01-01T12:00:00.000+0000".into());
        let summary = persist_calendar_window(
            &mut conn,
            "connection",
            "2030-01-01",
            "2030-01-07",
            &[edited],
            20,
        )
        .unwrap();

        assert_eq!(summary.updated, 1);
        let updated = synced_row(&conn, "connection");
        assert_eq!(updated.schedule.id, original.schedule.id);
        assert_eq!(updated.definition.id, original.definition.id);
        assert_eq!(updated.schedule.scheduled_date_local, "2030-01-02");
        assert_eq!(
            updated.schedule.scheduled_time_local.as_deref(),
            Some("07:30:00")
        );
        assert_eq!(
            updated.schedule.scheduled_time_zone.as_deref(),
            Some("Europe/Zurich")
        );
        assert_eq!(updated.schedule.last_synced_at_unix_ms, Some(20));
        assert_eq!(
            WorkoutDefinition::from_json(&updated.definition.tpw_json)
                .unwrap()
                .title,
            "Edited by provider"
        );
    }

    #[test]
    fn missing_event_is_soft_removed_and_can_reappear_with_same_ids() {
        let mut conn = crate::database::test_connection();
        connection(&conn, "connection", "i123");
        persist_calendar_window(
            &mut conn,
            "connection",
            "2030-01-01",
            "2030-01-07",
            &[fixture()],
            10,
        )
        .unwrap();
        let original = synced_row(&conn, "connection");

        let removed =
            persist_calendar_window(&mut conn, "connection", "2030-01-01", "2030-01-07", &[], 20)
                .unwrap();
        assert_eq!(removed.removed, 1);
        assert_eq!(
            synced_row(&conn, "connection").schedule.removed_at_unix_ms,
            Some(20)
        );
        assert!(crate::database::workout_definitions::list(&conn)
            .unwrap()
            .is_empty());

        let restored = persist_calendar_window(
            &mut conn,
            "connection",
            "2030-01-01",
            "2030-01-07",
            &[fixture()],
            30,
        )
        .unwrap();
        assert_eq!(restored.updated, 1);
        let current = synced_row(&conn, "connection");
        assert_eq!(current.schedule.id, original.schedule.id);
        assert_eq!(current.definition.id, original.definition.id);
        assert_eq!(current.schedule.removed_at_unix_ms, None);
        assert_eq!(
            crate::database::workout_definitions::list(&conn)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn malformed_seen_event_preserves_last_good_copy_and_records_failure() {
        let mut conn = crate::database::test_connection();
        connection(&conn, "connection", "i123");
        persist_calendar_window(
            &mut conn,
            "connection",
            "2030-01-01",
            "2030-01-07",
            &[fixture()],
            10,
        )
        .unwrap();

        let mut malformed = fixture();
        malformed.name = " ".into();
        let summary = persist_calendar_window(
            &mut conn,
            "connection",
            "2030-01-01",
            "2030-01-07",
            &[malformed],
            20,
        )
        .unwrap();

        assert_eq!(summary.issues.len(), 1);
        assert_eq!(summary.removed, 0);
        assert_eq!(
            synced_row(&conn, "connection").schedule.removed_at_unix_ms,
            None
        );
        let connection = provider_connections::get(&conn, "connection")
            .unwrap()
            .unwrap();
        assert_eq!(connection.last_sync_succeeded_at_unix_ms, Some(10));
        assert!(connection.last_sync_error.is_some());
    }

    #[test]
    fn event_that_changes_to_an_unsupported_sport_is_removed() {
        let mut conn = crate::database::test_connection();
        connection(&conn, "connection", "i123");
        persist_calendar_window(
            &mut conn,
            "connection",
            "2030-01-01",
            "2030-01-07",
            &[fixture()],
            10,
        )
        .unwrap();

        let mut run = fixture();
        run.event_type = "Run".into();
        let summary = persist_calendar_window(
            &mut conn,
            "connection",
            "2030-01-01",
            "2030-01-07",
            &[run],
            20,
        )
        .unwrap();

        assert_eq!(summary.unsupported, 1);
        assert_eq!(summary.removed, 1);
        assert_eq!(
            synced_row(&conn, "connection").schedule.removed_at_unix_ms,
            Some(20)
        );
    }

    #[test]
    fn provider_identity_scopes_equal_external_event_ids() {
        let mut conn = crate::database::test_connection();
        connection(&conn, "first", "i123");
        connection(&conn, "second", "i456");

        for connection_id in ["first", "second"] {
            let summary = persist_calendar_window(
                &mut conn,
                connection_id,
                "2030-01-01",
                "2030-01-07",
                &[fixture()],
                10,
            )
            .unwrap();
            assert_eq!(summary.inserted, 1);
        }
        assert_ne!(
            synced_row(&conn, "first").schedule.id,
            synced_row(&conn, "second").schedule.id
        );
    }
}
