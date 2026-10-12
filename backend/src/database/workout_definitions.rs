//! WorkoutSpec workout-definition persistence.

#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "provider persistence lands before the connection command that consumes it"
    )
)]

use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkoutDefinitionRow {
    pub id: String,
    pub tpw_json: String,
    pub origin: Option<String>,
}

pub struct NewWorkoutDefinition<'a> {
    pub id: &'a str,
    pub tpw_json: &'a str,
    pub created_at_ms: i64,
}

pub struct ProviderWorkoutDefinition<'a> {
    pub id: &'a str,
    pub tpw_json: &'a str,
    pub origin: &'a str,
    pub origin_id: Option<i64>,
    pub origin_ref: &'a str,
    pub synced_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteResult {
    Deleted,
    NotFound,
    ReferencedBySchedule,
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkoutDefinitionRow> {
    Ok(WorkoutDefinitionRow {
        id: row.get(0)?,
        tpw_json: row.get(1)?,
        origin: row.get(2)?,
    })
}

/// Refresh a catalog entry by source identity, independently of workout content.
/// An immediate transaction serializes concurrent lookup/create operations.
pub fn save_catalog_entry(
    conn: &mut Connection,
    definition: &ProviderWorkoutDefinition<'_>,
) -> rusqlite::Result<String> {
    let transaction = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let existing: Option<String> = transaction
        .query_row(
            "SELECT id FROM workout_definitions
         WHERE origin = ?1 AND origin_ref = ?2 AND retired_at_unix_ms IS NULL
           AND NOT EXISTS (
             SELECT 1 FROM scheduled_workouts sw
             WHERE sw.workout_definition_id = workout_definitions.id
               AND sw.provider_connection_id IS NOT NULL
           )
         ORDER BY created_at DESC, id ASC LIMIT 1",
            rusqlite::params![definition.origin, definition.origin_ref],
            |row| row.get(0),
        )
        .optional()?;
    let id = if let Some(id) = existing {
        update_provider_copy(
            &transaction,
            &ProviderWorkoutDefinition {
                id: &id,
                ..*definition
            },
        )?;
        id
    } else {
        insert_provider_copy(&transaction, definition)?;
        definition.id.to_owned()
    };
    transaction.commit()?;
    Ok(id)
}

pub fn insert(conn: &Connection, definition: &NewWorkoutDefinition<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO workout_definitions (id, tpw_json, created_at, updated_at)
         VALUES (?1,?2,?3,?3)",
        rusqlite::params![definition.id, definition.tpw_json, definition.created_at_ms,],
    )?;
    Ok(())
}

pub fn insert_provider_copy(
    conn: &Connection,
    definition: &ProviderWorkoutDefinition<'_>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO workout_definitions (
           id, tpw_json, created_at, updated_at, origin, origin_id, origin_ref)
         VALUES (?1,?2,?3,?3,?4,?5,?6)",
        rusqlite::params![
            definition.id,
            definition.tpw_json,
            definition.synced_at_unix_ms,
            definition.origin,
            definition.origin_id,
            definition.origin_ref,
        ],
    )?;
    Ok(())
}

pub fn update_provider_copy(
    conn: &Connection,
    definition: &ProviderWorkoutDefinition<'_>,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE workout_definitions
         SET tpw_json = ?1, updated_at = ?2, origin = ?3,
             origin_id = ?4, origin_ref = ?5, retired_at_unix_ms = NULL
         WHERE id = ?6",
        rusqlite::params![
            definition.tpw_json,
            definition.synced_at_unix_ms,
            definition.origin,
            definition.origin_id,
            definition.origin_ref,
            definition.id,
        ],
    )? > 0)
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<WorkoutDefinitionRow>> {
    conn.query_row(
        "SELECT id, tpw_json, origin FROM workout_definitions WHERE id = ?1",
        [id],
        read_row,
    )
    .optional()
}

