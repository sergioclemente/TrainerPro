//! SQLite schema + migrations. SPEC.md §8. Files are truth; the DB is the
//! queryable index.

use rusqlite::Connection;
use std::path::Path;

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
    // v5: legacy ride-upload marker (was intervals.icu, then Coach push).
    // The upload/push feature was removed to keep the app simple; the column is
    // retained (unread, unwritten) because dropping it buys nothing and a
    // SQLite column-drop migration is needless risk. Safe to reuse or drop later.
    "
    ALTER TABLE rides ADD COLUMN icu_activity_id TEXT;
    ",
];

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    let version: i64 =
        conn.query_row("SELECT user_version FROM pragma_user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        conn.execute_batch(sql)?;
        conn.pragma_update(None, "user_version", (i + 1) as i64)?;
    }
    Ok(conn)
}

/// Settings defaults, SPEC §8.
pub fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).ok()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}
