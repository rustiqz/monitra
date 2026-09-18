//! Default `Store` provider: SQLite (DESIGN.md §4 `storage`).
//!
//! Owns the SQLite implementation of the `Store` trait — schema, migrations,
//! connection lifecycle, all SQL, retention/pruning. The always-compiled
//! default; nothing outside this crate constructs raw queries or sees a
//! `StorageError`/`rusqlite::Error` (they're converted to `ProviderError` at
//! the trait boundary — see `error.rs`).
//!
//! A single connection behind a `std::sync::Mutex`, driven through
//! `tokio::task::spawn_blocking`. Not a placeholder for a future pool:
//! `engine`'s writer task (§6.2, Phase 6) is the one place that batches
//! writes, so there is exactly one logical writer regardless of how many
//! storage-side connections exist — a pool would add surface this phase's
//! gate doesn't need (P5, don't optimise ahead of a benchmark, §11.1).

mod codec;
mod error;
mod migrations;

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use monitra_provider::{ProviderError, Store};
use rusqlite::{Connection, OptionalExtension, Row, params};

use error::StorageError;
use monitra_models::{Agent, AlertEvent, CheckResult, Monitor, MonitorStatus};

/// The default `Store` provider. Construct with [`SqliteStore::open`].
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// Opens (creating if absent) the SQLite database at `path`, enables
    /// WAL and foreign keys, and runs any pending migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let display = path.as_ref().display().to_string();
        let open_err = |source: rusqlite::Error| StorageError::Open {
            path: display.clone(),
            source,
        };

        let mut conn = Connection::open(path.as_ref()).map_err(open_err)?;

        // `journal_mode` returns the resulting mode as a row rather than
        // being a fire-and-forget pragma, so it's queried, not executed.
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(open_err)?;
        tracing::debug!(journal_mode = %journal_mode, "storage: opened sqlite database");

        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(open_err)?;

        migrations::run(&mut conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Runs `f` against the shared connection on a blocking-pool thread,
    /// converting any `StorageError` into the `ProviderError` the `Store`
    /// trait's callers expect.
    async fn run_blocking<T, F>(&self, f: F) -> Result<T, ProviderError>
    where
        F: FnOnce(&mut Connection) -> Result<T, StorageError> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        let result = tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().unwrap_or_else(PoisonError::into_inner);
            f(&mut guard)
        })
        .await
        .map_err(StorageError::TaskJoin)?;
        result.map_err(ProviderError::from)
    }
}

/// Runs `sql`, mapping every returned row through `map_row`.
fn query_all<T>(
    conn: &Connection,
    sql: &str,
    query_params: impl rusqlite::Params,
    map_row: fn(&Row) -> Result<T, StorageError>,
) -> Result<Vec<T>, StorageError> {
    let mut stmt = conn.prepare(sql).map_err(codec::query_failed)?;
    let rows = stmt
        .query_map(query_params, |row| Ok(map_row(row)))
        .map_err(codec::query_failed)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(codec::query_failed)??);
    }
    Ok(out)
}

/// Runs `sql`, mapping at most one returned row through `map_row`.
fn query_optional<T>(
    conn: &Connection,
    sql: &str,
    query_params: impl rusqlite::Params,
    map_row: fn(&Row) -> Result<T, StorageError>,
) -> Result<Option<T>, StorageError> {
    conn.query_row(sql, query_params, |row| Ok(map_row(row)))
        .optional()
        .map_err(codec::query_failed)?
        .transpose()
}

#[async_trait]
impl Store for SqliteStore {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    async fn health_check(&self) -> Result<(), ProviderError> {
        self.run_blocking(|conn| {
            conn.query_row("SELECT 1", [], |_row| Ok(()))
                .map_err(codec::query_failed)
        })
        .await
    }

