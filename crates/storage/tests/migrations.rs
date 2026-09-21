//! Fresh-DB migrations cover all four §5.1 entities, re-opening an
//! already-migrated database is a no-op, and WAL mode is actually on
//! (DESIGN.md §11.1, Phase 4 gate).

use monitra_storage::SqliteStore;
use rusqlite::Connection;

fn table_names(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("prepare");
    stmt.query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(|row| row.expect("row"))
        .collect()
}

#[test]
fn fresh_database_gets_all_entity_tables() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("monitra.db");
    let _store = SqliteStore::open(&path).expect("open sqlite store");

    let conn = Connection::open(&path).expect("reopen for inspection");
    let tables = table_names(&conn);
    for expected in [
        "monitors",
        "check_results",
        "agents",
        "alert_events",
        "schema_migrations",
    ] {
        assert!(
            tables.iter().any(|t| t == expected),
            "expected table {expected} to exist, found {tables:?}"
        );
    }
}

#[test]
fn reopening_an_already_migrated_database_is_a_no_op() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("monitra.db");

    SqliteStore::open(&path).expect("first open runs migrations");
    // A second open against the same file must not error (idempotent
    // `schema_migrations` check) and must not duplicate tables.
    SqliteStore::open(&path).expect("second open is a no-op");

    let conn = Connection::open(&path).expect("reopen for inspection");
    let applied: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("count migrations");
    assert_eq!(applied, 7, "each migration version recorded exactly once");
}

#[test]
fn wal_mode_is_actually_on() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("monitra.db");
    let _store = SqliteStore::open(&path).expect("open sqlite store");

    let conn = Connection::open(&path).expect("reopen for inspection");
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("query journal_mode");
    assert_eq!(mode.to_lowercase(), "wal");
}
