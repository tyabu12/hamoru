//! In-memory telemetry store.
//!
//! Useful for testing and short-lived sessions where persistence is not needed.

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::RwLock;

use super::{HistoryEntry, Metrics, MetricsCache, TelemetryStore};
use crate::Result;

/// In-memory implementation of `TelemetryStore`.
///
/// Stores history entries in a `Vec` behind an async `RwLock`.
/// Data is lost when the process exits.
pub struct InMemoryTelemetryStore {
    entries: RwLock<Vec<HistoryEntry>>,
}

impl InMemoryTelemetryStore {
    /// Creates a new empty in-memory store.
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
        }
    }
}

impl Default for InMemoryTelemetryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryTelemetryStore {
    /// Returns a clone of all stored entries.
    pub(crate) async fn all_entries(&self) -> Vec<HistoryEntry> {
        self.entries.read().await.clone()
    }
}

#[async_trait]
impl TelemetryStore for InMemoryTelemetryStore {
    async fn record(&self, entry: &HistoryEntry) -> Result<()> {
        self.entries.write().await.push(entry.clone());
        Ok(())
    }

    async fn query_metrics(&self, period: Duration) -> Result<Metrics> {
        let entries = self.entries.read().await;
        let cutoff = Utc::now() - period;

        let filtered: Vec<&HistoryEntry> =
            entries.iter().filter(|e| e.timestamp >= cutoff).collect();

        if filtered.is_empty() {
            return Ok(Metrics::default());
        }

        let total_requests = filtered.len() as u64;
        let total_cost: f64 = filtered.iter().map(|e| e.cost).sum();
        let total_input_tokens: u64 = filtered.iter().map(|e| e.tokens.input_tokens).sum();
        let total_output_tokens: u64 = filtered.iter().map(|e| e.tokens.output_tokens).sum();
        let total_latency: u64 = filtered.iter().map(|e| e.latency_ms).sum();
        let avg_latency_ms = total_latency as f64 / total_requests as f64;

        Ok(Metrics {
            total_requests,
            total_cost,
            total_input_tokens,
            total_output_tokens,
            avg_latency_ms,
        })
    }

    async fn load_cache(&self) -> Result<MetricsCache> {
        Ok(MetricsCache)
    }
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
        }
    }

    #[tokio::test]
    async fn record_and_query_roundtrip() {
        let store = InMemoryTelemetryStore::new();
        let entry = sample_entry();
        store.record(&entry).await.unwrap();

        let metrics = store
            .query_metrics(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(metrics.total_requests, 1);
        assert!((metrics.total_cost - 0.0033).abs() < f64::EPSILON);
        assert_eq!(metrics.total_input_tokens, 100);
        assert_eq!(metrics.total_output_tokens, 200);
        assert!((metrics.avg_latency_ms - 500.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn empty_store_returns_zero_metrics() {
        let store = InMemoryTelemetryStore::new();
        let metrics = store
            .query_metrics(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(metrics.total_requests, 0);
        assert!((metrics.total_cost).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn query_filters_by_period() {
        let store = InMemoryTelemetryStore::new();

        // Entry from now
        let recent = sample_entry();
        store.record(&recent).await.unwrap();

        // Entry from the past (simulate by modifying timestamp)
        let mut old = sample_entry();
        old.timestamp = Utc::now() - Duration::from_secs(7200);
        store.record(&old).await.unwrap();

        // Query last hour — should only include the recent entry
        let metrics = store
            .query_metrics(Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(metrics.total_requests, 1);
    }

    #[tokio::test]
    async fn multiple_records_aggregate() {
        let store = InMemoryTelemetryStore::new();

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
}
