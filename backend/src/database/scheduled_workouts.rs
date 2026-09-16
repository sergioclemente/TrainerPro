//! Scheduled-workout persistence.

use rusqlite::{Connection, OptionalExtension};

use super::workout_definitions::WorkoutDefinitionRow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledWorkoutRow {
    pub id: String,
    pub workout_definition_id: String,
    pub scheduled_date_local: String,
    pub scheduled_time_local: Option<String>,
    pub scheduled_time_zone: Option<String>,
    pub removed_at_unix_ms: Option<i64>,
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
    })
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<ScheduledWorkoutRow>> {
    conn.query_row(
        "SELECT id, workout_definition_id, scheduled_date_local,
                scheduled_time_local, scheduled_time_zone, removed_at_unix_ms
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
) -> rusqlite::Result<Vec<ScheduledWorkoutWithDefinitionRow>> {
    let mut stmt = conn.prepare(
        "SELECT sw.id, sw.workout_definition_id, sw.scheduled_date_local,
                sw.scheduled_time_local, sw.scheduled_time_zone,
                sw.removed_at_unix_ms, wd.id, wd.tpw_json, wd.origin
         FROM scheduled_workouts sw
         JOIN workout_definitions wd ON wd.id = sw.workout_definition_id
         WHERE sw.removed_at_unix_ms IS NULL
           AND NOT EXISTS (
             SELECT 1 FROM activities a WHERE a.scheduled_workout_id = sw.id
           )
         ORDER BY sw.scheduled_date_local,
                  COALESCE(sw.scheduled_time_local, '00:00:00'), sw.id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ScheduledWorkoutWithDefinitionRow {
            schedule: ScheduledWorkoutRow {
                id: row.get(0)?,
                workout_definition_id: row.get(1)?,
                scheduled_date_local: row.get(2)?,
                scheduled_time_local: row.get(3)?,
                scheduled_time_zone: row.get(4)?,
                removed_at_unix_ms: row.get(5)?,
            },
            definition: WorkoutDefinitionRow {
                id: row.get(6)?,
                tpw_json: row.get(7)?,
                origin: row.get(8)?,
            },
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::workout_definitions::{self, NewWorkoutDefinition};

    const TPW_JSON: &str = r#"{"format":"TPW","version":1,"title":"Endurance","prescription":{"sport":"cycling","steps":[{"type":"steady","duration_seconds":3600,"power":{"type":"percent_ftp","percent":70}}]}}"#;

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
            list_for_next_up(&conn)
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
        assert!(list_for_next_up(&conn).unwrap().is_empty());
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
}
