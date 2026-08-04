//! Generic per-source cache over SQLite (source_cache table, migration v4).
//! Sources cache JSON payloads keyed by (source, key) with an optional
//! content hash for invalidation and fetched_at for TTLs. Kilobyte-scale
//! data; no eviction.

use rusqlite::Connection;

pub struct Cached {
    pub value: String,
    pub content_hash: Option<String>,
    pub fetched_at_ms: i64,
}

pub fn get(conn: &Connection, source: &str, key: &str) -> Option<Cached> {
    conn.query_row(
        "SELECT value, content_hash, fetched_at FROM source_cache WHERE source=?1 AND key=?2",
        [source, key],
        |r| {
            Ok(Cached {
                value: r.get(0)?,
                content_hash: r.get(1)?,
                fetched_at_ms: r.get(2)?,
            })
        },
    )
    .ok()
}

pub fn put(
    conn: &Connection,
    source: &str,
    key: &str,
    content_hash: Option<&str>,
    value: &str,
    now_ms: i64,
) {
    let _ = conn.execute(
        "INSERT INTO source_cache(source, key, content_hash, value, fetched_at)
         VALUES(?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(source, key) DO UPDATE SET
           content_hash = excluded.content_hash,
           value = excluded.value,
           fetched_at = excluded.fetched_at",
        rusqlite::params![source, key, content_hash, value, now_ms],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE source_cache (
              source TEXT NOT NULL, key TEXT NOT NULL, content_hash TEXT,
              value TEXT NOT NULL, fetched_at INTEGER NOT NULL,
              PRIMARY KEY (source, key));",
        )
        .unwrap();
        conn
    }

    #[test]
    fn put_get_upsert_roundtrip() {
        let conn = mem_db();
        assert!(get(&conn, "planner", "list").is_none());
        put(&conn, "planner", "list", Some("h1"), "[1]", 100);
        let c = get(&conn, "planner", "list").unwrap();
        assert_eq!((c.value.as_str(), c.content_hash.as_deref(), c.fetched_at_ms), ("[1]", Some("h1"), 100));
        put(&conn, "planner", "list", Some("h2"), "[2]", 200);
        let c = get(&conn, "planner", "list").unwrap();
        assert_eq!((c.value.as_str(), c.fetched_at_ms), ("[2]", 200));
        assert!(get(&conn, "woz", "list").is_none(), "sources are namespaced");
    }
}
