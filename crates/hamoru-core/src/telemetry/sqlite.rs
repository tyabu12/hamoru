//! SQLite-backed telemetry store.
//!
//! Replaces JsonFileTelemetryStore in Phase 2 for better query performance
//! and metrics aggregation. Uses `spawn_blocking` to bridge rusqlite's
//! synchronous API with the async `TelemetryStore` trait.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{HistoryEntry, Metrics, MetricsCache, ModelMetrics, TelemetryStore};
use crate::Result;
use crate::error::HamoruError;

/// Schema DDL for the telemetry database.
const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_creation_input_tokens INTEGER,
    cache_read_input_tokens INTEGER,
    cost REAL NOT NULL,
    latency_ms INTEGER NOT NULL,
    success INTEGER NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX IF NOT EXISTS idx_history_timestamp ON history(timestamp);
CREATE INDEX IF NOT EXISTS idx_history_model ON history(model);
CREATE INDEX IF NOT EXISTS idx_history_provider ON history(provider);
"#;

/// SQLite-backed implementation of `TelemetryStore`.
///
/// Wraps a `rusqlite::Connection` behind an async `Mutex`. All database
/// operations run on the blocking thread pool via `tokio::task::spawn_blocking`.
pub struct SqliteTelemetryStore {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl SqliteTelemetryStore {
    /// Opens (or creates) a SQLite database at the given path.
    ///
    /// Applies the schema DDL and sets WAL journal mode. On unix, the file
    /// permissions are set to 0o600.
    pub async fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let db_path = path.clone();

        // Ensure parent directory exists
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| HamoruError::TelemetryError {
                    reason: format!(
                        "Failed to create directory '{}': {e}. Check file system permissions.",
                        parent.display()
                    ),
                })?;
        }

        let conn = tokio::task::spawn_blocking(move || -> Result<Connection> {
            let conn = Connection::open(&db_path).map_err(|e| HamoruError::TelemetryError {
                reason: format!(
                    "Failed to open SQLite database '{}': {e}. Check file system permissions.",
                    db_path.display()
                ),
            })?;

            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
                .map_err(|e| HamoruError::TelemetryError {
                    reason: format!("Failed to set SQLite pragmas: {e}"),
                })?;

            conn.execute_batch(SCHEMA_SQL)
                .map_err(|e| HamoruError::TelemetryError {
                    reason: format!("Failed to apply database schema: {e}"),
                })?;

            Ok(conn)
        })
        .await
        .map_err(|e| HamoruError::TelemetryError {
            reason: format!("SQLite initialization task panicked: {e}"),
        })??;

        // Set file permissions to 0o600 on unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if path.exists() {
                let perms = std::fs::Permissions::from_mode(0o600);
                std::fs::set_permissions(&path, perms).map_err(|e| {
                    HamoruError::TelemetryError {
                        reason: format!("Failed to set permissions on '{}': {e}", path.display()),
                    }
                })?;
            }
        }

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path,
        })
    }

    /// Returns the path to the database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the timestamp range of stored entries as (oldest, newest).
    ///
    /// Returns `None` if the store is empty.
    pub async fn date_range(&self) -> Result<Option<(String, String)>> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))
                .map_err(|e| HamoruError::TelemetryError {
                    reason: format!("Failed to count entries: {e}"),
                })?;
            if count == 0 {
                return Ok(None);
            }
            let oldest: String = conn
                .query_row("SELECT MIN(timestamp) FROM history", [], |row| row.get(0))
                .map_err(|e| HamoruError::TelemetryError {
                    reason: format!("Failed to query date range: {e}"),
                })?;
            let newest: String = conn
                .query_row("SELECT MAX(timestamp) FROM history", [], |row| row.get(0))
                .map_err(|e| HamoruError::TelemetryError {
                    reason: format!("Failed to query date range: {e}"),
                })?;
            Ok(Some((oldest, newest)))
        })
        .await
        .map_err(|e| HamoruError::TelemetryError {
            reason: format!("Date range task panicked: {e}"),
        })?
    }

    /// Returns the number of entries in the database.
    pub async fn entry_count(&self) -> Result<u64> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))
                .map_err(|e| HamoruError::TelemetryError {
                    reason: format!("Failed to count entries: {e}"),
                })?;
            Ok(count as u64)
        })
        .await
        .map_err(|e| HamoruError::TelemetryError {
            reason: format!("Entry count task panicked: {e}"),
        })?
    }
}

