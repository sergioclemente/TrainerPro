//! TPW workout-definition persistence.

#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "provider persistence lands before the connection command that consumes it"
    )
)]

use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkoutDefinitionRow {
    pub id: String,
    pub tpw_json: String,
    pub origin: Option<String>,
}

pub struct NewWorkoutDefinition<'a> {
    pub id: &'a str,
    pub tpw_json: &'a str,
    pub created_at_ms: i64,
}

pub struct ProviderWorkoutDefinition<'a> {
    pub id: &'a str,
    pub tpw_json: &'a str,
    pub origin: &'a str,
    pub origin_id: Option<i64>,
    pub origin_ref: &'a str,
    pub synced_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteResult {
    Deleted,
    NotFound,
    ReferencedBySchedule,
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkoutDefinitionRow> {
    Ok(WorkoutDefinitionRow {
        id: row.get(0)?,
        tpw_json: row.get(1)?,
        origin: row.get(2)?,
    })
}

/// Find a reusable definition in TrainerPro's local-library lifecycle.
/// Provider-scheduled definitions are executable cache entries, not local
/// copies, even when their canonical TPW happens to be identical.
pub fn find_local_id_by_tpw_json(
    conn: &Connection,
    tpw_json: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM workout_definitions
         WHERE tpw_json = ?1 AND retired_at_unix_ms IS NULL
           AND NOT EXISTS (
             SELECT 1 FROM scheduled_workouts sw
             WHERE sw.workout_definition_id = workout_definitions.id
               AND sw.provider_connection_id IS NOT NULL
           )
         ORDER BY created_at DESC LIMIT 1",
        [tpw_json],
        |row| row.get(0),
    )
    .optional()
}

pub fn insert(conn: &Connection, definition: &NewWorkoutDefinition<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO workout_definitions (id, tpw_json, created_at, updated_at)
         VALUES (?1,?2,?3,?3)",
        rusqlite::params![definition.id, definition.tpw_json, definition.created_at_ms,],
    )?;
    Ok(())
}

pub fn insert_provider_copy(
    conn: &Connection,
    definition: &ProviderWorkoutDefinition<'_>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO workout_definitions (
           id, tpw_json, created_at, updated_at, origin, origin_id, origin_ref)
         VALUES (?1,?2,?3,?3,?4,?5,?6)",
        rusqlite::params![
            definition.id,
            definition.tpw_json,
            definition.synced_at_unix_ms,
            definition.origin,
            definition.origin_id,
            definition.origin_ref,
        ],
    )?;
    Ok(())
}

pub fn update_provider_copy(
    conn: &Connection,
    definition: &ProviderWorkoutDefinition<'_>,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE workout_definitions
         SET tpw_json = ?1, updated_at = ?2, origin = ?3,
             origin_id = ?4, origin_ref = ?5, retired_at_unix_ms = NULL
         WHERE id = ?6",
        rusqlite::params![
            definition.tpw_json,
            definition.synced_at_unix_ms,
            definition.origin,
            definition.origin_id,
            definition.origin_ref,
            definition.id,
        ],
    )? > 0)
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<WorkoutDefinitionRow>> {
    conn.query_row(
        "SELECT id, tpw_json, origin FROM workout_definitions WHERE id = ?1",
        [id],
        read_row,
    )
    .optional()
}

/// List definitions that belong to the local Library. Provider-scheduled
/// definitions remain addressable by Next Up but are not Library entries.
pub fn list_local(conn: &Connection) -> rusqlite::Result<Vec<WorkoutDefinitionRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, tpw_json, origin FROM workout_definitions
         WHERE retired_at_unix_ms IS NULL
           AND NOT EXISTS (
             SELECT 1 FROM scheduled_workouts sw
             WHERE sw.workout_definition_id = workout_definitions.id
               AND sw.provider_connection_id IS NOT NULL
           )
         ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], read_row)?;
    rows.collect()
}

/// Definitions ranked by how often they produced an Activity on or after the
/// cutoff, then by the most recent matching Activity. The Next Up projection
/// decides how many to expose and removes definitions already scheduled.
pub fn list_by_activity_frequency_since(
    conn: &Connection,
    cutoff_unix_ms: i64,
) -> rusqlite::Result<Vec<WorkoutDefinitionRow>> {
    let mut stmt = conn.prepare(
        "SELECT wd.id, wd.tpw_json, wd.origin
         FROM workout_definitions wd
         JOIN activities a ON a.workout_definition_id = wd.id
         WHERE a.started_at_unix_ms >= ?1 AND wd.retired_at_unix_ms IS NULL
         GROUP BY wd.id, wd.tpw_json, wd.origin
         ORDER BY COUNT(a.id) DESC, MAX(a.started_at_unix_ms) DESC, wd.id",
    )?;
    let rows = stmt.query_map([cutoff_unix_ms], read_row)?;
    rows.collect()
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<DeleteResult> {
    let deleted = conn.execute(
        "DELETE FROM workout_definitions
         WHERE id = ?1
           AND NOT EXISTS (
             SELECT 1 FROM scheduled_workouts sw
             WHERE sw.workout_definition_id = workout_definitions.id
           )",
        [id],
    )? > 0;
    if deleted {
        return Ok(DeleteResult::Deleted);
    }

    let exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM workout_definitions WHERE id = ?1)",
        [id],
        |row| row.get(0),
    )?;
    Ok(if exists {
        DeleteResult::ReferencedBySchedule
    } else {
        DeleteResult::NotFound
    })
}

