//! Activity persistence.

use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityListRow {
    pub id: String,
    pub scheduled_workout_id: Option<String>,
    pub workout_name: String,
    pub started_at_unix_ms: i64,
    pub timer_s: u32,
    pub average_power_w: Option<u16>,
    pub normalized_power_w: Option<u16>,
    pub training_stress_score: Option<f64>,
    pub average_heart_rate_bpm: Option<u16>,
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
    pub workout_session_id: &'a str,
    pub scheduled_workout_id: Option<&'a str>,
    pub workout_definition_id: Option<&'a str>,
    pub workout_definition_snapshot_json: &'a str,
    pub workout_name: &'a str,
    pub started_at_unix_ms: i64,
    pub elapsed_s: u32,
    pub timer_s: u32,
    pub average_power_w: Option<u16>,
    pub max_power_w: Option<u16>,
    pub normalized_power_w: Option<u16>,
    pub intensity_factor: Option<f64>,
    pub training_stress_score: Option<f64>,
    pub average_heart_rate_bpm: Option<u16>,
    pub max_heart_rate_bpm: Option<u16>,
    pub average_cadence_rpm: Option<u16>,
    pub work_kj: u32,
    pub ftp_used_w: u16,
    pub final_intensity_multiplier: f64,
    pub fit_path: &'a str,
    pub journal_path: &'a str,
    pub completed_pct: f64,
}

