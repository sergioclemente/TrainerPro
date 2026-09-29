//! Saved-device persistence.

use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedDevice {
    pub role: String,
    pub platform_id: String,
    pub name: String,
}

pub fn platform_id_for_role(conn: &Connection, role: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT platform_id FROM devices WHERE role = ?1",
        [role],
        |row| row.get(0),
    )
    .optional()
}

pub fn upsert(
    conn: &Connection,
    role: &str,
    platform_id: &str,
    name: &str,
    last_connected_at_ms: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO devices(role, platform_id, name, last_connected_at)
         VALUES(?1, ?2, ?3, ?4)
         ON CONFLICT(role) DO UPDATE SET platform_id = excluded.platform_id,
           name = excluded.name, last_connected_at = excluded.last_connected_at",
        rusqlite::params![role, platform_id, name, last_connected_at_ms],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, role: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM devices WHERE role = ?1", [role])?;
    Ok(())
}

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<SavedDevice>> {
    let mut stmt = conn.prepare("SELECT role, platform_id, name FROM devices")?;
    let rows = stmt.query_map([], |row| {
        Ok(SavedDevice {
            role: row.get(0)?,
            platform_id: row.get(1)?,
            name: row.get(2)?,
        })
    })?;
    rows.collect()
}

pub fn list_role_platform_ids(conn: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT role, platform_id FROM devices")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_source_and_saved_selection_round_trip_independently() {
        use crate::controller::ControllerSource;
        let conn = super::super::test_connection();
        assert_eq!(controller_source(&conn), ControllerSource::TrainerControls);
        upsert(&conn, "controller", "ride-id", "Ride", 1).unwrap();
        for source in [
            ControllerSource::PairedController,
            ControllerSource::Disabled,
            ControllerSource::TrainerControls,
        ] {
            save_controller_source(&conn, source).unwrap();
            assert_eq!(controller_source(&conn), source);
            assert_eq!(
                platform_id_for_role(&conn, "controller").unwrap(),
                Some("ride-id".into())
            );
        }
        save_controller_source(&conn, ControllerSource::PairedController).unwrap();
        assert_eq!(
            super::super::settings::get(&conn, "controller_source").unwrap(),
            Some("\"paired_controller\"".into())
        );
    }

    #[test]
    fn upsert_list_and_delete_saved_device() {
        let conn = super::super::test_connection();

        assert_eq!(platform_id_for_role(&conn, "trainer").unwrap(), None);
        upsert(&conn, "trainer", "first-id", "First", 10).unwrap();
        assert_eq!(
            platform_id_for_role(&conn, "trainer").unwrap().as_deref(),
            Some("first-id")
        );

        upsert(&conn, "trainer", "second-id", "Second", 20).unwrap();
        assert_eq!(
            list(&conn).unwrap(),
            vec![SavedDevice {
                role: "trainer".into(),
                platform_id: "second-id".into(),
                name: "Second".into(),
            }]
        );
        assert_eq!(
            list_role_platform_ids(&conn).unwrap(),
            vec![("trainer".into(), "second-id".into())]
        );

        delete(&conn, "trainer").unwrap();
        assert!(list(&conn).unwrap().is_empty());
    }
}

pub fn controller_source(conn: &Connection) -> crate::controller::ControllerSource {
    super::settings::get(conn, "controller_source")
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or(crate::controller::ControllerSource::TrainerControls)
}

pub fn save_controller_source(
    conn: &Connection,
    source: crate::controller::ControllerSource,
) -> rusqlite::Result<()> {
    super::settings::set(
        conn,
        "controller_source",
        &serde_json::to_string(&source).expect("enum serialization"),
    )
}
