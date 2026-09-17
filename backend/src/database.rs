//! SQLite ownership boundary: schema, migrations, and focused data access.

use rusqlite::Connection;
use std::path::Path;

pub mod activities;
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
    // v6: TPW becomes workout truth. Legacy workout rows are deliberately
    // not converted; activity history remains, detached from reset workouts.
    "
    CREATE TABLE workout_definitions (
      id TEXT PRIMARY KEY,
      tpw_json TEXT NOT NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL,
      origin TEXT,
      origin_id INTEGER,
      origin_ref TEXT
    );

    CREATE TABLE activities (
      id TEXT PRIMARY KEY,
      workout_definition_id TEXT REFERENCES workout_definitions(id) ON DELETE SET NULL,
      workout_name TEXT NOT NULL, started_at INTEGER NOT NULL,
      elapsed_s INTEGER NOT NULL, timer_s INTEGER NOT NULL,
      avg_power INTEGER, max_power INTEGER, np INTEGER, if_ REAL, tss REAL,
      avg_hr INTEGER, max_hr INTEGER, avg_cadence INTEGER, kj INTEGER,
      ftp_used INTEGER NOT NULL, intensity_final REAL NOT NULL,
      fit_path TEXT NOT NULL, journal_path TEXT NOT NULL, completed_pct REAL NOT NULL,
      icu_activity_id TEXT
    );

    INSERT INTO activities (
      id, workout_definition_id, workout_name, started_at, elapsed_s, timer_s,
      avg_power, max_power, np, if_, tss, avg_hr, max_hr, avg_cadence, kj,
      ftp_used, intensity_final, fit_path, journal_path, completed_pct,
      icu_activity_id
    )
    SELECT
      id, NULL, workout_name, started_at, elapsed_s, timer_s,
      avg_power, max_power, np, if_, tss, avg_hr, max_hr, avg_cadence, kj,
      ftp_used, intensity_final, fit_path, journal_path, completed_pct,
      icu_activity_id
    FROM rides;

    DROP TABLE rides;
    DROP TABLE workouts;
    ",
    // v7: an activity is the immutable result of a workout session. Existing
    // activities predate session identity and use their activity id as the
    // best available session id; they have no recoverable workout snapshot.
    "
    ALTER TABLE activities ADD COLUMN workout_session_id TEXT;
    ALTER TABLE activities ADD COLUMN workout_definition_snapshot_json TEXT;
    UPDATE activities SET workout_session_id = id;
    CREATE UNIQUE INDEX activities_workout_session_id
      ON activities(workout_session_id);
    ",
    // v8: persisted activity measurements name their meaning and units.
    "
    ALTER TABLE activities RENAME COLUMN started_at TO started_at_unix_ms;
    ALTER TABLE activities RENAME COLUMN avg_power TO average_power_w;
    ALTER TABLE activities RENAME COLUMN max_power TO max_power_w;
    ALTER TABLE activities RENAME COLUMN np TO normalized_power_w;
    ALTER TABLE activities RENAME COLUMN if_ TO intensity_factor;
    ALTER TABLE activities RENAME COLUMN tss TO training_stress_score;
    ALTER TABLE activities RENAME COLUMN avg_hr TO average_heart_rate_bpm;
    ALTER TABLE activities RENAME COLUMN max_hr TO max_heart_rate_bpm;
    ALTER TABLE activities RENAME COLUMN avg_cadence TO average_cadence_rpm;
    ALTER TABLE activities RENAME COLUMN kj TO work_kj;
    ALTER TABLE activities RENAME COLUMN ftp_used TO ftp_used_w;
    ALTER TABLE activities RENAME COLUMN intensity_final TO final_intensity_multiplier;
    ",
    // v9: a scheduled workout is a local calendar placement that references a
    // workout definition. It remains available for activity traceability when
    // removed from Next Up.
    "
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
      CHECK (
        (scheduled_time_local IS NULL AND scheduled_time_zone IS NULL) OR
        (scheduled_time_local IS NOT NULL AND scheduled_time_zone IS NOT NULL)
      )
    );

    CREATE INDEX scheduled_workouts_next_up
      ON scheduled_workouts(removed_at_unix_ms, scheduled_date_local,
                            scheduled_time_local);

    ALTER TABLE activities ADD COLUMN scheduled_workout_id TEXT
      REFERENCES scheduled_workouts(id) ON DELETE SET NULL;
    ",
    // v10: provider-scoped identity for connected calendar sync. Credentials
    // remain outside this table; it stores only non-secret account identity,
    // placement context, and observable sync status.
    "
    CREATE TABLE provider_connections (
      id TEXT PRIMARY KEY,
      provider TEXT NOT NULL CHECK(length(provider) > 0),
      external_account_id TEXT NOT NULL CHECK(length(external_account_id) > 0),
      display_name TEXT,
      time_zone TEXT NOT NULL CHECK(length(time_zone) > 0),
      last_sync_succeeded_at_unix_ms INTEGER,
      last_sync_error TEXT,
      created_at_unix_ms INTEGER NOT NULL,
      updated_at_unix_ms INTEGER NOT NULL,
      UNIQUE(provider, external_account_id)
    );

    ALTER TABLE workout_definitions ADD COLUMN retired_at_unix_ms INTEGER;

    ALTER TABLE scheduled_workouts ADD COLUMN provider_connection_id TEXT
      REFERENCES provider_connections(id) ON DELETE RESTRICT;
    ALTER TABLE scheduled_workouts ADD COLUMN external_event_id TEXT
      CHECK (
        (provider_connection_id IS NULL AND external_event_id IS NULL) OR
        (provider_connection_id IS NOT NULL AND external_event_id IS NOT NULL)
      );
    ALTER TABLE scheduled_workouts ADD COLUMN external_revision TEXT
      CHECK (external_revision IS NULL OR external_event_id IS NOT NULL);
    ALTER TABLE scheduled_workouts ADD COLUMN last_synced_at_unix_ms INTEGER
      CHECK (last_synced_at_unix_ms IS NULL OR external_event_id IS NOT NULL);

    CREATE UNIQUE INDEX scheduled_workouts_provider_event
      ON scheduled_workouts(provider_connection_id, external_event_id)
      WHERE provider_connection_id IS NOT NULL;
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
    use super::*;

    const PRE_PROVIDER_SYNC_MIGRATION_COUNT: usize = 9;

    #[test]
    fn tpw_migration_resets_workouts_but_preserves_activities_and_settings() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for (index, sql) in MIGRATIONS.iter().take(5).enumerate() {
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
               ftp_used, intensity_final, fit_path, journal_path, completed_pct
             ) VALUES ('activity', 'legacy', 'Legacy', 1, 60, 60, 200, 1.0,
                       '/tmp/activity.fit', '/tmp/activity.jsonl', 100.0)",
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
        let activity: (Option<String>, String, Option<String>, String, i64, String, String) = conn
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
        let setting: String = conn
            .query_row("SELECT value FROM settings WHERE key = 'profile'", [], |row| {
                row.get(0)
            })
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
    fn provider_sync_migration_preserves_existing_local_schedules() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for (index, sql) in MIGRATIONS
            .iter()
            .take(PRE_PROVIDER_SYNC_MIGRATION_COUNT)
            .enumerate()
        {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", (index + 1) as i64)
                .unwrap();
        }
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
}