#[async_trait]
impl TelemetryStore for SqliteTelemetryStore {
    async fn record(&self, entry: &HistoryEntry) -> Result<()> {
        let conn = Arc::clone(&self.conn);
        let timestamp = entry.timestamp.to_rfc3339();
        let provider = entry.provider.clone();
        let model = entry.model.clone();
        let input_tokens = entry.tokens.input_tokens as i64;
        let output_tokens = entry.tokens.output_tokens as i64;
        let cache_creation = entry.tokens.cache_creation_input_tokens.map(|v| v as i64);
        let cache_read = entry.tokens.cache_read_input_tokens.map(|v| v as i64);
        let cost = entry.cost;
        let latency_ms = entry.latency_ms as i64;
        let success = entry.success as i32;
        let tags = serde_json::to_string(&entry.tags).map_err(|e| HamoruError::TelemetryError {
            reason: format!("Failed to serialize tags: {e}"),
        })?;

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO history (timestamp, provider, model, input_tokens, output_tokens, \
                 cache_creation_input_tokens, cache_read_input_tokens, cost, latency_ms, success, tags) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    timestamp,
                    provider,
                    model,
                    input_tokens,
                    output_tokens,
                    cache_creation,
                    cache_read,
                    cost,
                    latency_ms,
                    success,
                    tags,
                ],
            )
            .map_err(|e| HamoruError::TelemetryError {
                reason: format!("Failed to insert history entry: {e}"),
            })?;
            Ok(())
        })
        .await
        .map_err(|e| HamoruError::TelemetryError {
            reason: format!("Record task panicked: {e}"),
        })?
    }

    async fn query_metrics(&self, period: Duration) -> Result<Metrics> {
        let conn = Arc::clone(&self.conn);
        let cutoff = (Utc::now() - period).to_rfc3339();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn
                .prepare(
                    "SELECT COUNT(*), COALESCE(SUM(cost), 0), COALESCE(SUM(input_tokens), 0), \
                     COALESCE(SUM(output_tokens), 0), COALESCE(AVG(latency_ms), 0) \
                     FROM history WHERE timestamp >= ?1",
                )
                .map_err(|e| HamoruError::TelemetryError {
                    reason: format!("Failed to prepare metrics query: {e}"),
                })?;

            stmt.query_row(params![cutoff], |row| {
                Ok(Metrics {
                    total_requests: row.get::<_, i64>(0)? as u64,
                    total_cost: row.get(1)?,
                    total_input_tokens: row.get::<_, i64>(2)? as u64,
                    total_output_tokens: row.get::<_, i64>(3)? as u64,
                    avg_latency_ms: row.get(4)?,
                })
            })
            .map_err(|e| HamoruError::TelemetryError {
                reason: format!("Failed to query metrics: {e}"),
            })
        })
        .await
        .map_err(|e| HamoruError::TelemetryError {
            reason: format!("Query metrics task panicked: {e}"),
        })?
    }

    async fn load_cache(&self) -> Result<MetricsCache> {
        self.query_detailed_metrics(Duration::from_secs(7 * 24 * 3600))
            .await
    }

    async fn query_detailed_metrics(&self, period: Duration) -> Result<MetricsCache> {
        let conn = Arc::clone(&self.conn);
        let cutoff = (Utc::now() - period).to_rfc3339();
        let period_days = period.as_secs() / 86400;

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();

            // Total metrics (includes COUNT as total_requests)
            let total = query_aggregate_metrics(&conn, &cutoff)?;
            let entry_count = total.total_requests;

            // Per-model breakdown
            let by_model = query_model_breakdown(&conn, &cutoff)?;

            // Per-provider breakdown
            let by_provider = query_provider_breakdown(&conn, &cutoff)?;

            Ok(MetricsCache {
                total,
                by_model,
                by_provider,
                period_days,
                entry_count,
            })
        })
        .await
        .map_err(|e| HamoruError::TelemetryError {
            reason: format!("Detailed metrics task panicked: {e}"),
        })?
    }
}

