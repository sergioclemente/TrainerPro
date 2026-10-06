//! SQLite ownership boundary: schema, migrations, and focused data access.

use rusqlite::Connection;
use std::path::Path;

pub mod activities;
pub mod activity_uploads;
pub mod devices;
pub mod provider_connections;
pub mod scheduled_workouts;
pub mod settings;
pub mod source_cache;
pub mod workout_definitions;

const MIGRATIONS: &[&str] = &[
    // v1
    "
    CREATE TABLE workouts (
      id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
      source_format TEXT NOT NULL, file_path TEXT NOT NULL, sha256 TEXT NOT NULL UNIQUE,
      duration_s INTEGER NOT NULL, est_if REAL, est_tss REAL,
      graph_json TEXT NOT NULL, imported_at INTEGER NOT NULL);

    CREATE TABLE rides (
      id TEXT PRIMARY KEY, workout_id TEXT REFERENCES workouts(id) ON DELETE SET NULL,
      workout_name TEXT NOT NULL, started_at INTEGER NOT NULL,
      elapsed_s INTEGER NOT NULL, timer_s INTEGER NOT NULL,
      avg_power INTEGER, max_power INTEGER, np INTEGER, if_ REAL, tss REAL,
      avg_hr INTEGER, max_hr INTEGER, avg_cadence INTEGER, kj INTEGER,
      ftp_used INTEGER NOT NULL, intensity_final REAL NOT NULL,
      fit_path TEXT NOT NULL, journal_path TEXT NOT NULL, completed_pct REAL NOT NULL);

    CREATE TABLE devices (
      role TEXT PRIMARY KEY CHECK(role IN ('trainer','hrm')),
      platform_id TEXT NOT NULL, name TEXT NOT NULL, last_connected_at INTEGER);

    CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    ",
    // v2: provenance for workouts imported from integrations (WorkoutPlanner)
    "
    ALTER TABLE workouts ADD COLUMN origin TEXT;
    ALTER TABLE workouts ADD COLUMN origin_id INTEGER;
    ",
    // v3: string provenance ref for sources without numeric ids (whatsonzwift)
    "
    ALTER TABLE workouts ADD COLUMN origin_ref TEXT;
    ",
    // v4: generic per-source cache (lists, catalogs, previews) for instant
    // startup + offline use
    "
    CREATE TABLE source_cache (
      source TEXT NOT NULL,
      key TEXT NOT NULL,
      content_hash TEXT,
      value TEXT NOT NULL,
      fetched_at INTEGER NOT NULL,
      PRIMARY KEY (source, key)
    );
    ",
    // v5: reserved marker for the planned intervals.icu post-ride upload.
    // It remains unread and unwritten until that integration is implemented;
    // retaining it also avoids rewriting migration history.
    "
    ALTER TABLE rides ADD COLUMN icu_activity_id TEXT;
    ",
    // v6: TPW definitions, session Activities, schedules, provider connections,
    // upload receipts, and controller pairing. Legacy workouts are reset;
    // recorded rides retain their measurements and artifacts. Their IDs are
    // the best available session IDs; workout snapshots cannot be recovered.
    "
    CREATE TABLE workout_definitions (
      id TEXT PRIMARY KEY,
      tpw_json TEXT NOT NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL,
      origin TEXT,
      origin_id INTEGER,
      origin_ref TEXT,
      retired_at_unix_ms INTEGER
    );

    CREATE TABLE provider_connections (
      id TEXT PRIMARY KEY,
      provider TEXT NOT NULL CHECK(length(provider) > 0),
      external_account_id TEXT NOT NULL CHECK(length(external_account_id) > 0),
      display_name TEXT,
      time_zone TEXT CHECK(time_zone IS NULL OR length(time_zone) > 0),
      last_sync_succeeded_at_unix_ms INTEGER,
      last_sync_error TEXT,
      created_at_unix_ms INTEGER NOT NULL,
      updated_at_unix_ms INTEGER NOT NULL,
      disconnected_at_unix_ms INTEGER,
      UNIQUE(provider, external_account_id)
    );

    CREATE UNIQUE INDEX provider_connections_one_active_account
      ON provider_connections(provider)
      WHERE disconnected_at_unix_ms IS NULL;

    CREATE TABLE scheduled_workouts (
      id TEXT PRIMARY KEY,
      workout_definition_id TEXT NOT NULL
        REFERENCES workout_definitions(id) ON DELETE RESTRICT,
      scheduled_date_local TEXT NOT NULL,
      scheduled_time_local TEXT,
      scheduled_time_zone TEXT,
      removed_at_unix_ms INTEGER,
      created_at_unix_ms INTEGER NOT NULL,
      updated_at_unix_ms INTEGER NOT NULL,
      provider_connection_id TEXT REFERENCES provider_connections(id) ON DELETE RESTRICT,
      external_event_id TEXT,
      external_revision TEXT,
      last_synced_at_unix_ms INTEGER,
      CHECK (
        (scheduled_time_local IS NULL AND scheduled_time_zone IS NULL) OR
        (scheduled_time_local IS NOT NULL AND scheduled_time_zone IS NOT NULL)
      ),
      CHECK (
        (provider_connection_id IS NULL AND external_event_id IS NULL) OR
        (provider_connection_id IS NOT NULL AND external_event_id IS NOT NULL)
      ),
      CHECK (external_revision IS NULL OR external_event_id IS NOT NULL),
      CHECK (last_synced_at_unix_ms IS NULL OR external_event_id IS NOT NULL)
    );

    CREATE INDEX scheduled_workouts_next_up
      ON scheduled_workouts(removed_at_unix_ms, scheduled_date_local,
                            scheduled_time_local);
    CREATE UNIQUE INDEX scheduled_workouts_provider_event
      ON scheduled_workouts(provider_connection_id, external_event_id)
      WHERE provider_connection_id IS NOT NULL;

    CREATE TABLE activities (
      id TEXT PRIMARY KEY,
      workout_definition_id TEXT REFERENCES workout_definitions(id) ON DELETE SET NULL,
      workout_name TEXT NOT NULL, started_at_unix_ms INTEGER NOT NULL,
      elapsed_s INTEGER NOT NULL, timer_s INTEGER NOT NULL,
      average_power_w INTEGER, max_power_w INTEGER, normalized_power_w INTEGER,
      intensity_factor REAL, training_stress_score REAL,
      average_heart_rate_bpm INTEGER, max_heart_rate_bpm INTEGER,
      average_cadence_rpm INTEGER, work_kj INTEGER,
      ftp_used_w INTEGER NOT NULL, final_intensity_multiplier REAL NOT NULL,
      fit_path TEXT NOT NULL, journal_path TEXT NOT NULL, completed_pct REAL NOT NULL,
      icu_activity_id TEXT,
      workout_session_id TEXT,
      workout_definition_snapshot_json TEXT,
      scheduled_workout_id TEXT REFERENCES scheduled_workouts(id) ON DELETE SET NULL
    );

    INSERT INTO activities (
      id, workout_definition_id, workout_name, started_at_unix_ms, elapsed_s, timer_s,
      average_power_w, max_power_w, normalized_power_w, intensity_factor,
      training_stress_score, average_heart_rate_bpm, max_heart_rate_bpm,
      average_cadence_rpm, work_kj, ftp_used_w, final_intensity_multiplier,
      fit_path, journal_path, completed_pct, icu_activity_id, workout_session_id
    )
    SELECT
      id, NULL, workout_name, started_at, elapsed_s, timer_s,
      avg_power, max_power, np, if_, tss, avg_hr, max_hr, avg_cadence, kj,
      ftp_used, intensity_final, fit_path, journal_path, completed_pct,
      icu_activity_id, id
    FROM rides;

    CREATE UNIQUE INDEX activities_workout_session_id
      ON activities(workout_session_id);

    DROP TABLE rides;
    DROP TABLE workouts;

    CREATE TABLE activity_uploads (
      activity_id TEXT NOT NULL REFERENCES activities(id) ON DELETE CASCADE,
      provider_connection_id TEXT NOT NULL REFERENCES provider_connections(id) ON DELETE RESTRICT,
      remote_activity_id TEXT NOT NULL CHECK(length(remote_activity_id) > 0),
      uploaded_at_unix_ms INTEGER NOT NULL,
      PRIMARY KEY(activity_id, provider_connection_id)
    );

    CREATE TABLE devices_with_controller (
      role TEXT PRIMARY KEY CHECK(role IN ('trainer','hrm','controller')),
      platform_id TEXT NOT NULL, name TEXT NOT NULL, last_connected_at INTEGER);
    INSERT INTO devices_with_controller(role, platform_id, name, last_connected_at)
      SELECT role, platform_id, name, last_connected_at FROM devices;
    DROP TABLE devices;
    ALTER TABLE devices_with_controller RENAME TO devices;
    ",
];

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    prepare(conn)
}

