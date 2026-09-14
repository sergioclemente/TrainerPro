//! Activity persistence.

use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityListRow {
    pub id: String,
    pub workout_name: String,
    pub started_at: i64,
    pub timer_s: u32,
    pub avg_power: Option<u16>,
    pub np: Option<u16>,
    pub tss: Option<f64>,
    pub avg_hr: Option<u16>,
    pub completed_pct: f64,
    pub fit_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityArtifacts {
    pub fit_path: String,
    pub journal_path: String,
}

pub struct NewActivity<'a> {
    pub id: &'a str,
    pub workout_definition_id: Option<&'a str>,
    pub workout_name: &'a str,
    pub started_at_ms: i64,
    pub elapsed_s: u32,
    pub timer_s: u32,
    pub avg_power: Option<u16>,
    pub max_power: Option<u16>,
    pub np: Option<u16>,
    pub if_: Option<f64>,
    pub tss: Option<f64>,
    pub avg_hr: Option<u16>,
    pub max_hr: Option<u16>,
    pub avg_cadence: Option<u16>,
    pub kj: u32,
    pub ftp_used: u16,
    pub intensity_final: f64,
    pub fit_path: &'a str,
    pub journal_path: &'a str,
    pub completed_pct: f64,
}

pub fn insert(conn: &Connection, activity: &NewActivity<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO activities (id, workout_definition_id, workout_name, started_at, elapsed_s,
           timer_s, avg_power, max_power, np, if_, tss, avg_hr, max_hr, avg_cadence,
           kj, ftp_used, intensity_final, fit_path, journal_path, completed_pct)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
        rusqlite::params![
            activity.id,
            activity.workout_definition_id,
            activity.workout_name,
            activity.started_at_ms,
            activity.elapsed_s,
            activity.timer_s,
            activity.avg_power,
            activity.max_power,
            activity.np,
            activity.if_,
            activity.tss,
            activity.avg_hr,
            activity.max_hr,
            activity.avg_cadence,
            activity.kj,
            activity.ftp_used,
            activity.intensity_final,
            activity.fit_path,
            activity.journal_path,
            activity.completed_pct,
        ],
    )?;
    Ok(())
}

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<ActivityListRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, workout_name, started_at, timer_s, avg_power, np, tss, avg_hr,
                completed_pct, fit_path
         FROM activities ORDER BY started_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ActivityListRow {
            id: row.get(0)?,
            workout_name: row.get(1)?,
            started_at: row.get(2)?,
            timer_s: row.get(3)?,
            avg_power: row.get(4)?,
            np: row.get(5)?,
            tss: row.get(6)?,
            avg_hr: row.get(7)?,
            completed_pct: row.get(8)?,
            fit_path: row.get(9)?,
        })
    })?;
    rows.collect()
}

pub fn artifacts(conn: &Connection, id: &str) -> rusqlite::Result<Option<ActivityArtifacts>> {
    conn.query_row(
        "SELECT fit_path, journal_path FROM activities WHERE id = ?1",
        [id],
        |row| {
            Ok(ActivityArtifacts {
                fit_path: row.get(0)?,
                journal_path: row.get(1)?,
            })
        },
    )
    .optional()
}

pub fn fit_path(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT fit_path FROM activities WHERE id = ?1", [id], |row| {
        row.get(0)
    })
    .optional()
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<Option<ActivityArtifacts>> {
    let artifacts = artifacts(conn, id)?;
    conn.execute("DELETE FROM activities WHERE id = ?1", [id])?;
    Ok(artifacts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture<'a>(id: &'a str, fit_path: &'a str, started_at_ms: i64) -> NewActivity<'a> {
        NewActivity {
            id,
            workout_definition_id: None,
            workout_name: "Endurance",
            started_at_ms,
            elapsed_s: 3600,
            timer_s: 3500,
            avg_power: Some(180),
            max_power: Some(400),
            np: Some(190),
            if_: Some(0.76),
            tss: Some(56.0),
            avg_hr: Some(140),
            max_hr: Some(170),
            avg_cadence: Some(88),
            kj: 630,
            ftp_used: 250,
            intensity_final: 1.0,
            fit_path,
            journal_path: "/tmp/ride.jsonl",
            completed_pct: 100.0,
        }
    }

    #[test]
    fn insert_list_artifacts_and_delete_activity() {
        let conn = super::super::test_connection();
        insert(&conn, &fixture("older", "/tmp/older.fit", 10)).unwrap();
        insert(&conn, &fixture("newer", "/tmp/newer.fit", 20)).unwrap();

        let rows = list(&conn).unwrap();
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            vec!["newer", "older"]
        );
        assert_eq!(
            fit_path(&conn, "older").unwrap().as_deref(),
            Some("/tmp/older.fit")
        );
        assert_eq!(
            artifacts(&conn, "older").unwrap().unwrap(),
            ActivityArtifacts {
                fit_path: "/tmp/older.fit".into(),
                journal_path: "/tmp/ride.jsonl".into(),
            }
        );

        assert_eq!(
            delete(&conn, "older").unwrap().unwrap().fit_path,
            "/tmp/older.fit"
        );
        assert!(list(&conn).unwrap().iter().all(|row| row.id != "older"));
    }
}