/// Queries aggregate metrics from the history table.
fn query_aggregate_metrics(conn: &Connection, cutoff: &str) -> Result<Metrics> {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(cost), 0), COALESCE(SUM(input_tokens), 0), \
         COALESCE(SUM(output_tokens), 0), COALESCE(AVG(latency_ms), 0) \
         FROM history WHERE timestamp >= ?1",
        params![cutoff],
        |row| {
            Ok(Metrics {
                total_requests: row.get::<_, i64>(0)? as u64,
                total_cost: row.get(1)?,
                total_input_tokens: row.get::<_, i64>(2)? as u64,
                total_output_tokens: row.get::<_, i64>(3)? as u64,
                avg_latency_ms: row.get(4)?,
            })
        },
    )
    .map_err(|e| HamoruError::TelemetryError {
        reason: format!("Failed to query aggregate metrics: {e}"),
    })
}

/// Queries per-model breakdown from the history table.
///
/// Uses `MIN(provider)` to produce a deterministic provider value when the
/// same model appears across multiple providers.
fn query_model_breakdown(conn: &Connection, cutoff: &str) -> Result<HashMap<String, ModelMetrics>> {
    let mut stmt = conn
        .prepare(
            "SELECT model, MIN(provider), COUNT(*), COALESCE(SUM(cost), 0), \
             COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), \
             COALESCE(AVG(latency_ms), 0) \
             FROM history WHERE timestamp >= ?1 GROUP BY model",
        )
        .map_err(|e| HamoruError::TelemetryError {
            reason: format!("Failed to prepare model breakdown query: {e}"),
        })?;

    let rows = stmt
        .query_map(params![cutoff], |row| {
            Ok((
                row.get::<_, String>(0)?,
                ModelMetrics {
                    provider: row.get(1)?,
                    requests: row.get::<_, i64>(2)? as u64,
                    cost: row.get(3)?,
                    input_tokens: row.get::<_, i64>(4)? as u64,
                    output_tokens: row.get::<_, i64>(5)? as u64,
                    avg_latency_ms: row.get(6)?,
                },
            ))
        })
        .map_err(|e| HamoruError::TelemetryError {
            reason: format!("Failed to query model breakdown: {e}"),
        })?;

    let mut map = HashMap::new();
    for row in rows {
        let (model, metrics) = row.map_err(|e| HamoruError::TelemetryError {
            reason: format!("Failed to read model breakdown row: {e}"),
        })?;
        map.insert(model, metrics);
    }
    Ok(map)
}

/// Queries per-provider breakdown from the history table.
fn query_provider_breakdown(conn: &Connection, cutoff: &str) -> Result<HashMap<String, Metrics>> {
    let mut stmt = conn
        .prepare(
            "SELECT provider, COUNT(*), COALESCE(SUM(cost), 0), \
             COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), \
             COALESCE(AVG(latency_ms), 0) \
             FROM history WHERE timestamp >= ?1 GROUP BY provider",
        )
        .map_err(|e| HamoruError::TelemetryError {
            reason: format!("Failed to prepare provider breakdown query: {e}"),
        })?;

    let rows = stmt
        .query_map(params![cutoff], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Metrics {
                    total_requests: row.get::<_, i64>(1)? as u64,
                    total_cost: row.get(2)?,
                    total_input_tokens: row.get::<_, i64>(3)? as u64,
                    total_output_tokens: row.get::<_, i64>(4)? as u64,
                    avg_latency_ms: row.get(5)?,
                },
            ))
        })
        .map_err(|e| HamoruError::TelemetryError {
            reason: format!("Failed to query provider breakdown: {e}"),
        })?;

    let mut map = HashMap::new();
    for row in rows {
        let (provider, metrics) = row.map_err(|e| HamoruError::TelemetryError {
            reason: format!("Failed to read provider breakdown row: {e}"),
        })?;
        map.insert(provider, metrics);
    }
    Ok(map)
}

/// Result of a JSON-to-SQLite migration.
#[derive(Debug, Clone, Serialize)]
pub struct MigrationResult {
    /// Number of entries successfully migrated.
    pub entries_migrated: u64,
    /// Number of entries skipped (already existed).
    pub entries_skipped: u64,
}

