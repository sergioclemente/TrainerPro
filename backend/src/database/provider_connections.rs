//! Non-secret identity and sync status for connected providers.

use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConnectionRow {
    pub id: String,
    pub provider: String,
    pub external_account_id: String,
    pub display_name: Option<String>,
    pub time_zone: String,
    pub last_sync_succeeded_at_unix_ms: Option<i64>,
    pub last_sync_error: Option<String>,
    pub disconnected_at_unix_ms: Option<i64>,
}

pub struct ConnectedProvider<'a> {
    pub id: &'a str,
    pub provider: &'a str,
    pub external_account_id: &'a str,
    pub display_name: Option<&'a str>,
    pub time_zone: &'a str,
    pub connected_at_unix_ms: i64,
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderConnectionRow> {
    Ok(ProviderConnectionRow {
        id: row.get(0)?,
        provider: row.get(1)?,
        external_account_id: row.get(2)?,
        display_name: row.get(3)?,
        time_zone: row.get(4)?,
        last_sync_succeeded_at_unix_ms: row.get(5)?,
        last_sync_error: row.get(6)?,
        disconnected_at_unix_ms: row.get(7)?,
    })
}

/// Create a connection or refresh its non-secret account metadata. Reconnecting
/// the same provider account preserves its local connection identity.
pub fn connect(conn: &Connection, provider: &ConnectedProvider<'_>) -> rusqlite::Result<String> {
    conn.query_row(
        "INSERT INTO provider_connections (
           id, provider, external_account_id, display_name, time_zone,
           created_at_unix_ms, updated_at_unix_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?6)
         ON CONFLICT(provider, external_account_id) DO UPDATE SET
           display_name = excluded.display_name,
           time_zone = excluded.time_zone,
           disconnected_at_unix_ms = NULL,
           updated_at_unix_ms = excluded.updated_at_unix_ms
         RETURNING id",
        rusqlite::params![
            provider.id,
            provider.provider,
            provider.external_account_id,
            provider.display_name,
            provider.time_zone,
            provider.connected_at_unix_ms,
        ],
        |row| row.get(0),
    )
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<ProviderConnectionRow>> {
    conn.query_row(
        "SELECT id, provider, external_account_id, display_name, time_zone,
                last_sync_succeeded_at_unix_ms, last_sync_error,
                disconnected_at_unix_ms
         FROM provider_connections WHERE id = ?1",
        [id],
        read_row,
    )
    .optional()
}

pub fn get_by_provider_account(
    conn: &Connection,
    provider: &str,
    external_account_id: &str,
) -> rusqlite::Result<Option<ProviderConnectionRow>> {
    conn.query_row(
        "SELECT id, provider, external_account_id, display_name, time_zone,
                last_sync_succeeded_at_unix_ms, last_sync_error,
                disconnected_at_unix_ms
         FROM provider_connections
         WHERE provider = ?1 AND external_account_id = ?2",
        rusqlite::params![provider, external_account_id],
        read_row,
    )
    .optional()
}

pub fn get_active_by_provider(
    conn: &Connection,
    provider: &str,
) -> rusqlite::Result<Option<ProviderConnectionRow>> {
    conn.query_row(
        "SELECT id, provider, external_account_id, display_name, time_zone,
                last_sync_succeeded_at_unix_ms, last_sync_error,
                disconnected_at_unix_ms
         FROM provider_connections
         WHERE provider = ?1 AND disconnected_at_unix_ms IS NULL",
        [provider],
        read_row,
    )
    .optional()
}

pub fn mark_disconnected(
    conn: &Connection,
    id: &str,
    disconnected_at_unix_ms: i64,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE provider_connections
         SET disconnected_at_unix_ms = ?1, updated_at_unix_ms = ?1
         WHERE id = ?2 AND disconnected_at_unix_ms IS NULL",
        rusqlite::params![disconnected_at_unix_ms, id],
    )? > 0)
}

pub fn record_sync_success(
    conn: &Connection,
    id: &str,
    succeeded_at_unix_ms: i64,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE provider_connections
         SET last_sync_succeeded_at_unix_ms = ?1, last_sync_error = NULL,
             updated_at_unix_ms = ?1
         WHERE id = ?2",
        rusqlite::params![succeeded_at_unix_ms, id],
    )? > 0)
}

pub fn record_sync_failure(
    conn: &Connection,
    id: &str,
    message: &str,
    failed_at_unix_ms: i64,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE provider_connections
         SET last_sync_error = ?1, updated_at_unix_ms = ?2
         WHERE id = ?3",
        rusqlite::params![message, failed_at_unix_ms, id],
    )? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection<'a>(id: &'a str, name: Option<&'a str>, at: i64) -> ConnectedProvider<'a> {
        ConnectedProvider {
            id,
            provider: "intervals_icu",
            external_account_id: "i123",
            display_name: name,
            time_zone: "Europe/Zurich",
            connected_at_unix_ms: at,
        }
    }

    #[test]
    fn reconnect_preserves_identity_and_refreshes_metadata() {
        let conn = super::super::test_connection();
        assert_eq!(
            connect(&conn, &connection("first", Some("Ada"), 10)).unwrap(),
            "first"
        );
        assert_eq!(
            connect(&conn, &connection("replacement", Some("A. Rider"), 20)).unwrap(),
            "first"
        );

        let row = get(&conn, "first").unwrap().unwrap();
        assert_eq!(row.external_account_id, "i123");
        assert_eq!(row.display_name.as_deref(), Some("A. Rider"));
        assert_eq!(row.time_zone, "Europe/Zurich");
        assert_eq!(row.disconnected_at_unix_ms, None);
        assert_eq!(get(&conn, "replacement").unwrap(), None);
    }

    #[test]
    fn sync_outcome_is_observable_and_success_clears_failure() {
        let conn = super::super::test_connection();
        connect(&conn, &connection("connection", None, 10)).unwrap();

        assert!(record_sync_failure(&conn, "connection", "network unavailable", 20).unwrap());
        assert_eq!(
            get(&conn, "connection")
                .unwrap()
                .unwrap()
                .last_sync_error
                .as_deref(),
            Some("network unavailable")
        );

        assert!(record_sync_success(&conn, "connection", 30).unwrap());
        let row = get(&conn, "connection").unwrap().unwrap();
        assert_eq!(row.last_sync_succeeded_at_unix_ms, Some(30));
        assert_eq!(row.last_sync_error, None);
    }

    #[test]
    fn disconnected_account_can_be_reconnected_with_the_same_identity() {
        let conn = super::super::test_connection();
        connect(&conn, &connection("connection", Some("Ada"), 10)).unwrap();
        assert!(mark_disconnected(&conn, "connection", 20).unwrap());
        assert_eq!(
            get_active_by_provider(&conn, "intervals_icu").unwrap(),
            None
        );

        assert_eq!(
            connect(&conn, &connection("replacement", Some("Ada Rider"), 30)).unwrap(),
            "connection"
        );
        let active = get_active_by_provider(&conn, "intervals_icu")
            .unwrap()
            .unwrap();
        assert_eq!(active.id, "connection");
        assert_eq!(active.display_name.as_deref(), Some("Ada Rider"));
        assert_eq!(active.disconnected_at_unix_ms, None);
    }
}