pub fn set_origin(
    conn: &Connection,
    id: &str,
    origin: &str,
    origin_id: Option<i64>,
    origin_ref: &str,
    updated_at_ms: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE workout_definitions
         SET origin = ?1, origin_id = ?2, origin_ref = ?3, updated_at = ?4
         WHERE id = ?5",
        rusqlite::params![origin, origin_id, origin_ref, updated_at_ms, id,],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TPW_JSON: &str = r#"{"format":"TPW","version":1,"title":"Endurance","prescription":{"sport":"cycling","steps":[{"type":"steady","duration_seconds":3600,"power":{"type":"percent_ftp","percent":70}}]}}"#;

    fn insert_fixture(conn: &Connection, id: &str, created_at_ms: i64) {
        insert(
            conn,
            &NewWorkoutDefinition {
                id,
                tpw_json: &TPW_JSON.replace("Endurance", id),
                created_at_ms,
            },
        )
        .unwrap();
    }

    #[test]
    fn insert_find_list_update_and_delete_definition() {
        let conn = super::super::test_connection();
        insert_fixture(&conn, "older", 10);
        insert_fixture(&conn, "newer", 20);

        let older_json = TPW_JSON.replace("Endurance", "older");
        assert_eq!(
            find_local_id_by_tpw_json(&conn, &older_json)
                .unwrap()
                .as_deref(),
            Some("older")
        );
        assert_eq!(
            list_local(&conn)
                .unwrap()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["newer", "older"]
        );

        set_origin(&conn, "older", "planner", Some(42), "42", 30).unwrap();
        let row = get(&conn, "older").unwrap().unwrap();
        assert_eq!(row.tpw_json, older_json);
        assert_eq!(row.origin.as_deref(), Some("planner"));

        assert_eq!(delete(&conn, "older").unwrap(), DeleteResult::Deleted);
        assert_eq!(get(&conn, "older").unwrap(), None);
        assert_eq!(delete(&conn, "missing").unwrap(), DeleteResult::NotFound);
    }

    #[test]
    fn provider_scheduled_definitions_are_not_local_library_duplicates() {
        let conn = super::super::test_connection();
        let provider_json = TPW_JSON.replace("Endurance", "shared");
        insert_provider_copy(
            &conn,
            &ProviderWorkoutDefinition {
                id: "provider-definition",
                tpw_json: &provider_json,
                origin: "intervals_icu",
                origin_id: Some(42),
                origin_ref: "42",
                synced_at_unix_ms: 20,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO provider_connections (
               id, provider, external_account_id, time_zone,
               created_at_unix_ms, updated_at_unix_ms)
             VALUES ('connection','intervals_icu','athlete','Europe/Zurich',10,10)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scheduled_workouts (
               id, workout_definition_id, scheduled_date_local,
               created_at_unix_ms, updated_at_unix_ms,
               provider_connection_id, external_event_id)
             VALUES ('provider-schedule','provider-definition','2030-01-01',
                     20,20,'connection','42')",
            [],
        )
        .unwrap();

        assert_eq!(
            find_local_id_by_tpw_json(&conn, &provider_json).unwrap(),
            None
        );
        assert!(list_local(&conn).unwrap().is_empty());
        assert_eq!(
            delete(&conn, "provider-definition").unwrap(),
            DeleteResult::ReferencedBySchedule
        );
        assert!(get(&conn, "provider-definition").unwrap().is_some());

        insert(
            &conn,
            &NewWorkoutDefinition {
                id: "local-copy",
                tpw_json: &provider_json,
                created_at_ms: 30,
            },
        )
        .unwrap();
        assert_eq!(
            find_local_id_by_tpw_json(&conn, &provider_json)
                .unwrap()
                .as_deref(),
            Some("local-copy")
        );
        assert_eq!(
            list_local(&conn)
                .unwrap()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["local-copy"]
        );
    }

    #[test]
    fn favorite_definitions_rank_recent_activity_and_exclude_stale_history() {
        let conn = super::super::test_connection();
        insert_fixture(&conn, "frequent", 10);
        insert_fixture(&conn, "recent", 20);
        insert_fixture(&conn, "stale", 30);
        insert_fixture(&conn, "unused", 40);

        for (id, definition_id, started_at_unix_ms) in [
            ("frequent-1", "frequent", 300),
            ("frequent-2", "frequent", 400),
            ("recent-1", "recent", 500),
            ("stale-1", "stale", 100),
            ("stale-2", "stale", 150),
            ("stale-3", "stale", 200),
        ] {
            conn.execute(
                "INSERT INTO activities (
                   id, workout_definition_id, workout_name,
                   started_at_unix_ms, elapsed_s, timer_s, ftp_used_w,
                   final_intensity_multiplier, fit_path, journal_path,
                   completed_pct)
                 VALUES (?1,?2,'Workout',?3,60,60,250,1.0,
                         '/tmp/activity.fit','/tmp/session.jsonl',100.0)",
                rusqlite::params![id, definition_id, started_at_unix_ms],
            )
            .unwrap();
        }

        assert_eq!(
            list_by_activity_frequency_since(&conn, 250)
                .unwrap()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["frequent", "recent"]
        );
    }
}