/// Migrates telemetry data from a JSON state file to a SQLite store.
///
/// Reads the Phase 1 `.hamoru/state.json` format and inserts each entry
/// into the SQLite database. Entries that already exist (matching timestamp,
/// provider, and model) are skipped, making this function idempotent.
pub async fn migrate_from_json(
    json_path: &Path,
    store: &SqliteTelemetryStore,
) -> Result<MigrationResult> {
    // Read and parse the JSON state file
    let content =
        tokio::fs::read_to_string(json_path)
            .await
            .map_err(|e| HamoruError::TelemetryError {
                reason: format!(
                    "Failed to read JSON state file '{}': {e}",
                    json_path.display()
                ),
            })?;

    #[derive(Deserialize)]
    struct StateFile {
        #[allow(dead_code)]
        version: String,
        entries: Vec<HistoryEntry>,
    }

    let state: StateFile =
        serde_json::from_str(&content).map_err(|e| HamoruError::TelemetryError {
            reason: format!(
                "Failed to parse JSON state file '{}': {e}. The file may be corrupted.",
                json_path.display()
            ),
        })?;

    // Batch all inserts in a single spawn_blocking call with a transaction
    let conn = Arc::clone(&store.conn);
    let entries = state.entries;

    let (migrated, skipped) = tokio::task::spawn_blocking(move || {
        let conn = conn.blocking_lock();
        let tx = conn.unchecked_transaction().map_err(|e| HamoruError::TelemetryError {
            reason: format!("Failed to begin migration transaction: {e}"),
        })?;

        let mut migrated = 0u64;
        let mut skipped = 0u64;

        for entry in &entries {
            let timestamp = entry.timestamp.to_rfc3339();

            // Check if entry already exists
            let exists: bool = tx
                .query_row(
                    "SELECT COUNT(*) > 0 FROM history WHERE timestamp = ?1 AND provider = ?2 AND model = ?3",
                    params![timestamp, entry.provider, entry.model],
                    |row| row.get(0),
                )
                .map_err(|e| HamoruError::TelemetryError {
                    reason: format!("Failed to check for duplicate entry: {e}"),
                })?;

            if exists {
                skipped += 1;
                continue;
            }

            let tags = serde_json::to_string(&entry.tags).unwrap_or_else(|_| "[]".to_string());
            tx.execute(
                "INSERT INTO history (timestamp, provider, model, input_tokens, output_tokens, \
                 cache_creation_input_tokens, cache_read_input_tokens, cost, latency_ms, success, tags) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    timestamp,
                    entry.provider,
                    entry.model,
                    entry.tokens.input_tokens as i64,
                    entry.tokens.output_tokens as i64,
                    entry.tokens.cache_creation_input_tokens.map(|v| v as i64),
                    entry.tokens.cache_read_input_tokens.map(|v| v as i64),
                    entry.cost,
                    entry.latency_ms as i64,
                    entry.success as i32,
                    tags,
                ],
            )
            .map_err(|e| HamoruError::TelemetryError {
                reason: format!("Failed to insert migrated entry: {e}"),
            })?;
            migrated += 1;
        }

        tx.commit().map_err(|e| HamoruError::TelemetryError {
            reason: format!("Failed to commit migration transaction: {e}"),
        })?;

        Ok::<_, HamoruError>((migrated, skipped))
    })
    .await
    .map_err(|e| HamoruError::TelemetryError {
        reason: format!("Migration task panicked: {e}"),
    })??;

    Ok(MigrationResult {
        entries_migrated: migrated,
        entries_skipped: skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::TokenUsage;

    fn sample_entry() -> HistoryEntry {
        HistoryEntry {
            timestamp: Utc::now(),
            provider: "claude".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            tokens: TokenUsage {
                input_tokens: 100,
                output_tokens: 200,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
            cost: 0.0033,
            latency_ms: 500,
            success: true,
            tags: vec![],
        }
    }

    #[tokio::test]
    async fn constructor_creates_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");

        let store = SqliteTelemetryStore::new(&path).await.unwrap();
        assert!(path.exists());
        assert_eq!(store.path(), path);
    }

    #[tokio::test]
    async fn schema_is_applied() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let _store = SqliteTelemetryStore::new(&path).await.unwrap();

        // Verify the history table exists by querying sqlite_master
        let conn = Connection::open(&path).unwrap();
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='history'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn file_permissions_are_600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let _store = SqliteTelemetryStore::new(&path).await.unwrap();

        let metadata = std::fs::metadata(&path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn creates_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("state.db");
        let _store = SqliteTelemetryStore::new(&path).await.unwrap();
        assert!(path.exists());
    }

    #[tokio::test]
    async fn record_and_query_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let store = SqliteTelemetryStore::new(&path).await.unwrap();

        store.record(&sample_entry()).await.unwrap();

        let metrics = store
            .query_metrics(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(metrics.total_requests, 1);
        assert!((metrics.total_cost - 0.0033).abs() < f64::EPSILON);
        assert_eq!(metrics.total_input_tokens, 100);
        assert_eq!(metrics.total_output_tokens, 200);
    }

    #[tokio::test]
    async fn empty_store_returns_zero_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let store = SqliteTelemetryStore::new(&path).await.unwrap();

        let metrics = store
            .query_metrics(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(metrics.total_requests, 0);
        assert!((metrics.total_cost).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn query_filters_by_period() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let store = SqliteTelemetryStore::new(&path).await.unwrap();

        store.record(&sample_entry()).await.unwrap();

        let mut old = sample_entry();
        old.timestamp = Utc::now() - Duration::from_secs(7200);
        store.record(&old).await.unwrap();

        let metrics = store
            .query_metrics(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(metrics.total_requests, 1);
    }

    #[tokio::test]
    async fn multiple_records_aggregate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let store = SqliteTelemetryStore::new(&path).await.unwrap();

        let mut e1 = sample_entry();
        e1.cost = 0.01;
        e1.latency_ms = 200;
        store.record(&e1).await.unwrap();

        let mut e2 = sample_entry();
        e2.cost = 0.02;
        e2.latency_ms = 400;
        store.record(&e2).await.unwrap();

        let metrics = store
            .query_metrics(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(metrics.total_requests, 2);
        assert!((metrics.total_cost - 0.03).abs() < f64::EPSILON);
        assert!((metrics.avg_latency_ms - 300.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn detailed_metrics_per_model_breakdown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let store = SqliteTelemetryStore::new(&path).await.unwrap();

        let mut e1 = sample_entry();
        e1.model = "claude-sonnet-4-6".to_string();
        e1.provider = "claude".to_string();
        e1.cost = 0.01;
        store.record(&e1).await.unwrap();

        let mut e2 = sample_entry();
        e2.model = "llama3.3:70b".to_string();
        e2.provider = "ollama".to_string();
        e2.cost = 0.001;
        store.record(&e2).await.unwrap();

        let mut e3 = sample_entry();
        e3.model = "claude-sonnet-4-6".to_string();
        e3.provider = "claude".to_string();
        e3.cost = 0.02;
        store.record(&e3).await.unwrap();

        let cache = store
            .query_detailed_metrics(Duration::from_secs(3600))
            .await
            .unwrap();

        assert_eq!(cache.entry_count, 3);
        assert_eq!(cache.by_model.len(), 2);

        let sonnet = &cache.by_model["claude-sonnet-4-6"];
        assert_eq!(sonnet.requests, 2);
        assert!((sonnet.cost - 0.03).abs() < f64::EPSILON);

        let llama = &cache.by_model["llama3.3:70b"];
        assert_eq!(llama.requests, 1);
    }

    #[tokio::test]
    async fn detailed_metrics_same_model_multiple_providers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let store = SqliteTelemetryStore::new(&path).await.unwrap();

        // Same model from two different providers
        let mut e1 = sample_entry();
        e1.model = "shared-model".to_string();
        e1.provider = "provider-b".to_string();
        e1.cost = 0.01;
        store.record(&e1).await.unwrap();

        let mut e2 = sample_entry();
        e2.model = "shared-model".to_string();
        e2.provider = "provider-a".to_string();
        e2.cost = 0.02;
        store.record(&e2).await.unwrap();

        let cache = store
            .query_detailed_metrics(Duration::from_secs(3600))
            .await
            .unwrap();

        assert_eq!(cache.by_model.len(), 1);
        let model = &cache.by_model["shared-model"];
        assert_eq!(model.requests, 2);
        assert!((model.cost - 0.03).abs() < f64::EPSILON);
        // Deterministic: MIN(provider) = lexicographically smallest
        assert_eq!(model.provider, "provider-a");
    }

    #[tokio::test]
    async fn persistence_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");

        // Write data, drop store
        {
            let store = SqliteTelemetryStore::new(&path).await.unwrap();
            store.record(&sample_entry()).await.unwrap();
        }

        // Reopen and verify data persists
        let store = SqliteTelemetryStore::new(&path).await.unwrap();
        let metrics = store
            .query_metrics(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(metrics.total_requests, 1);
    }

    #[tokio::test]
    async fn tags_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let store = SqliteTelemetryStore::new(&path).await.unwrap();

        let mut entry = sample_entry();
        entry.tags = vec!["review".to_string(), "security".to_string()];
        store.record(&entry).await.unwrap();

        // Read back via raw query to verify tags stored correctly
        let conn = Arc::clone(&store.conn);
        let tags: String = tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.query_row("SELECT tags FROM history LIMIT 1", [], |row| row.get(0))
                .unwrap()
        })
        .await
        .unwrap();

        let parsed: Vec<String> = serde_json::from_str(&tags).unwrap();
        assert_eq!(parsed, vec!["review", "security"]);
    }

    /// Helper: creates a Phase 1 state.json file with given entries.
    async fn write_state_json(path: &Path, entries: &[HistoryEntry]) {
        #[derive(Serialize)]
        struct StateFile<'a> {
            version: &'a str,
            entries: &'a [HistoryEntry],
        }
        let state = StateFile {
            version: "1",
            entries,
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        tokio::fs::write(path, json).await.unwrap();
    }

    #[tokio::test]
    async fn migrate_from_json_basic() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("state.json");
        let db_path = dir.path().join("state.db");

        let e1 = sample_entry();
        let mut e2 = sample_entry();
        // Ensure entries are distinct for the duplicate check (timestamp+provider+model)
        e2.model = "llama3.3:70b".to_string();
        e2.provider = "ollama".to_string();
        let entries = vec![e1, e2];
        write_state_json(&json_path, &entries).await;

        let store = SqliteTelemetryStore::new(&db_path).await.unwrap();
        let result = migrate_from_json(&json_path, &store).await.unwrap();

        assert_eq!(result.entries_migrated, 2);
        assert_eq!(result.entries_skipped, 0);
        assert_eq!(store.entry_count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn migrate_from_json_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("state.json");
        let db_path = dir.path().join("state.db");

        let entries = vec![sample_entry()];
        write_state_json(&json_path, &entries).await;

        let store = SqliteTelemetryStore::new(&db_path).await.unwrap();

        // First migration
        let r1 = migrate_from_json(&json_path, &store).await.unwrap();
        assert_eq!(r1.entries_migrated, 1);

        // Second migration — should skip
        let r2 = migrate_from_json(&json_path, &store).await.unwrap();
        assert_eq!(r2.entries_migrated, 0);
        assert_eq!(r2.entries_skipped, 1);
        assert_eq!(store.entry_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn migrate_from_json_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("state.json");
        let db_path = dir.path().join("state.db");

        write_state_json(&json_path, &[]).await;

        let store = SqliteTelemetryStore::new(&db_path).await.unwrap();
        let result = migrate_from_json(&json_path, &store).await.unwrap();

        assert_eq!(result.entries_migrated, 0);
        assert_eq!(result.entries_skipped, 0);
    }

    #[tokio::test]
    async fn migrate_from_json_corrupted_file() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("state.json");
        let db_path = dir.path().join("state.db");

        tokio::fs::write(&json_path, "not valid json")
            .await
            .unwrap();

        let store = SqliteTelemetryStore::new(&db_path).await.unwrap();
        let result = migrate_from_json(&json_path, &store).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("corrupted"));
    }

    #[tokio::test]
    async fn migrate_preserves_tags() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("state.json");
        let db_path = dir.path().join("state.db");

        let mut entry = sample_entry();
        entry.tags = vec!["review".to_string()];
        write_state_json(&json_path, &[entry]).await;

        let store = SqliteTelemetryStore::new(&db_path).await.unwrap();
        migrate_from_json(&json_path, &store).await.unwrap();

        // Verify tags were preserved
        let conn = Arc::clone(&store.conn);
        let tags: String = tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.query_row("SELECT tags FROM history LIMIT 1", [], |row| row.get(0))
                .unwrap()
        })
        .await
        .unwrap();
        let parsed: Vec<String> = serde_json::from_str(&tags).unwrap();
        assert_eq!(parsed, vec!["review"]);
    }
}
