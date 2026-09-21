//! Scheduled-workout persistence.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, Transaction};

use super::workout_definitions::{self, ProviderWorkoutDefinition, WorkoutDefinitionRow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledWorkoutRow {
    pub id: String,
    pub workout_definition_id: String,
    pub scheduled_date_local: String,
    pub scheduled_time_local: Option<String>,
    pub scheduled_time_zone: Option<String>,
    pub removed_at_unix_ms: Option<i64>,
    pub provider_connection_id: Option<String>,
    pub external_event_id: Option<String>,
    pub external_revision: Option<String>,
    pub last_synced_at_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledWorkoutWithDefinitionRow {
    pub schedule: ScheduledWorkoutRow,
    pub definition: WorkoutDefinitionRow,
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledWorkoutRow> {
    Ok(ScheduledWorkoutRow {
        id: row.get(0)?,
        workout_definition_id: row.get(1)?,
        scheduled_date_local: row.get(2)?,
        scheduled_time_local: row.get(3)?,
        scheduled_time_zone: row.get(4)?,
        removed_at_unix_ms: row.get(5)?,
        provider_connection_id: row.get(6)?,
        external_event_id: row.get(7)?,
        external_revision: row.get(8)?,
        last_synced_at_unix_ms: row.get(9)?,
    })
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<ScheduledWorkoutRow>> {
    conn.query_row(
        "SELECT id, workout_definition_id, scheduled_date_local,
                scheduled_time_local, scheduled_time_zone, removed_at_unix_ms,
                provider_connection_id, external_event_id, external_revision,
                last_synced_at_unix_ms
         FROM scheduled_workouts WHERE id = ?1",
        [id],
        read_row,
    )
    .optional()
}

/// The active, unfulfilled scheduled workouts that feed Next Up. Completion is
/// represented by an Activity link instead of mirrored schedule state.
pub fn list_for_next_up(
    conn: &Connection,
    oldest_date_local: &str,
) -> rusqlite::Result<Vec<ScheduledWorkoutWithDefinitionRow>> {
    let mut stmt = conn.prepare(
        "SELECT sw.id, sw.workout_definition_id, sw.scheduled_date_local,
                sw.scheduled_time_local, sw.scheduled_time_zone,
                sw.removed_at_unix_ms, sw.provider_connection_id,
                sw.external_event_id, sw.external_revision,
                sw.last_synced_at_unix_ms, wd.id, wd.tpw_json, wd.origin
         FROM scheduled_workouts sw
         JOIN workout_definitions wd ON wd.id = sw.workout_definition_id
         WHERE sw.removed_at_unix_ms IS NULL
           AND sw.scheduled_date_local >= ?1
           AND NOT EXISTS (
             SELECT 1 FROM activities a WHERE a.scheduled_workout_id = sw.id
           )
         ORDER BY sw.scheduled_date_local,
                  COALESCE(sw.scheduled_time_local, '00:00:00'), sw.id",
    )?;
    let rows = stmt.query_map([oldest_date_local], |row| {
        Ok(ScheduledWorkoutWithDefinitionRow {
            schedule: ScheduledWorkoutRow {
                id: row.get(0)?,
                workout_definition_id: row.get(1)?,
                scheduled_date_local: row.get(2)?,
                scheduled_time_local: row.get(3)?,
                scheduled_time_zone: row.get(4)?,
                removed_at_unix_ms: row.get(5)?,
                provider_connection_id: row.get(6)?,
                external_event_id: row.get(7)?,
                external_revision: row.get(8)?,
                last_synced_at_unix_ms: row.get(9)?,
            },
            definition: WorkoutDefinitionRow {
                id: row.get(10)?,
                tpw_json: row.get(11)?,
                origin: row.get(12)?,
            },
        })
    })?;
    rows.collect()
}

pub struct SyncedScheduledWorkout<'a> {
    pub schedule_id_for_insert: &'a str,
    pub definition_id_for_insert: &'a str,
    pub provider_connection_id: &'a str,
    pub external_event_id: &'a str,
    pub external_event_id_i64: Option<i64>,
    pub definition_origin: &'a str,
    pub external_revision: Option<&'a str>,
    pub tpw_json: &'a str,
    pub scheduled_date_local: &'a str,
    pub scheduled_time_local: Option<&'a str>,
    pub scheduled_time_zone: Option<&'a str>,
    pub synced_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncedScheduleChange {
    Inserted,
    Updated,
    Unchanged,
}

pub fn get_by_provider_event(
    conn: &Connection,
    provider_connection_id: &str,
    external_event_id: &str,
) -> rusqlite::Result<Option<ScheduledWorkoutWithDefinitionRow>> {
    conn.query_row(
        "SELECT sw.id, sw.workout_definition_id, sw.scheduled_date_local,
                sw.scheduled_time_local, sw.scheduled_time_zone,
                sw.removed_at_unix_ms, sw.provider_connection_id,
                sw.external_event_id, sw.external_revision,
                sw.last_synced_at_unix_ms, wd.id, wd.tpw_json, wd.origin
         FROM scheduled_workouts sw
         JOIN workout_definitions wd ON wd.id = sw.workout_definition_id
         WHERE sw.provider_connection_id = ?1 AND sw.external_event_id = ?2",
        rusqlite::params![provider_connection_id, external_event_id],
        |row| {
            Ok(ScheduledWorkoutWithDefinitionRow {
                schedule: read_row(row)?,
                definition: WorkoutDefinitionRow {
                    id: row.get(10)?,
                    tpw_json: row.get(11)?,
                    origin: row.get(12)?,
                },
            })
        },
    )
    .optional()
}

/// Insert or refresh one provider-owned schedule inside the caller's sync
/// transaction. Stable provider identity preserves both local IDs.
pub fn upsert_synced(
    transaction: &Transaction<'_>,
    synced: &SyncedScheduledWorkout<'_>,
) -> rusqlite::Result<SyncedScheduleChange> {
    let existing = get_by_provider_event(
        transaction,
        synced.provider_connection_id,
        synced.external_event_id,
    )?;
    if let Some(existing) = existing {
        let unchanged = existing.definition.tpw_json == synced.tpw_json
            && existing.schedule.scheduled_date_local == synced.scheduled_date_local
            && existing.schedule.scheduled_time_local.as_deref() == synced.scheduled_time_local
            && existing.schedule.scheduled_time_zone.as_deref() == synced.scheduled_time_zone
            && existing.schedule.external_revision.as_deref() == synced.external_revision
            && existing.schedule.removed_at_unix_ms.is_none();
        if unchanged {
            return Ok(SyncedScheduleChange::Unchanged);
        }

        workout_definitions::update_provider_copy(
            transaction,
            &ProviderWorkoutDefinition {
                id: &existing.definition.id,
                tpw_json: synced.tpw_json,
                origin: synced.definition_origin,
                origin_id: synced.external_event_id_i64,
                origin_ref: synced.external_event_id,
                synced_at_unix_ms: synced.synced_at_unix_ms,
            },
        )?;
        transaction.execute(
            "UPDATE scheduled_workouts
             SET scheduled_date_local = ?1, scheduled_time_local = ?2,
                 scheduled_time_zone = ?3, removed_at_unix_ms = NULL,
                 external_revision = ?4, last_synced_at_unix_ms = ?5,
                 updated_at_unix_ms = ?5
             WHERE id = ?6",
            rusqlite::params![
                synced.scheduled_date_local,
                synced.scheduled_time_local,
                synced.scheduled_time_zone,
                synced.external_revision,
                synced.synced_at_unix_ms,
                existing.schedule.id,
            ],
        )?;
        return Ok(SyncedScheduleChange::Updated);
    }

    workout_definitions::insert_provider_copy(
        transaction,
        &ProviderWorkoutDefinition {
            id: synced.definition_id_for_insert,
            tpw_json: synced.tpw_json,
            origin: synced.definition_origin,
            origin_id: synced.external_event_id_i64,
            origin_ref: synced.external_event_id,
            synced_at_unix_ms: synced.synced_at_unix_ms,
        },
    )?;
    transaction.execute(
        "INSERT INTO scheduled_workouts (
           id, workout_definition_id, scheduled_date_local,
           scheduled_time_local, scheduled_time_zone,
           created_at_unix_ms, updated_at_unix_ms, provider_connection_id,
           external_event_id, external_revision, last_synced_at_unix_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?6,?7,?8,?9,?6)",
        rusqlite::params![
            synced.schedule_id_for_insert,
            synced.definition_id_for_insert,
            synced.scheduled_date_local,
            synced.scheduled_time_local,
            synced.scheduled_time_zone,
            synced.synced_at_unix_ms,
            synced.provider_connection_id,
            synced.external_event_id,
            synced.external_revision,
        ],
    )?;
    Ok(SyncedScheduleChange::Inserted)
}

/// Soft-remove active provider schedules that disappeared from one complete,
/// bounded response. Rows outside the fetched date window are untouched.
pub fn remove_missing_in_window(
    transaction: &Transaction<'_>,
    provider_connection_id: &str,
    oldest_date_local: &str,
    newest_date_local: &str,
    present_external_event_ids: &HashSet<String>,
    removed_at_unix_ms: i64,
) -> rusqlite::Result<u32> {
    let mut statement = transaction.prepare(
        "SELECT id, external_event_id
         FROM scheduled_workouts
         WHERE provider_connection_id = ?1
           AND scheduled_date_local BETWEEN ?2 AND ?3
           AND removed_at_unix_ms IS NULL",
    )?;
    let rows = statement.query_map(
        rusqlite::params![provider_connection_id, oldest_date_local, newest_date_local,],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;
    let missing_ids = rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter_map(|(id, external_event_id)| {
            (!present_external_event_ids.contains(&external_event_id)).then_some(id)
        })
        .collect::<Vec<_>>();
    drop(statement);

    for id in &missing_ids {
        transaction.execute(
            "UPDATE workout_definitions
             SET retired_at_unix_ms = ?1, updated_at = ?1
             WHERE id = (
               SELECT workout_definition_id FROM scheduled_workouts WHERE id = ?2
             )",
            rusqlite::params![removed_at_unix_ms, id],
        )?;
        transaction.execute(
            "UPDATE scheduled_workouts
             SET removed_at_unix_ms = ?1, updated_at_unix_ms = ?1
             WHERE id = ?2",
            rusqlite::params![removed_at_unix_ms, id],
        )?;
    }
    Ok(missing_ids.len() as u32)
}

/// Retire every active schedule owned by a disconnected provider account.
/// Activity references and the last synchronized definitions remain intact.
pub fn remove_all_for_connection(
    transaction: &Transaction<'_>,
    provider_connection_id: &str,
    removed_at_unix_ms: i64,
) -> rusqlite::Result<u32> {
    let mut statement = transaction.prepare(
        "SELECT id FROM scheduled_workouts
         WHERE provider_connection_id = ?1 AND removed_at_unix_ms IS NULL",
    )?;
    let ids = statement
        .query_map([provider_connection_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);

    for id in &ids {
        transaction.execute(
            "UPDATE workout_definitions
             SET retired_at_unix_ms = ?1, updated_at = ?1
             WHERE id = (
               SELECT workout_definition_id FROM scheduled_workouts WHERE id = ?2
             )",
            rusqlite::params![removed_at_unix_ms, id],
        )?;
        transaction.execute(
            "UPDATE scheduled_workouts
             SET removed_at_unix_ms = ?1, updated_at_unix_ms = ?1
             WHERE id = ?2",
            rusqlite::params![removed_at_unix_ms, id],
        )?;
    }
    Ok(ids.len() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::provider_connections::{self, ConnectedProvider};
    use crate::database::workout_definitions::{
        self, NewWorkoutDefinition, ProviderWorkoutDefinition,
    };

    const TPW_JSON: &str = r#"{"format":"TPW","version":1,"title":"Endurance","prescription":{"sport":"cycling","steps":[{"type":"steady","duration_seconds":3600,"power":{"type":"percent_ftp","percent":70}}]}}"#;
    const EARLIEST_TEST_DATE: &str = "0001-01-01";

    fn insert_definition(conn: &Connection, id: &str) {
        workout_definitions::insert(
            conn,
            &NewWorkoutDefinition {
                id,
                tpw_json: &TPW_JSON.replace("Endurance", id),
                created_at_ms: 1,
            },
        )
        .unwrap();
    }

    fn insert_schedule<'a>(
        conn: &Connection,
        id: &'a str,
        workout_definition_id: &'a str,
        date: &'a str,
    ) {
        conn.execute(
            "INSERT INTO scheduled_workouts (
               id, workout_definition_id, scheduled_date_local,
               created_at_unix_ms, updated_at_unix_ms)
             VALUES (?1,?2,?3,10,10)",
            rusqlite::params![id, workout_definition_id, date],
        )
        .unwrap();
    }

    #[test]
    fn insert_get_order_remove_and_fulfill() {
        let conn = super::super::test_connection();
        insert_definition(&conn, "definition-a");
        insert_definition(&conn, "definition-b");
        insert_schedule(&conn, "later", "definition-b", "2026-09-16");
        insert_schedule(&conn, "earlier", "definition-a", "2026-09-15");

        assert_eq!(
            list_for_next_up(&conn, EARLIEST_TEST_DATE)
                .unwrap()
                .iter()
                .map(|row| row.schedule.id.as_str())
                .collect::<Vec<_>>(),
            vec!["earlier", "later"]
        );
        assert_eq!(
            get(&conn, "earlier")
                .unwrap()
                .unwrap()
                .workout_definition_id,
            "definition-a"
        );

        conn.execute(
            "UPDATE scheduled_workouts
             SET removed_at_unix_ms = 20, updated_at_unix_ms = 20
             WHERE id = 'later'",
            [],
        )
        .unwrap();
        assert_eq!(
            get(&conn, "later").unwrap().unwrap().removed_at_unix_ms,
            Some(20)
        );

        conn.execute(
            "INSERT INTO activities (
               id, scheduled_workout_id, workout_name, started_at_unix_ms,
               elapsed_s, timer_s, ftp_used_w, final_intensity_multiplier,
               fit_path, journal_path, completed_pct)
             VALUES ('activity-a', 'earlier', 'Endurance', 30, 60, 60, 250,
                     1.0, '/tmp/activity.fit', '/tmp/session.jsonl', 100.0)",
            [],
        )
        .unwrap();
        assert!(list_for_next_up(&conn, EARLIEST_TEST_DATE)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn next_up_includes_the_oldest_allowed_date_and_excludes_older_schedules() {
        let conn = super::super::test_connection();
        for (id, date) in [
            ("too-old", "2026-09-13"),
            ("oldest-allowed", "2026-09-14"),
            ("today", "2026-09-21"),
            ("future", "2026-09-22"),
        ] {
            insert_definition(&conn, id);
            insert_schedule(&conn, id, id, date);
        }

        assert_eq!(
            list_for_next_up(&conn, "2026-09-14")
                .unwrap()
                .iter()
                .map(|row| row.schedule.id.as_str())
                .collect::<Vec<_>>(),
            vec!["oldest-allowed", "today", "future"]
        );
        assert!(get(&conn, "too-old").unwrap().is_some());
    }

    #[test]
    fn timed_schedule_requires_a_time_zone() {
        let conn = super::super::test_connection();
        insert_definition(&conn, "definition");
        let result = conn.execute(
            "INSERT INTO scheduled_workouts (
               id, workout_definition_id, scheduled_date_local,
               scheduled_time_local, created_at_unix_ms, updated_at_unix_ms)
             VALUES ('scheduled','definition','2026-09-15','07:00:00',10,10)",
            [],
        );
        assert!(result.is_err());
    }

    #[test]
    fn provider_connection_and_external_event_identity_are_paired() {
        let conn = super::super::test_connection();
        insert_definition(&conn, "definition");
        provider_connections::connect(
            &conn,
            &ConnectedProvider {
                id: "connection",
                provider: "intervals_icu",
                external_account_id: "i123",
                display_name: None,
                time_zone: "Europe/Zurich",
                connected_at_unix_ms: 1,
            },
        )
        .unwrap();

        let missing_event = conn.execute(
            "INSERT INTO scheduled_workouts (
               id, workout_definition_id, scheduled_date_local,
               created_at_unix_ms, updated_at_unix_ms,
               provider_connection_id)
             VALUES ('missing-event','definition','2030-01-01',10,10,'connection')",
            [],
        );
        assert!(missing_event.is_err());

        let missing_connection = conn.execute(
            "INSERT INTO scheduled_workouts (
               id, workout_definition_id, scheduled_date_local,
               created_at_unix_ms, updated_at_unix_ms, external_event_id)
             VALUES ('missing-connection','definition','2030-01-01',10,10,'42')",
            [],
        );
        assert!(missing_connection.is_err());
    }

    #[test]
    fn disconnect_removes_only_the_connections_active_schedules() {
        let mut conn = super::super::test_connection();
        insert_definition(&conn, "local-definition");
        insert_schedule(&conn, "local-schedule", "local-definition", "2030-01-01");
        provider_connections::connect(
            &conn,
            &ConnectedProvider {
                id: "connection",
                provider: "intervals_icu",
                external_account_id: "i123",
                display_name: None,
                time_zone: "Europe/Zurich",
                connected_at_unix_ms: 1,
            },
        )
        .unwrap();
        workout_definitions::insert_provider_copy(
            &conn,
            &ProviderWorkoutDefinition {
                id: "provider-definition",
                tpw_json: TPW_JSON,
                origin: "intervals_icu",
                origin_id: Some(42),
                origin_ref: "42",
                synced_at_unix_ms: 10,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scheduled_workouts (
               id, workout_definition_id, scheduled_date_local,
               created_at_unix_ms, updated_at_unix_ms,
               provider_connection_id, external_event_id)
             VALUES ('provider-schedule','provider-definition','2030-01-02',
                     10,10,'connection','42')",
            [],
        )
        .unwrap();

        let transaction = conn.transaction().unwrap();
        assert_eq!(
            remove_all_for_connection(&transaction, "connection", 20).unwrap(),
            1
        );
        transaction.commit().unwrap();

        assert_eq!(
            list_for_next_up(&conn, EARLIEST_TEST_DATE)
                .unwrap()
                .iter()
                .map(|row| row.schedule.id.as_str())
                .collect::<Vec<_>>(),
            vec!["local-schedule"]
        );
        assert_eq!(
            get(&conn, "provider-schedule")
                .unwrap()
                .unwrap()
                .removed_at_unix_ms,
            Some(20)
        );
        assert_eq!(
            workout_definitions::list_local(&conn)
                .unwrap()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["local-definition"]
        );
    }
}
