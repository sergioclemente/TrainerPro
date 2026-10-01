//! Confirmed Activity uploads, scoped to the owning Activity and provider account.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActivityUpload {
    pub connection_id: String,
    pub activity_id: String,
    pub remote_activity_id: String,
    pub uploaded_at_unix_ms: i64,
}

pub fn list(conn: &Connection, connection_id: &str) -> rusqlite::Result<Vec<ActivityUpload>> {
    conn.prepare("SELECT activity_id, remote_activity_id, uploaded_at_unix_ms FROM activity_uploads WHERE provider_connection_id = ?1")?
        .query_map([connection_id], |row| Ok(ActivityUpload {
            connection_id: connection_id.to_string(), activity_id: row.get(0)?, remote_activity_id: row.get(1)?, uploaded_at_unix_ms: row.get(2)?,
        }))?.collect()
}

pub fn get(
    conn: &Connection,
    connection_id: &str,
    activity_id: &str,
) -> rusqlite::Result<Option<ActivityUpload>> {
    conn.query_row("SELECT activity_id, remote_activity_id, uploaded_at_unix_ms FROM activity_uploads WHERE provider_connection_id = ?1 AND activity_id = ?2",
        params![connection_id, activity_id], |row| Ok(ActivityUpload {
            connection_id: connection_id.to_string(), activity_id: row.get(0)?, remote_activity_id: row.get(1)?, uploaded_at_unix_ms: row.get(2)?,
        })).optional()
}

pub fn record(conn: &Connection, upload: &ActivityUpload) -> rusqlite::Result<()> {
    conn.execute("INSERT INTO activity_uploads(activity_id, provider_connection_id, remote_activity_id, uploaded_at_unix_ms) VALUES (?1, ?2, ?3, ?4)",
        params![upload.activity_id, upload.connection_id, upload.remote_activity_id, upload.uploaded_at_unix_ms])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{activities, provider_connections, test_connection};

    #[test]
    fn receipts_survive_reconnect_are_account_scoped_and_follow_local_deletion() {
        let conn = test_connection();
        conn.execute("INSERT INTO activities(id, workout_name, started_at_unix_ms, elapsed_s, timer_s, ftp_used_w, final_intensity_multiplier, fit_path, journal_path, completed_pct) VALUES ('ride', 'Test ride', 1, 60, 60, 200, 1, '/missing.fit', '/missing.jsonl', 100)", []).unwrap();
        for (id, account) in [("first", "123"), ("second", "456")] {
            provider_connections::connect(
                &conn,
                &provider_connections::ConnectedProvider {
                    id,
                    provider: "garmin",
                    external_account_id: account,
                    display_name: None,
                    time_zone: None,
                    connected_at_unix_ms: 1,
                },
            )
            .unwrap();
            provider_connections::mark_disconnected(&conn, id, 2).unwrap();
        }
        let upload = ActivityUpload {
            connection_id: "first".into(),
            activity_id: "ride".into(),
            remote_activity_id: "789".into(),
            uploaded_at_unix_ms: 3,
        };
        record(&conn, &upload).unwrap();
        assert!(record(&conn, &upload).is_err());
        assert!(get(&conn, "second", "ride").unwrap().is_none());
        let reconnected = provider_connections::connect(
            &conn,
            &provider_connections::ConnectedProvider {
                id: "new-id",
                provider: "garmin",
                external_account_id: "123",
                display_name: None,
                time_zone: None,
                connected_at_unix_ms: 4,
            },
        )
        .unwrap();
        assert_eq!(reconnected, "first");
        assert_eq!(list(&conn, &reconnected).unwrap(), vec![upload]);
        assert_eq!(
            activities::delete(&conn, "ride").unwrap().unwrap().fit_path,
            "/missing.fit"
        );
        assert!(list(&conn, &reconnected).unwrap().is_empty());
    }
}