pub fn insert(conn: &Connection, activity: &NewActivity<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO activities (
           id, workout_session_id, workout_definition_id,
           workout_definition_snapshot_json, workout_name, started_at_unix_ms, elapsed_s,
           timer_s, average_power_w, max_power_w, normalized_power_w,
           intensity_factor, training_stress_score, average_heart_rate_bpm,
           max_heart_rate_bpm, average_cadence_rpm, work_kj, ftp_used_w,
           final_intensity_multiplier, fit_path, journal_path, completed_pct,
           scheduled_workout_id)
         VALUES (?1,?2,(SELECT id FROM workout_definitions WHERE id = ?3),?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,
                 (SELECT id FROM scheduled_workouts WHERE id = ?23))",
        rusqlite::params![
            activity.id,
            activity.workout_session_id,
            activity.workout_definition_id,
            activity.workout_definition_snapshot_json,
            activity.workout_name,
            activity.started_at_unix_ms,
            activity.elapsed_s,
            activity.timer_s,
            activity.average_power_w,
            activity.max_power_w,
            activity.normalized_power_w,
            activity.intensity_factor,
            activity.training_stress_score,
            activity.average_heart_rate_bpm,
            activity.max_heart_rate_bpm,
            activity.average_cadence_rpm,
            activity.work_kj,
            activity.ftp_used_w,
            activity.final_intensity_multiplier,
            activity.fit_path,
            activity.journal_path,
            activity.completed_pct,
            activity.scheduled_workout_id,
        ],
    )?;
    Ok(())
}

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<ActivityListRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, scheduled_workout_id, workout_name, started_at_unix_ms,
                timer_s, average_power_w,
                normalized_power_w, training_stress_score, average_heart_rate_bpm,
                completed_pct, fit_path
         FROM activities ORDER BY started_at_unix_ms DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ActivityListRow {
            id: row.get(0)?,
            scheduled_workout_id: row.get(1)?,
            workout_name: row.get(2)?,
            started_at_unix_ms: row.get(3)?,
            timer_s: row.get(4)?,
            average_power_w: row.get(5)?,
            normalized_power_w: row.get(6)?,
            training_stress_score: row.get(7)?,
            average_heart_rate_bpm: row.get(8)?,
            completed_pct: row.get(9)?,
            fit_path: row.get(10)?,
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

    const TPW_JSON: &str = r#"{"format":"TPW","version":1,"title":"Endurance","prescription":{"sport":"cycling","steps":[{"type":"steady","duration_seconds":60,"power":{"type":"percent_ftp","percent":70}}]}}"#;

    fn fixture<'a>(id: &'a str, fit_path: &'a str, started_at_unix_ms: i64) -> NewActivity<'a> {
        NewActivity {
            id,
            workout_session_id: id,
            scheduled_workout_id: None,
            workout_definition_id: Some("definition-no-longer-present"),
            workout_definition_snapshot_json: TPW_JSON,
            workout_name: "Endurance",
            started_at_unix_ms,
            elapsed_s: 3600,
            timer_s: 3500,
            average_power_w: Some(180),
            max_power_w: Some(400),
            normalized_power_w: Some(190),
            intensity_factor: Some(0.76),
            training_stress_score: Some(56.0),
            average_heart_rate_bpm: Some(140),
            max_heart_rate_bpm: Some(170),
            average_cadence_rpm: Some(88),
            work_kj: 630,
            ftp_used_w: 250,
            final_intensity_multiplier: 1.0,
            fit_path,
            journal_path: "/tmp/session.jsonl",
            completed_pct: 100.0,
        }
    }

    #[test]
    fn insert_list_artifacts_and_delete_activity() {
        let conn = super::super::test_connection();
        super::super::workout_definitions::insert(
            &conn,
            &super::super::workout_definitions::NewWorkoutDefinition {
                id: "definition-present",
                tpw_json: TPW_JSON,
                created_at_ms: 1,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scheduled_workouts (
               id, workout_definition_id, scheduled_date_local,
               created_at_unix_ms, updated_at_unix_ms)
             VALUES ('scheduled-present','definition-present','2026-09-15',1,1)",
            [],
        )
        .unwrap();
        let mut older = fixture("older", "/tmp/older.fit", 10);
        older.scheduled_workout_id = Some("scheduled-present");
        older.workout_definition_id = Some("definition-present");
        insert(&conn, &older).unwrap();
        insert(&conn, &fixture("newer", "/tmp/newer.fit", 20)).unwrap();

        let rows = list(&conn).unwrap();
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            vec!["newer", "older"]
        );
        assert_eq!(
            rows[1].scheduled_workout_id.as_deref(),
            Some("scheduled-present")
        );
        assert_eq!(
            fit_path(&conn, "older").unwrap().as_deref(),
            Some("/tmp/older.fit")
        );
        assert_eq!(
            artifacts(&conn, "older").unwrap().unwrap(),
            ActivityArtifacts {
                fit_path: "/tmp/older.fit".into(),
                journal_path: "/tmp/session.jsonl".into(),
            }
        );
        let context: (String, Option<String>, String) = conn
            .query_row(
                "SELECT workout_session_id, workout_definition_id,
                        workout_definition_snapshot_json
                 FROM activities WHERE id = 'older'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            context,
            (
                "older".into(),
                Some("definition-present".into()),
                TPW_JSON.into(),
            )
        );
        tp_core::workout_definition::WorkoutDefinition::from_json(&context.2).unwrap();

        conn.execute(
            "DELETE FROM scheduled_workouts WHERE id = 'scheduled-present'",
            [],
        )
        .unwrap();
        assert_eq!(
            list(&conn).unwrap()[1].scheduled_workout_id,
            None,
            "activity must survive schedule removal"
        );
        super::super::workout_definitions::delete(&conn, "definition-present").unwrap();
        let detached_context: (Option<String>, String) = conn
            .query_row(
                "SELECT workout_definition_id, workout_definition_snapshot_json
                 FROM activities WHERE id = 'older'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(detached_context, (None, TPW_JSON.into()));

        let mut duplicate_session = fixture("duplicate", "/tmp/duplicate.fit", 30);
        duplicate_session.workout_session_id = "newer";
        assert!(insert(&conn, &duplicate_session).is_err());

        assert_eq!(
            delete(&conn, "older").unwrap().unwrap().fit_path,
            "/tmp/older.fit"
        );
        assert!(list(&conn).unwrap().iter().all(|row| row.id != "older"));
    }
}
