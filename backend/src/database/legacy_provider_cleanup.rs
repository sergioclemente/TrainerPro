//! Durable vault-deletion markers left by the removed provider's migration.

use rusqlite::Connection;

const CREDENTIAL_CLEANUP_PREFIX: &str = "legacy_intervals_icu_credential:";

pub fn pending_credentials(conn: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    let mut statement =
        conn.prepare("SELECT key, value FROM settings WHERE key GLOB ?1 ORDER BY key")?;
    let rows = statement.query_map([format!("{CREDENTIAL_CLEANUP_PREFIX}*")], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    rows.collect()
}

pub fn credential_deleted(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{
        self, activities, activity_uploads, scheduled_workouts, workout_definitions,
    };

    const BEFORE_REMOVAL_VERSION: usize = 14;
    const WORKOUT_JSON: &str = r#"{"format":"WorkoutSpec","version":1,"title":"Endurance","prescription":{"sport":"cycling","steps":[{"type":"steady","duration_seconds":60,"power":{"type":"watts","watts":150}}]}}"#;

    fn old_database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        for sql in database::MIGRATIONS.iter().take(BEFORE_REMOVAL_VERSION) {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", BEFORE_REMOVAL_VERSION)
            .unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn
    }

    #[test]
    fn upgrade_removes_active_disconnected_and_retired_cache_without_losing_history() {
        let conn = old_database();
        conn.execute_batch(
            "INSERT INTO provider_connections(id,provider,external_account_id,created_at_unix_ms,updated_at_unix_ms,disconnected_at_unix_ms)
             VALUES ('active','intervals_icu','first',1,1,NULL),
                    ('old','intervals_icu','second',1,2,2),
                    ('garmin','garmin','third',1,1,NULL);
             INSERT INTO settings VALUES ('profile','saved');",
        ).unwrap();
        for (id, origin, retired) in [
            ("active-definition", Some("intervals_icu"), None),
            ("retired-definition", Some("intervals_icu"), Some(2)),
            ("orphan-definition", Some("intervals_icu"), Some(2)),
            ("shared-local", Some("intervals_icu"), None),
            ("shared-garmin", Some("intervals_icu"), None),
            ("local-copy", None, None),
        ] {
            conn.execute(
                "INSERT INTO workout_definitions(id,tpw_json,origin,created_at,updated_at,retired_at_unix_ms) VALUES (?1,?2,?3,1,1,?4)",
                rusqlite::params![id, WORKOUT_JSON, origin, retired],
            ).unwrap();
        }
        for (id, definition, account, removed) in [
            ("active-schedule", "active-definition", Some("active"), None),
            (
                "retired-schedule",
                "retired-definition",
                Some("old"),
                Some(2),
            ),
            ("shared-local-old", "shared-local", Some("old"), Some(2)),
            ("local-schedule", "shared-local", None, None),
            ("shared-garmin-old", "shared-garmin", Some("active"), None),
            ("garmin-schedule", "shared-garmin", Some("garmin"), None),
        ] {
            conn.execute(
                "INSERT INTO scheduled_workouts(id,workout_definition_id,scheduled_date_local,created_at_unix_ms,updated_at_unix_ms,provider_connection_id,external_event_id,removed_at_unix_ms)
                 VALUES (?1,?2,'2030-01-01',1,1,?3,?4,?5)",
                rusqlite::params![id, definition, account, account.map(|_| id), removed],
            ).unwrap();
        }
        conn.execute(
            "INSERT INTO activities(id,workout_definition_id,scheduled_workout_id,workout_name,started_at_unix_ms,elapsed_s,timer_s,average_power_w,max_power_w,normalized_power_w,intensity_factor,training_stress_score,average_heart_rate_bpm,max_heart_rate_bpm,average_cadence_rpm,work_kj,ftp_used_w,final_intensity_multiplier,fit_path,journal_path,completed_pct,workout_session_id,workout_definition_snapshot_json,icu_activity_id)
             VALUES ('ride','active-definition','active-schedule','Endurance',123,60,55,150,200,160,0.8,1.1,130,145,90,8,200,1.0,'/preserved.fit','/preserved.jsonl',90,'session',?1,'obsolete')",
            [WORKOUT_JSON],
        ).unwrap();
        conn.execute_batch(
            "INSERT INTO activity_uploads VALUES ('ride','garmin','confirmed-garmin',3), ('ride','old','obsolete',2);",
        ).unwrap();
        // Compare every Activity field other than the two detached references
        // and the deliberately removed column, including fields added later.
        let columns = conn.prepare("SELECT name FROM pragma_table_info('activities') WHERE name NOT IN ('workout_definition_id','scheduled_workout_id','icu_activity_id') ORDER BY cid")
            .unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap().join(",");
        let activity_query = format!("SELECT {columns} FROM activities WHERE id='ride'");
        let read_activity = |db: &Connection| {
            db.query_row(&activity_query, [], |row| {
                (0..row.as_ref().column_count())
                    .map(|index| row.get::<_, rusqlite::types::Value>(index))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
        };
        let before = read_activity(&conn);
        let conn = database::prepare(conn).unwrap();
        assert_eq!(read_activity(&conn), before);
        assert_eq!(
            activities::list(&conn).unwrap()[0].scheduled_workout_id,
            None
        );
        let definition_id: Option<String> = conn
            .query_row(
                "SELECT workout_definition_id FROM activities WHERE id='ride'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(definition_id, None);
        assert!(conn
            .prepare("SELECT icu_activity_id FROM activities")
            .is_err());
        for id in ["active", "old"] {
            assert!(database::provider_connections::get(&conn, id)
                .unwrap()
                .is_none());
        }
        assert!(database::provider_connections::get(&conn, "garmin")
            .unwrap()
            .is_some());
        for id in [
            "active-definition",
            "retired-definition",
            "orphan-definition",
        ] {
            assert!(workout_definitions::get(&conn, id).unwrap().is_none());
        }
        for id in ["shared-local", "shared-garmin", "local-copy"] {
            assert_eq!(
                workout_definitions::get(&conn, id)
                    .unwrap()
                    .unwrap()
                    .tpw_json,
                WORKOUT_JSON
            );
        }
        let next_up = scheduled_workouts::list_for_next_up(&conn, "2029-01-01").unwrap();
        let mut schedule_ids = next_up
            .iter()
            .map(|row| row.schedule.id.as_str())
            .collect::<Vec<_>>();
        schedule_ids.sort_unstable();
        assert_eq!(schedule_ids, ["garmin-schedule", "local-schedule"]);
        let local = workout_definitions::list_local(&conn).unwrap();
        let mut local_ids = local.iter().map(|row| row.id.as_str()).collect::<Vec<_>>();
        local_ids.sort_unstable();
        assert_eq!(local_ids, ["local-copy", "shared-local"]);
        let receipt = activity_uploads::get(&conn, "garmin", "ride")
            .unwrap()
            .unwrap();
        assert_eq!(receipt.remote_activity_id, "confirmed-garmin");
        assert!(activity_uploads::list(&conn, "old").unwrap().is_empty());
        let pending = pending_credentials(&conn).unwrap();
        assert_eq!(
            pending,
            [
                (
                    format!("{CREDENTIAL_CLEANUP_PREFIX}active"),
                    "intervals_icu:active".into()
                ),
                (
                    format!("{CREDENTIAL_CLEANUP_PREFIX}old"),
                    "intervals_icu:old".into()
                ),
            ]
        );
        assert_eq!(
            database::settings::get(&conn, "profile")
                .unwrap()
                .as_deref(),
            Some("saved")
        );
        let violations: i64 = conn
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(violations, 0);
        let conn = database::prepare(conn).unwrap();
        assert_eq!(pending_credentials(&conn).unwrap(), pending);
        assert_eq!(read_activity(&conn), before);
    }

    #[test]
    fn fresh_database_needs_no_vault_cleanup() {
        let conn = database::test_connection();
        assert!(pending_credentials(&conn).unwrap().is_empty());
        assert!(conn
            .prepare("SELECT icu_activity_id FROM activities")
            .is_err());
    }

    #[test]
    fn released_schema_upgrades_through_provider_table_rebuild_and_removal() {
        const SQUASHED_SCHEMA_VERSION: usize = 12;
        let conn = Connection::open_in_memory().unwrap();
        for sql in database::MIGRATIONS.iter().take(SQUASHED_SCHEMA_VERSION) {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", SQUASHED_SCHEMA_VERSION)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO provider_connections(id,provider,external_account_id,time_zone,created_at_unix_ms,updated_at_unix_ms)
             VALUES ('legacy','intervals_icu','athlete','Europe/Zurich',1,1);
             INSERT INTO workout_definitions(id,tpw_json,created_at,updated_at)
             VALUES ('cached','{}',1,1);
             INSERT INTO scheduled_workouts(id,workout_definition_id,scheduled_date_local,created_at_unix_ms,updated_at_unix_ms,provider_connection_id,external_event_id)
             VALUES ('scheduled','cached','2030-01-01',1,1,'legacy','event');",
        ).unwrap();
        let conn = database::prepare(conn).unwrap();
        assert!(database::provider_connections::get(&conn, "legacy")
            .unwrap()
            .is_none());
        assert!(workout_definitions::get(&conn, "cached").unwrap().is_none());
        assert!(scheduled_workouts::list_for_next_up(&conn, "2029-01-01")
            .unwrap()
            .is_empty());
        assert_eq!(
            pending_credentials(&conn).unwrap(),
            [(
                format!("{CREDENTIAL_CLEANUP_PREFIX}legacy"),
                "intervals_icu:legacy".into()
            )]
        );
    }
}
