//! Generic per-source cache persistence.

use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cached {
    pub value: String,
    pub content_hash: Option<String>,
    pub fetched_at_ms: i64,
}

pub fn get(conn: &Connection, source: &str, key: &str) -> rusqlite::Result<Option<Cached>> {
    conn.query_row(
        "SELECT value, content_hash, fetched_at FROM source_cache WHERE source=?1 AND key=?2",
        [source, key],
        |row| {
            Ok(Cached {
                value: row.get(0)?,
                content_hash: row.get(1)?,
                fetched_at_ms: row.get(2)?,
            })
        },
    )
    .optional()
}

pub fn put(
    conn: &Connection,
    source: &str,
    key: &str,
    content_hash: Option<&str>,
    value: &str,
    now_ms: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO source_cache(source, key, content_hash, value, fetched_at)
         VALUES(?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(source, key) DO UPDATE SET
           content_hash = excluded.content_hash,
           value = excluded.value,
           fetched_at = excluded.fetched_at",
        rusqlite::params![source, key, content_hash, value, now_ms],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_upsert_roundtrip() {
        let conn = super::super::test_connection();
        assert_eq!(get(&conn, "planner", "list").unwrap(), None);

        put(&conn, "planner", "list", Some("h1"), "[1]", 100).unwrap();
        let cached = get(&conn, "planner", "list").unwrap().unwrap();
        assert_eq!(
            (
                cached.value.as_str(),
                cached.content_hash.as_deref(),
                cached.fetched_at_ms
            ),
            ("[1]", Some("h1"), 100)
        );

        put(&conn, "planner", "list", Some("h2"), "[2]", 200).unwrap();
        let cached = get(&conn, "planner", "list").unwrap().unwrap();
        assert_eq!((cached.value.as_str(), cached.fetched_at_ms), ("[2]", 200));
        assert_eq!(get(&conn, "woz", "list").unwrap(), None);
    }
}
