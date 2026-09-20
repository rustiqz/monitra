//! Embedded SQL migrations, applied at startup (DESIGN.md §8 — no separate
//! migration-tool crate). Each entry is `(version, sql)`; versions are
//! applied in order and recorded in `schema_migrations`, so re-opening an
//! already-migrated database is a no-op rather than an error.

use rusqlite::Connection;

use crate::error::StorageError;

/// One entry per schema change, in DESIGN.md §5.1 order (`Monitor`,
/// `CheckResult`, then the ADR-008/009 additions `Agent`, `AlertEvent`).
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_monitors",
        "CREATE TABLE monitors (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            target TEXT NOT NULL,
            kind TEXT NOT NULL,
            interval_secs INTEGER NOT NULL,
            status TEXT NOT NULL
        );",
    ),
    (
        "0002_check_results",
        "CREATE TABLE check_results (
            id INTEGER PRIMARY KEY,
            monitor_id INTEGER NOT NULL REFERENCES monitors(id) ON DELETE CASCADE,
            checked_at INTEGER NOT NULL,
            success INTEGER NOT NULL,
            latency_ms INTEGER NOT NULL,
            message TEXT
        );
        CREATE INDEX idx_check_results_monitor_checked_at
            ON check_results(monitor_id, checked_at);",
    ),
    (
        "0003_agents",
        "CREATE TABLE agents (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            last_heartbeat_at INTEGER NOT NULL,
            scope TEXT NOT NULL
        );",
    ),
    (
        "0004_alert_events",
        "CREATE TABLE alert_events (
            id INTEGER PRIMARY KEY,
            monitor_id INTEGER NOT NULL REFERENCES monitors(id) ON DELETE CASCADE,
            transitioned_to TEXT NOT NULL,
            occurred_at INTEGER NOT NULL,
            sinks_attempted TEXT NOT NULL,
            delivery_outcome TEXT NOT NULL
        );
        CREATE INDEX idx_alert_events_monitor ON alert_events(monitor_id);",
    ),
    (
        "0005_monitor_agent_id",
        "ALTER TABLE monitors ADD COLUMN agent_id INTEGER REFERENCES agents(id);",
    ),
    (
        "0006_agent_token",
        "ALTER TABLE agents ADD COLUMN token TEXT NOT NULL DEFAULT '';",
    ),
];

pub fn run(conn: &mut Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version TEXT PRIMARY KEY,
            applied_at INTEGER NOT NULL
        );",
    )
    .map_err(|source| StorageError::MigrationFailed {
        version: "schema_migrations",
        source,
    })?;

    for (version, sql) in MIGRATIONS {
        let already_applied: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
                [version],
                |row| row.get(0),
            )
            .map_err(|source| StorageError::MigrationFailed { version, source })?;
        if already_applied {
            continue;
        }

        let tx = conn
            .transaction()
            .map_err(|source| StorageError::MigrationFailed { version, source })?;
        tx.execute_batch(sql)
            .map_err(|source| StorageError::MigrationFailed { version, source })?;
        tx.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, unixepoch())",
            [version],
        )
        .map_err(|source| StorageError::MigrationFailed { version, source })?;
        tx.commit()
            .map_err(|source| StorageError::MigrationFailed { version, source })?;
    }

    Ok(())
}