    async fn insert_monitor(&self, monitor: Monitor) -> Result<Monitor, ProviderError> {
        self.run_blocking(move |conn| {
            conn.execute(
                "INSERT INTO monitors (name, target, kind, interval_secs, status) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    monitor.name,
                    monitor.target,
                    codec::monitor_kind_to_str(monitor.kind),
                    monitor.interval_secs as i64,
                    codec::monitor_status_to_str(monitor.status),
                ],
            )
            .map_err(codec::query_failed)?;
            let id = conn.last_insert_rowid() as u64;
            Ok(Monitor { id, ..monitor })
        })
        .await
    }

    async fn get_monitor(&self, id: u64) -> Result<Option<Monitor>, ProviderError> {
        self.run_blocking(move |conn| {
            query_optional(
                conn,
                "SELECT id, name, target, kind, interval_secs, status \
                 FROM monitors WHERE id = ?1",
                params![id as i64],
                codec::row_to_monitor,
            )
        })
        .await
    }

    async fn list_monitors(&self) -> Result<Vec<Monitor>, ProviderError> {
        self.run_blocking(|conn| {
            query_all(
                conn,
                "SELECT id, name, target, kind, interval_secs, status \
                 FROM monitors ORDER BY id",
                [],
                codec::row_to_monitor,
            )
        })
        .await
    }

    async fn update_monitor(
        &self,
        id: u64,
        name: Option<String>,
        target: Option<String>,
        interval_secs: Option<u64>,
    ) -> Result<(), ProviderError> {
        self.run_blocking(move |conn| {
            let changed = conn
                .execute(
                    "UPDATE monitors SET \
                     name = COALESCE(?1, name), \
                     target = COALESCE(?2, target), \
                     interval_secs = COALESCE(?3, interval_secs) \
                     WHERE id = ?4",
                    params![name, target, interval_secs.map(|v| v as i64), id as i64],
                )
                .map_err(codec::query_failed)?;
            if changed == 0 {
                return Err(StorageError::NotFound {
                    entity: "monitor",
                    id,
                });
            }
            Ok(())
        })
        .await
    }

    async fn set_monitor_status(
        &self,
        id: u64,
        status: MonitorStatus,
    ) -> Result<(), ProviderError> {
        self.run_blocking(move |conn| {
            let changed = conn
                .execute(
                    "UPDATE monitors SET status = ?1 WHERE id = ?2",
                    params![codec::monitor_status_to_str(status), id as i64],
                )
                .map_err(codec::query_failed)?;
            if changed == 0 {
                return Err(StorageError::NotFound {
                    entity: "monitor",
                    id,
                });
            }
            Ok(())
        })
        .await
    }

    async fn delete_monitor(&self, id: u64) -> Result<(), ProviderError> {
        self.run_blocking(move |conn| {
            let changed = conn
                .execute("DELETE FROM monitors WHERE id = ?1", params![id as i64])
                .map_err(codec::query_failed)?;
            if changed == 0 {
                return Err(StorageError::NotFound {
                    entity: "monitor",
                    id,
                });
            }
            Ok(())
        })
        .await
    }

    async fn insert_check_results(&self, results: &[CheckResult]) -> Result<(), ProviderError> {
        let results = results.to_vec();
        self.run_blocking(move |conn| {
            let tx = conn.transaction().map_err(codec::query_failed)?;
            for result in &results {
                tx.execute(
                    "INSERT INTO check_results \
                     (monitor_id, checked_at, success, latency_ms, message) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        result.monitor_id as i64,
                        result.checked_at as i64,
                        result.success,
                        result.latency_ms as i64,
                        result.message,
                    ],
                )
                .map_err(codec::query_failed)?;
            }
            tx.commit().map_err(codec::query_failed)?;
            Ok(())
        })
        .await
    }

    async fn list_check_results(
        &self,
        monitor_id: u64,
        since: Option<u64>,
    ) -> Result<Vec<CheckResult>, ProviderError> {
        self.run_blocking(move |conn| {
            query_all(
                conn,
                "SELECT monitor_id, checked_at, success, latency_ms, message \
                 FROM check_results WHERE monitor_id = ?1 AND checked_at >= ?2 \
                 ORDER BY checked_at",
                params![monitor_id as i64, since.unwrap_or(0) as i64],
                codec::row_to_check_result,
            )
        })
        .await
    }

    async fn prune_check_results_older_than(
        &self,
        cutoff_unix_secs: u64,
    ) -> Result<u64, ProviderError> {
        self.run_blocking(move |conn| {
            let changed = conn
                .execute(
                    "DELETE FROM check_results WHERE checked_at < ?1",
                    params![cutoff_unix_secs as i64],
                )
                .map_err(codec::query_failed)?;
            Ok(changed as u64)
        })
        .await
    }

    async fn upsert_agent(&self, agent: Agent) -> Result<Agent, ProviderError> {
        self.run_blocking(move |conn| {
            let tx = conn.transaction().map_err(codec::query_failed)?;
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM agents WHERE name = ?1",
                    params![agent.name],
                    |row| row.get(0),
                )
                .optional()
                .map_err(codec::query_failed)?;
            let id = match existing {
                Some(id) => {
                    tx.execute(
                        "UPDATE agents SET last_heartbeat_at = ?1, scope = ?2 WHERE id = ?3",
                        params![agent.last_heartbeat_at as i64, agent.scope, id],
                    )
                    .map_err(codec::query_failed)?;
                    id as u64
                }
                None => {
                    tx.execute(
                        "INSERT INTO agents (name, last_heartbeat_at, scope) \
                         VALUES (?1, ?2, ?3)",
                        params![agent.name, agent.last_heartbeat_at as i64, agent.scope],
                    )
                    .map_err(codec::query_failed)?;
                    tx.last_insert_rowid() as u64
                }
            };
            tx.commit().map_err(codec::query_failed)?;
            Ok(Agent { id, ..agent })
        })
        .await
    }

    async fn heartbeat_agent(&self, id: u64, at_unix_secs: u64) -> Result<(), ProviderError> {
        self.run_blocking(move |conn| {
            let changed = conn
                .execute(
                    "UPDATE agents SET last_heartbeat_at = ?1 WHERE id = ?2",
                    params![at_unix_secs as i64, id as i64],
                )
                .map_err(codec::query_failed)?;
            if changed == 0 {
                return Err(StorageError::NotFound {
                    entity: "agent",
                    id,
                });
            }
            Ok(())
        })
        .await
    }

    async fn get_agent(&self, id: u64) -> Result<Option<Agent>, ProviderError> {
        self.run_blocking(move |conn| {
            query_optional(
                conn,
                "SELECT id, name, last_heartbeat_at, scope FROM agents WHERE id = ?1",
                params![id as i64],
                codec::row_to_agent,
            )
        })
        .await
    }

    async fn list_agents(&self) -> Result<Vec<Agent>, ProviderError> {
        self.run_blocking(|conn| {
            query_all(
                conn,
                "SELECT id, name, last_heartbeat_at, scope FROM agents ORDER BY id",
                [],
                codec::row_to_agent,
            )
        })
        .await
    }

    async fn insert_alert_event(&self, event: AlertEvent) -> Result<AlertEvent, ProviderError> {
        self.run_blocking(move |conn| {
            conn.execute(
                "INSERT INTO alert_events \
                 (monitor_id, transitioned_to, occurred_at, sinks_attempted, delivery_outcome) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    event.monitor_id as i64,
                    codec::monitor_status_to_str(event.transitioned_to),
                    event.occurred_at as i64,
                    event.sinks_attempted,
                    event.delivery_outcome,
                ],
            )
            .map_err(codec::query_failed)?;
            let id = conn.last_insert_rowid() as u64;
            Ok(AlertEvent { id, ..event })
        })
        .await
    }

    async fn list_alert_events(&self, monitor_id: u64) -> Result<Vec<AlertEvent>, ProviderError> {
        self.run_blocking(move |conn| {
            query_all(
                conn,
                "SELECT id, monitor_id, transitioned_to, occurred_at, \
                 sinks_attempted, delivery_outcome \
                 FROM alert_events WHERE monitor_id = ?1 ORDER BY occurred_at",
                params![monitor_id as i64],
                codec::row_to_alert_event,
            )
        })
        .await
    }
}