/// List definitions that belong to the local Library. Provider-scheduled
/// definitions remain addressable by Next Up but are not Library entries.
pub fn list_local(conn: &Connection) -> rusqlite::Result<Vec<WorkoutDefinitionRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, tpw_json, origin FROM workout_definitions
         WHERE retired_at_unix_ms IS NULL
           AND NOT EXISTS (
             SELECT 1 FROM scheduled_workouts sw
             WHERE sw.workout_definition_id = workout_definitions.id
               AND sw.provider_connection_id IS NOT NULL
           )
         ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], read_row)?;
    rows.collect()
}

/// Definitions ranked by how often they produced an Activity on or after the
/// cutoff, then by the most recent matching Activity. The Next Up projection
/// decides how many to expose and removes definitions already scheduled.
pub fn list_by_activity_frequency_since(
    conn: &Connection,
    cutoff_unix_ms: i64,
) -> rusqlite::Result<Vec<WorkoutDefinitionRow>> {
    let mut stmt = conn.prepare(
        "SELECT wd.id, wd.tpw_json, wd.origin
         FROM workout_definitions wd
         JOIN activities a ON a.workout_definition_id = wd.id
         WHERE a.started_at_unix_ms >= ?1 AND wd.retired_at_unix_ms IS NULL
         GROUP BY wd.id, wd.tpw_json, wd.origin
         ORDER BY COUNT(a.id) DESC, MAX(a.started_at_unix_ms) DESC, wd.id",
    )?;
    let rows = stmt.query_map([cutoff_unix_ms], read_row)?;
    rows.collect()
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<DeleteResult> {
    let deleted = conn.execute(
        "DELETE FROM workout_definitions
         WHERE id = ?1
           AND NOT EXISTS (
             SELECT 1 FROM scheduled_workouts sw
             WHERE sw.workout_definition_id = workout_definitions.id
           )",
        [id],
    )? > 0;
    if deleted {
        return Ok(DeleteResult::Deleted);
    }

    let exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM workout_definitions WHERE id = ?1)",
        [id],
        |row| row.get(0),
    )?;
    Ok(if exists {
        DeleteResult::ReferencedBySchedule
    } else {
        DeleteResult::NotFound
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TPW_JSON: &str = r#"{"format":"TPW","version":1,"title":"Endurance","prescription":{"sport":"cycling","steps":[{"type":"steady","duration_seconds":3600,"power":{"type":"percent_ftp","percent":70}}]}}"#;

    fn insert_fixture(conn: &Connection, id: &str, created_at_ms: i64) {
        insert(
            conn,
            &NewWorkoutDefinition {
                id,
                tpw_json: &TPW_JSON.replace("Endurance", id),
                created_at_ms,
            },
        )
        .unwrap();
    }

    #[test]
    fn insert_list_update_and_delete_definition() {
        let conn = super::super::test_connection();
        insert_fixture(&conn, "older", 10);
        insert_fixture(&conn, "newer", 20);

        let older_json = TPW_JSON.replace("Endurance", "older");
        assert_eq!(
            list_local(&conn)
                .unwrap()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["newer", "older"]
        );

        update_provider_copy(
            &conn,
            &ProviderWorkoutDefinition {
                id: "older",
                tpw_json: &older_json,
                origin: "planner",
                origin_id: Some(42),
                origin_ref: "42",
                synced_at_unix_ms: 30,
            },
        )
        .unwrap();
        let row = get(&conn, "older").unwrap().unwrap();
        assert_eq!(row.tpw_json, older_json);
        assert_eq!(row.origin.as_deref(), Some("planner"));

        assert_eq!(delete(&conn, "older").unwrap(), DeleteResult::Deleted);
        assert_eq!(get(&conn, "older").unwrap(), None);
        assert_eq!(delete(&conn, "missing").unwrap(), DeleteResult::NotFound);
    }

    #[test]
    fn provider_scheduled_definitions_are_not_local_library_entries() {
        let mut conn = super::super::test_connection();
        let provider_json = TPW_JSON.replace("Endurance", "shared");
        insert_provider_copy(
            &conn,
            &ProviderWorkoutDefinition {
                id: "provider-definition",
                tpw_json: &provider_json,
                origin: "intervals_icu",
                origin_id: Some(42),
                origin_ref: "42",
                synced_at_unix_ms: 20,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO provider_connections (
               id, provider, external_account_id, time_zone,
               created_at_unix_ms, updated_at_unix_ms)
             VALUES ('connection','intervals_icu','athlete','Europe/Zurich',10,10)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scheduled_workouts (
               id, workout_definition_id, scheduled_date_local,
               created_at_unix_ms, updated_at_unix_ms,
               provider_connection_id, external_event_id)
             VALUES ('provider-schedule','provider-definition','2030-01-01',
                     20,20,'connection','42')",
            [],
        )
        .unwrap();

        assert!(list_local(&conn).unwrap().is_empty());
        assert_eq!(
            delete(&conn, "provider-definition").unwrap(),
            DeleteResult::ReferencedBySchedule
        );
        assert!(get(&conn, "provider-definition").unwrap().is_some());

        insert(
            &conn,
            &NewWorkoutDefinition {
                id: "local-copy",
                tpw_json: &provider_json,
                created_at_ms: 30,
            },
        )
        .unwrap();
        assert_eq!(
            list_local(&conn)
                .unwrap()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["local-copy"]
        );
        let catalog_id = save_catalog_entry(
            &mut conn,
            &catalog("catalog-copy", &provider_json, "intervals_icu", "42", 40),
        )
        .unwrap();
        assert_eq!(catalog_id, "catalog-copy");
        assert_eq!(
            get(&conn, "provider-definition").unwrap().unwrap().tpw_json,
            provider_json
        );
    }

    fn catalog<'a>(
        id: &'a str,
        json: &'a str,
        origin: &'a str,
        reference: &'a str,
        time: i64,
    ) -> ProviderWorkoutDefinition<'a> {
        ProviderWorkoutDefinition {
            id,
            tpw_json: json,
            origin,
            origin_id: None,
            origin_ref: reference,
            synced_at_unix_ms: time,
        }
    }

    #[test]
    fn identical_local_creations_remain_independent() {
        let conn = super::super::test_connection();
        for id in ["first", "second"] {
            insert(
                &conn,
                &NewWorkoutDefinition {
                    id,
                    tpw_json: TPW_JSON,
                    created_at_ms: 1,
                },
            )
            .unwrap();
        }
        assert_eq!(list_local(&conn).unwrap().len(), 2);
        assert_eq!(
            get(&conn, "first").unwrap().unwrap().tpw_json,
            get(&conn, "second").unwrap().unwrap().tpw_json
        );
    }

    #[test]
    fn catalog_refresh_preserves_identity_creation_time_and_activity_snapshot() {
        let mut conn = super::super::test_connection();
        let first = save_catalog_entry(&mut conn, &catalog("first", TPW_JSON, "planner", "42", 10))
            .unwrap();
        conn.execute("INSERT INTO activities (
            id, workout_definition_id, workout_name, started_at_unix_ms,
            elapsed_s, timer_s, ftp_used_w, final_intensity_multiplier,
            fit_path, journal_path, completed_pct, workout_definition_snapshot_json)
            VALUES ('ride',?1,'Original',10,60,60,250,1.0,'/synthetic.fit','/synthetic.jsonl',100,?2)",
            rusqlite::params![first, TPW_JSON]).unwrap();
        let changed = TPW_JSON.replace("Endurance", "Updated");
        let second =
            save_catalog_entry(&mut conn, &catalog("unused", &changed, "planner", "42", 20))
                .unwrap();
        assert_eq!(first, second);
        assert!(get(&conn, "unused").unwrap().is_none());
        assert_eq!(get(&conn, &first).unwrap().unwrap().tpw_json, changed);
        let times: (i64, i64) = conn
            .query_row(
                "SELECT created_at, updated_at FROM workout_definitions WHERE id = ?1",
                [&first],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(times, (10, 20));
        let snapshot: String = conn
            .query_row(
                "SELECT workout_definition_snapshot_json FROM activities WHERE id = 'ride'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(snapshot, TPW_JSON);
        assert_eq!(
            save_catalog_entry(
                &mut conn,
                &catalog("also-unused", &changed, "planner", "42", 30)
            )
            .unwrap(),
            first
        );
        assert_eq!(list_local(&conn).unwrap().len(), 1);
    }

    #[test]
    fn catalog_identity_does_not_merge_other_sources_local_or_retired_entries() {
        let mut conn = super::super::test_connection();
        insert(
            &conn,
            &NewWorkoutDefinition {
                id: "local",
                tpw_json: TPW_JSON,
                created_at_ms: 1,
            },
        )
        .unwrap();
        for (id, origin, reference) in [
            ("planner", "planner", "42"),
            ("other", "whatsonzwift", "42"),
            ("other-ref", "planner", "43"),
        ] {
            assert_eq!(
                save_catalog_entry(&mut conn, &catalog(id, TPW_JSON, origin, reference, 2))
                    .unwrap(),
                id
            );
        }
        conn.execute(
            "UPDATE workout_definitions SET retired_at_unix_ms = 3 WHERE id = 'planner'",
            [],
        )
        .unwrap();
        assert_eq!(
            save_catalog_entry(
                &mut conn,
                &catalog("replacement", TPW_JSON, "planner", "42", 4)
            )
            .unwrap(),
            "replacement"
        );
        assert_eq!(get(&conn, "local").unwrap().unwrap().origin, None);
        assert!(get(&conn, "planner").unwrap().is_some());
    }

    #[test]
    fn historical_catalog_duplicates_choose_newest_then_id_without_merging() {
        let mut conn = super::super::test_connection();
        for (id, time) in [("old", 1), ("b", 2), ("a", 2)] {
            insert_provider_copy(&conn, &catalog(id, TPW_JSON, "planner", "42", time)).unwrap();
        }
        assert_eq!(
            save_catalog_entry(&mut conn, &catalog("unused", TPW_JSON, "planner", "42", 3))
                .unwrap(),
            "a"
        );
        assert_eq!(list_local(&conn).unwrap().len(), 3);
    }

    #[test]
    fn concurrent_catalog_saves_reuse_one_entry() {
        use std::sync::{Arc, Barrier};
        use std::time::Duration;
        const LOCK_WAIT: Duration = Duration::from_secs(5);
        let directory =
            std::env::temp_dir().join(format!("trainerpro-catalog-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("test.sqlite");
        drop(super::super::open(&path).unwrap());
        let barrier = Arc::new(Barrier::new(2));
        let threads: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|id| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut conn = Connection::open(path).unwrap();
                    conn.busy_timeout(LOCK_WAIT).unwrap();
                    barrier.wait();
                    save_catalog_entry(&mut conn, &catalog(id, TPW_JSON, "planner", "42", 1))
                        .unwrap()
                })
            })
            .collect();
        let ids: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(ids[0], ids[1]);
        let conn = Connection::open(&path).unwrap();
        assert_eq!(list_local(&conn).unwrap().len(), 1);
        drop(conn);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn favorite_definitions_rank_recent_activity_and_exclude_stale_history() {
        let conn = super::super::test_connection();
        insert_fixture(&conn, "frequent", 10);
        insert_fixture(&conn, "recent", 20);
        insert_fixture(&conn, "stale", 30);
        insert_fixture(&conn, "unused", 40);

        for (id, definition_id, started_at_unix_ms) in [
            ("frequent-1", "frequent", 300),
            ("frequent-2", "frequent", 400),
            ("recent-1", "recent", 500),
            ("stale-1", "stale", 100),
            ("stale-2", "stale", 150),
            ("stale-3", "stale", 200),
        ] {
            conn.execute(
                "INSERT INTO activities (
                   id, workout_definition_id, workout_name,
                   started_at_unix_ms, elapsed_s, timer_s, ftp_used_w,
                   final_intensity_multiplier, fit_path, journal_path,
                   completed_pct)
                 VALUES (?1,?2,'Workout',?3,60,60,250,1.0,
                         '/tmp/activity.fit','/tmp/session.jsonl',100.0)",
                rusqlite::params![id, definition_id, started_at_unix_ms],
            )
            .unwrap();
        }

        assert_eq!(
            list_by_activity_frequency_since(&conn, 250)
                .unwrap()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["frequent", "recent"]
        );
    }
}
