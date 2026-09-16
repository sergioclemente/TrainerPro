//! TPW workout-definition persistence.

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

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkoutDefinitionRow> {
    Ok(WorkoutDefinitionRow {
        id: row.get(0)?,
        tpw_json: row.get(1)?,
        origin: row.get(2)?,
    })
}

pub fn find_id_by_tpw_json(conn: &Connection, tpw_json: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM workout_definitions WHERE tpw_json = ?1 ORDER BY created_at DESC LIMIT 1",
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

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<WorkoutDefinitionRow>> {
    conn.query_row(
        "SELECT id, tpw_json, origin FROM workout_definitions WHERE id = ?1",
        [id],
        read_row,
    )
    .optional()
}

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<WorkoutDefinitionRow>> {
    let mut stmt = conn
        .prepare("SELECT id, tpw_json, origin FROM workout_definitions ORDER BY created_at DESC")?;
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
         WHERE a.started_at_unix_ms >= ?1
         GROUP BY wd.id, wd.tpw_json, wd.origin
         ORDER BY COUNT(a.id) DESC, MAX(a.started_at_unix_ms) DESC, wd.id",
    )?;
    let rows = stmt.query_map([cutoff_unix_ms], read_row)?;
    rows.collect()
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute("DELETE FROM workout_definitions WHERE id = ?1", [id])? > 0)
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
            find_id_by_tpw_json(&conn, &older_json).unwrap().as_deref(),
            Some("older")
        );
        assert_eq!(
            list(&conn)
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

        assert!(delete(&conn, "older").unwrap());
        assert_eq!(get(&conn, "older").unwrap(), None);
        assert!(!delete(&conn, "missing").unwrap());
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
