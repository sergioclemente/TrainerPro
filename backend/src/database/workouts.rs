//! Current workout-library persistence.

use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq)]
pub struct WorkoutRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source_format: String,
    pub duration_s: u32,
    pub est_if: Option<f64>,
    pub est_tss: Option<f64>,
    pub graph_json: String,
    pub origin: Option<String>,
}

pub struct NewWorkout<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub source_format: &'a str,
    pub file_path: &'a str,
    pub sha256: &'a str,
    pub duration_s: u32,
    pub est_if: f64,
    pub est_tss: f64,
    pub graph_json: &'a str,
    pub imported_at_ms: i64,
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkoutRow> {
    Ok(WorkoutRow {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        source_format: row.get(3)?,
        duration_s: row.get(4)?,
        est_if: row.get(5)?,
        est_tss: row.get(6)?,
        graph_json: row.get(7)?,
        origin: row.get(8)?,
    })
}

pub fn find_id_by_sha256(conn: &Connection, sha256: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM workouts WHERE sha256 = ?1",
        [sha256],
        |row| row.get(0),
    )
    .optional()
}

pub fn insert(conn: &Connection, workout: &NewWorkout<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO workouts (id, name, description, source_format, file_path, sha256,
           duration_s, est_if, est_tss, graph_json, imported_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        rusqlite::params![
            workout.id,
            workout.name,
            workout.description,
            workout.source_format,
            workout.file_path,
            workout.sha256,
            workout.duration_s,
            workout.est_if,
            workout.est_tss,
            workout.graph_json,
            workout.imported_at_ms,
        ],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<WorkoutRow>> {
    conn.query_row(
        "SELECT id, name, description, source_format, duration_s, est_if, est_tss, graph_json, origin
         FROM workouts WHERE id = ?1",
        [id],
        read_row,
    )
    .optional()
}

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<WorkoutRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, description, source_format, duration_s, est_if, est_tss, graph_json, origin
         FROM workouts ORDER BY imported_at DESC",
    )?;
    let rows = stmt.query_map([], read_row)?;
    rows.collect()
}

pub fn file_path(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT file_path FROM workouts WHERE id = ?1",
        [id],
        |row| row.get(0),
    )
    .optional()
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    let file_path = file_path(conn, id)?;
    conn.execute("DELETE FROM workouts WHERE id = ?1", [id])?;
    Ok(file_path)
}

pub fn set_origin(
    conn: &Connection,
    id: &str,
    origin: &str,
    origin_id: Option<i64>,
    origin_ref: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE workouts SET origin = ?1, origin_id = ?2, origin_ref = ?3 WHERE id = ?4",
        rusqlite::params![origin, origin_id, origin_ref, id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_fixture(conn: &Connection, id: &str, imported_at_ms: i64) {
        insert(
            conn,
            &NewWorkout {
                id,
                name: "Endurance",
                description: "Steady aerobic ride",
                source_format: "zwo",
                file_path: "/tmp/endurance.zwo",
                sha256: id,
                duration_s: 3600,
                est_if: 0.7,
                est_tss: 49.0,
                graph_json: "[[0,70],[3600,70]]",
                imported_at_ms,
            },
        )
        .unwrap();
    }

    #[test]
    fn insert_find_list_update_and_delete_workout() {
        let conn = super::super::test_connection();
        insert_fixture(&conn, "older", 10);
        insert_fixture(&conn, "newer", 20);

        assert_eq!(
            find_id_by_sha256(&conn, "older").unwrap().as_deref(),
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

        set_origin(&conn, "older", "planner", Some(42), "42").unwrap();
        let row = get(&conn, "older").unwrap().unwrap();
        assert_eq!(row.name, "Endurance");
        assert_eq!(row.origin.as_deref(), Some("planner"));
        assert_eq!(
            file_path(&conn, "older").unwrap().as_deref(),
            Some("/tmp/endurance.zwo")
        );

        assert_eq!(
            delete(&conn, "older").unwrap().as_deref(),
            Some("/tmp/endurance.zwo")
        );
        assert_eq!(get(&conn, "older").unwrap(), None);
    }
}