fn prepare(mut conn: Connection) -> rusqlite::Result<Connection> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    let version: i64 = conn.query_row("SELECT user_version FROM pragma_user_version", [], |r| {
        r.get(0)
    })?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let transaction = conn.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.pragma_update(None, "user_version", (i + 1) as i64)?;
        transaction.commit()?;
    }
    Ok(conn)
}

#[cfg(test)]
pub(super) fn test_connection() -> Connection {
    prepare(Connection::open_in_memory().expect("open in-memory database"))
        .expect("prepare in-memory database")
}

#[cfg(test)]
mod tests {
    #[test]
    fn activity_upload_receipts_enforce_identity_and_foreign_keys() {
        let conn = test_connection();
        conn.execute_batch("INSERT INTO provider_connections(id, provider, external_account_id, created_at_unix_ms, updated_at_unix_ms) VALUES ('account', 'garmin', '123', 1, 1);
            INSERT INTO activities(id, workout_name, started_at_unix_ms, elapsed_s, timer_s, ftp_used_w, final_intensity_multiplier, fit_path, journal_path, completed_pct) VALUES ('ride', 'Test', 1, 30, 30, 200, 1, '/missing.fit', '/missing.jsonl', 100);
            INSERT INTO activity_uploads VALUES ('ride', 'account', '456', 2);").unwrap();
        let conn = prepare(conn).unwrap();
        let receipt = activity_uploads::get(&conn, "account", "ride")
            .unwrap()
            .unwrap();
        assert_eq!(receipt.remote_activity_id, "456");
        assert_eq!(receipt.connection_id, "account");
        assert_eq!(receipt.uploaded_at_unix_ms, 2);
        assert!(activity_uploads::record(&conn, &receipt).is_err());
        assert!(conn
            .execute("DELETE FROM provider_connections WHERE id='account'", [])
            .is_err());
        assert!(conn.prepare("SELECT * FROM garmin_uploads").is_err());
        activities::delete(&conn, "ride").unwrap();
        assert!(activity_uploads::list(&conn, "account").unwrap().is_empty());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
    }

    #[test]
    fn providers_support_schedules_and_accounts_without_time_zones() {
        let conn = test_connection();
        conn.execute_batch("INSERT INTO provider_connections(id, provider, external_account_id, time_zone, created_at_unix_ms, updated_at_unix_ms) VALUES ('icu', 'intervals_icu', 'i123', 'Europe/Zurich', 1, 1);
            INSERT INTO workout_definitions(id, tpw_json, created_at, updated_at) VALUES ('definition', '{}', 1, 1);
            INSERT INTO scheduled_workouts(id, workout_definition_id, scheduled_date_local, created_at_unix_ms, updated_at_unix_ms, provider_connection_id, external_event_id) VALUES ('scheduled', 'definition', '2030-01-01', 1, 1, 'icu', 'event');").unwrap();
        let conn = super::prepare(conn).unwrap();
        let connection = super::provider_connections::get(&conn, "icu")
            .unwrap()
            .unwrap();
        assert_eq!(connection.time_zone.as_deref(), Some("Europe/Zurich"));
        let linked: String = conn
            .query_row(
                "SELECT provider_connection_id FROM scheduled_workouts WHERE id = 'scheduled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(linked, "icu");
        assert_eq!(
            conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
        super::provider_connections::connect(
            &conn,
            &super::provider_connections::ConnectedProvider {
                id: "garmin",
                provider: "garmin",
                external_account_id: "123",
                display_name: None,
                time_zone: None,
                connected_at_unix_ms: 1,
            },
        )
        .unwrap();
        assert!(super::provider_connections::get(&conn, "garmin")
            .unwrap()
            .unwrap()
            .time_zone
            .is_none());
    }

    #[test]
    fn controller_migration_preserves_pairings_and_accepts_controller() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for (index, sql) in super::MIGRATIONS
            .iter()
            .take(RELEASED_MIGRATION_COUNT)
            .enumerate()
        {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", (index + 1) as i64)
                .unwrap();
        }
        super::devices::upsert(&conn, "trainer", "bike", "My bike", 1).unwrap();
        super::devices::upsert(&conn, "hrm", "strap", "My HRM", 2).unwrap();
        let conn = super::prepare(conn).unwrap();
        super::devices::upsert(&conn, "controller", "ride", "My Ride", 3).unwrap();
        assert_eq!(super::devices::list(&conn).unwrap().len(), 3);
        assert_eq!(
            super::devices::platform_id_for_role(&conn, "trainer").unwrap(),
            Some("bike".into())
        );
        assert_eq!(
            super::devices::platform_id_for_role(&conn, "hrm").unwrap(),
            Some("strap".into())
        );
        let connected_at: i64 = conn
            .query_row(
                "SELECT last_connected_at FROM devices WHERE role = 'trainer'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(connected_at, 1);
        assert!(super::devices::upsert(&conn, "unknown", "id", "name", 4).is_err());
    }

    use super::*;

    const RELEASED_MIGRATION_COUNT: usize = 5;

    #[test]
    fn tpw_migration_resets_workouts_but_preserves_activities_and_settings() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for (index, sql) in MIGRATIONS.iter().take(RELEASED_MIGRATION_COUNT).enumerate() {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", (index + 1) as i64)
                .unwrap();
        }
        conn.execute(
            "INSERT INTO workouts (
               id, name, description, source_format, file_path, sha256,
               duration_s, est_if, est_tss, graph_json, imported_at
             ) VALUES ('legacy', 'Legacy', '', 'zwo', '/tmp/legacy.zwo',
                       'hash', 60, 0.7, 1.0, '[]', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO rides (
               id, workout_id, workout_name, started_at, elapsed_s, timer_s,
               ftp_used, intensity_final, fit_path, journal_path, completed_pct,
               avg_power, max_power, np, if_, tss, avg_hr, max_hr, avg_cadence, kj,
               icu_activity_id
             ) VALUES ('activity', 'legacy', 'Legacy', 1, 60, 60, 200, 1.0,
                       '/tmp/activity.fit', '/tmp/activity.jsonl', 100.0,
                       180, 300, 195, 0.975, 1.5, 130, 150, 90, 11, 'remote-ride')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('profile', 'saved')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO devices (role, platform_id, name)
             VALUES ('trainer', 'trainer-id', 'Saved trainer')",
            [],
        )
        .unwrap();

        let conn = prepare(conn).unwrap();

        let definitions: i64 = conn
            .query_row("SELECT count(*) FROM workout_definitions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(definitions, 0);
        let activity: (
            Option<String>,
            String,
            Option<String>,
            String,
            i64,
            String,
            String,
        ) = conn
            .query_row(
                "SELECT workout_definition_id, workout_session_id,
                        workout_definition_snapshot_json, workout_name,
                        started_at_unix_ms, fit_path, journal_path
                 FROM activities WHERE id = 'activity'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            activity,
            (
                None,
                "activity".into(),
                None,
                "Legacy".into(),
                1,
                "/tmp/activity.fit".into(),
                "/tmp/activity.jsonl".into(),
            )
        );
        let preserved_measurements: bool = conn
            .query_row(
                "SELECT average_power_w = 180 AND max_power_w = 300
                    AND normalized_power_w = 195 AND intensity_factor = 0.975
                    AND training_stress_score = 1.5 AND average_heart_rate_bpm = 130
                    AND max_heart_rate_bpm = 150 AND average_cadence_rpm = 90
                    AND work_kj = 11 AND ftp_used_w = 200
                    AND final_intensity_multiplier = 1.0 AND elapsed_s = 60
                    AND timer_s = 60 AND completed_pct = 100.0
                    AND icu_activity_id = 'remote-ride'
                 FROM activities WHERE id = 'activity'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(preserved_measurements);
        let setting: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'profile'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(setting, "saved");
        let device: String = conn
            .query_row(
                "SELECT name FROM devices WHERE role = 'trainer'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(device, "Saved trainer");
        let legacy_ride_tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_schema
                 WHERE type = 'table' AND name = 'rides'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy_ride_tables, 0);
        assert_eq!(
            conn.query_row("SELECT user_version FROM pragma_user_version", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            MIGRATIONS.len() as i64
        );
    }

    #[test]
    fn local_schedules_do_not_require_a_provider() {
        let conn = test_connection();
        conn.execute(
            "INSERT INTO workout_definitions (
               id, tpw_json, created_at, updated_at)
             VALUES ('definition','{}',1,1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scheduled_workouts (
               id, workout_definition_id, scheduled_date_local,
               created_at_unix_ms, updated_at_unix_ms)
             VALUES ('schedule','definition','2030-01-01',1,1)",
            [],
        )
        .unwrap();

        let conn = prepare(conn).unwrap();
        let migrated: (Option<String>, Option<String>, Option<i64>) = conn
            .query_row(
                "SELECT provider_connection_id, external_event_id,
                        last_synced_at_unix_ms
                 FROM scheduled_workouts WHERE id = 'schedule'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(migrated, (None, None, None));
        let retired_at: Option<i64> = conn
            .query_row(
                "SELECT retired_at_unix_ms FROM workout_definitions
                 WHERE id = 'definition'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retired_at, None);
    }

    #[test]
    fn provider_connections_allow_only_one_active_account() {
        let conn = test_connection();
        conn.execute(
            "INSERT INTO provider_connections (
               id, provider, external_account_id, time_zone,
               created_at_unix_ms, updated_at_unix_ms)
             VALUES ('connection','intervals_icu','i123','Europe/Zurich',1,1)",
            [],
        )
        .unwrap();

        let conn = prepare(conn).unwrap();
        let disconnected_at_unix_ms: Option<i64> = conn
            .query_row(
                "SELECT disconnected_at_unix_ms FROM provider_connections
                 WHERE id = 'connection'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(disconnected_at_unix_ms, None);
        assert!(conn
            .execute(
                "INSERT INTO provider_connections (
                   id, provider, external_account_id, time_zone,
                   created_at_unix_ms, updated_at_unix_ms)
                 VALUES ('second','intervals_icu','i456','Europe/Zurich',2,2)",
                [],
            )
            .is_err());
    }
}
