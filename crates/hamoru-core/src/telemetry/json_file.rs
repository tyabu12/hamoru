//! JSON file-backed telemetry store.
//!
//! Persists execution history to `.hamoru/state.json`. Each `record()` call
//! appends to the in-memory list and writes the full state to disk.
//! Phase 2 replaces this with SQLite for better query performance.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{HistoryEntry, Metrics, MetricsCache, TelemetryStore};
use crate::Result;
use crate::error::HamoruError;

/// On-disk state format for `.hamoru/state.json`.
#[derive(Debug, Serialize, Deserialize)]
struct StateFile {
    version: String,
    entries: Vec<HistoryEntry>,
}

impl Default for StateFile {
    fn default() -> Self {
        Self {
            version: "1".to_string(),
            entries: Vec::new(),
        }
    }
}

/// JSON file-backed implementation of `TelemetryStore`.
///
/// Wraps an in-memory store and persists to disk on each `record()`.
pub struct JsonFileTelemetryStore {
    path: PathBuf,
    inner: super::memory::InMemoryTelemetryStore,
}

impl JsonFileTelemetryStore {
    /// Creates a new store, loading existing state from disk if the file exists.
    ///
    /// Creates parent directories if they don't exist.
    pub async fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| HamoruError::TelemetryError {
                    reason: format!(
                        "Failed to create directory '{}': {e}. Check file system permissions.",
                        parent.display()
                    ),
                })?;
        }

        let inner = super::memory::InMemoryTelemetryStore::new();

        // Load existing state if file exists
        if path.exists() {
            let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
                HamoruError::TelemetryError {
                    reason: format!("Failed to read state file '{}': {e}", path.display()),
                }
            })?;

            let state: StateFile =
                serde_json::from_str(&content).map_err(|e| HamoruError::TelemetryError {
                    reason: format!(
                        "Failed to parse state file '{}': {e}. The file may be corrupted.",
                        path.display()
                    ),
                })?;

            for entry in &state.entries {
                inner.record(entry).await?;
            }
        }

        Ok(Self { path, inner })
    }

    /// Returns the path to the state file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the current state to disk.
    async fn persist(&self, entries: &[HistoryEntry]) -> Result<()> {
        let state = StateFile {
            version: "1".to_string(),
            entries: entries.to_vec(),
        };
        let json =
            serde_json::to_string_pretty(&state).map_err(|e| HamoruError::TelemetryError {
                reason: format!("Failed to serialize state: {e}"),
            })?;
        tokio::fs::write(&self.path, json)
            .await
            .map_err(|e| HamoruError::TelemetryError {
                reason: format!(
                    "Failed to write state file '{}': {e}. Check file system permissions.",
                    self.path.display()
                ),
            })?;
        Ok(())
    }
}

#[async_trait]
impl TelemetryStore for JsonFileTelemetryStore {
    async fn record(&self, entry: &HistoryEntry) -> Result<()> {
        self.inner.record(entry).await?;
        let entries = self.inner.all_entries().await;
        self.persist(&entries).await?;
        Ok(())
    }

    async fn query_metrics(&self, period: Duration) -> Result<Metrics> {
        self.inner.query_metrics(period).await
    }

    async fn load_cache(&self) -> Result<MetricsCache> {
        self.inner.load_cache().await
    }

    async fn query_detailed_metrics(&self, period: Duration) -> Result<MetricsCache> {
        self.inner.query_detailed_metrics(period).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::TokenUsage;
    use chrono::Utc;

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
    async fn record_creates_file_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        let store = JsonFileTelemetryStore::new(&path).await.unwrap();
        store.record(&sample_entry()).await.unwrap();

        assert!(path.exists());

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let state: StateFile = serde_json::from_str(&content).unwrap();
        assert_eq!(state.version, "1");
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].provider, "claude");
    }

    #[tokio::test]
    async fn load_existing_state_on_construction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        // Write initial state
        {
            let store = JsonFileTelemetryStore::new(&path).await.unwrap();
            store.record(&sample_entry()).await.unwrap();
        }

        // Load from existing file
        let store = JsonFileTelemetryStore::new(&path).await.unwrap();
        let metrics = store
            .query_metrics(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(metrics.total_requests, 1);
    }

    #[tokio::test]
    async fn creates_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("state.json");

        let store = JsonFileTelemetryStore::new(&path).await.unwrap();
        store.record(&sample_entry()).await.unwrap();

        assert!(path.exists());
    }

    #[tokio::test]
    async fn multiple_records_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        let store = JsonFileTelemetryStore::new(&path).await.unwrap();
        store.record(&sample_entry()).await.unwrap();
        store.record(&sample_entry()).await.unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let state: StateFile = serde_json::from_str(&content).unwrap();
        assert_eq!(state.entries.len(), 2);
    }

    #[tokio::test]
    async fn empty_file_returns_zero_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        let store = JsonFileTelemetryStore::new(&path).await.unwrap();
        let metrics = store
            .query_metrics(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(metrics.total_requests, 0);
    }
}
