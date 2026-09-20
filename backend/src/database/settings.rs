//! Settings persistence.

use rusqlite::{Connection, OptionalExtension};

pub fn get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .optional()
}

pub fn set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_and_upsert_setting() {
        let conn = super::super::test_connection();

        assert_eq!(get(&conn, "profile").unwrap(), None);
        set(&conn, "profile", "first").unwrap();
        assert_eq!(get(&conn, "profile").unwrap().as_deref(), Some("first"));
        set(&conn, "profile", "second").unwrap();
        assert_eq!(get(&conn, "profile").unwrap().as_deref(), Some("second"));
    }
}
