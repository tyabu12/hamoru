//! In-memory telemetry store.
//!
//! Useful for testing and short-lived sessions where persistence is not needed.

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::RwLock;

use std::collections::HashMap;

use super::{HistoryEntry, Metrics, MetricsCache, ModelMetrics, TelemetryStore};
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
        Ok(aggregate_metrics(&filtered))
    }

    async fn load_cache(&self) -> Result<MetricsCache> {
        self.query_detailed_metrics(Duration::from_secs(7 * 24 * 3600))
            .await
    }

    async fn query_detailed_metrics(&self, period: Duration) -> Result<MetricsCache> {
        let entries = self.entries.read().await;
        let cutoff = Utc::now() - period;
        let filtered: Vec<&HistoryEntry> =
            entries.iter().filter(|e| e.timestamp >= cutoff).collect();

        if filtered.is_empty() {
            return Ok(MetricsCache {
                period_days: period.as_secs() / 86400,
                ..Default::default()
            });
        }

        let total = aggregate_metrics(&filtered);
        let entry_count = filtered.len() as u64;

        // Per-model breakdown
        let mut by_model: HashMap<String, Vec<&HistoryEntry>> = HashMap::new();
        for entry in &filtered {
            by_model.entry(entry.model.clone()).or_default().push(entry);
        }
        let by_model = by_model
            .into_iter()
            .map(|(model, entries)| {
                // Use the lexicographically smallest provider for deterministic results
                // when the same model appears across multiple providers.
                let provider = entries
                    .iter()
                    .map(|e| &e.provider)
                    .min()
                    .cloned()
                    .unwrap_or_default();
                let m = aggregate_metrics(&entries);
                (
                    model,
                    ModelMetrics {
                        provider,
                        requests: m.total_requests,
                        cost: m.total_cost,
                        input_tokens: m.total_input_tokens,
                        output_tokens: m.total_output_tokens,
                        avg_latency_ms: m.avg_latency_ms,
                    },
                )
            })
            .collect();

        // Per-provider breakdown
        let mut by_provider_entries: HashMap<String, Vec<&HistoryEntry>> = HashMap::new();
        for entry in &filtered {
            by_provider_entries
                .entry(entry.provider.clone())
                .or_default()
                .push(entry);
        }
        let by_provider = by_provider_entries
            .into_iter()
            .map(|(provider, entries)| (provider, aggregate_metrics(&entries)))
            .collect();

        Ok(MetricsCache {
            total,
            by_model,
            by_provider,
            period_days: period.as_secs() / 86400,
            entry_count,
        })
    }
}

/// Aggregates metrics from a slice of history entries.
fn aggregate_metrics(entries: &[&HistoryEntry]) -> Metrics {
    if entries.is_empty() {
        return Metrics::default();
    }
    let total_requests = entries.len() as u64;
    let total_cost: f64 = entries.iter().map(|e| e.cost).sum();
    let total_input_tokens: u64 = entries.iter().map(|e| e.tokens.input_tokens).sum();
    let total_output_tokens: u64 = entries.iter().map(|e| e.tokens.output_tokens).sum();
    let total_latency: u64 = entries.iter().map(|e| e.latency_ms).sum();
    let avg_latency_ms = total_latency as f64 / total_requests as f64;

    Metrics {
        total_requests,
        total_cost,
        total_input_tokens,
        total_output_tokens,
        avg_latency_ms,
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
            tags: vec![],
            request_id: None,
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

    #[tokio::test]
    async fn detailed_metrics_per_model_breakdown() {
        let store = InMemoryTelemetryStore::new();

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
        assert_eq!(sonnet.provider, "claude");

        let llama = &cache.by_model["llama3.3:70b"];
        assert_eq!(llama.requests, 1);
        assert!((llama.cost - 0.001).abs() < f64::EPSILON);
        assert_eq!(llama.provider, "ollama");
    }

    #[tokio::test]
    async fn detailed_metrics_same_model_multiple_providers() {
        let store = InMemoryTelemetryStore::new();

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
        // Deterministic: lexicographically smallest provider wins
        assert_eq!(model.provider, "provider-a");
    }

    #[tokio::test]
    async fn detailed_metrics_per_provider_breakdown() {
        let store = InMemoryTelemetryStore::new();

        let mut e1 = sample_entry();
        e1.provider = "claude".to_string();
        e1.cost = 0.01;
        store.record(&e1).await.unwrap();

        let mut e2 = sample_entry();
        e2.provider = "ollama".to_string();
        e2.cost = 0.001;
        store.record(&e2).await.unwrap();

        let cache = store
            .query_detailed_metrics(Duration::from_secs(3600))
            .await
            .unwrap();

        assert_eq!(cache.by_provider.len(), 2);
        assert_eq!(cache.by_provider["claude"].total_requests, 1);
        assert_eq!(cache.by_provider["ollama"].total_requests, 1);
    }

    #[tokio::test]
    async fn detailed_metrics_empty_store() {
        let store = InMemoryTelemetryStore::new();
        let cache = store
            .query_detailed_metrics(Duration::from_secs(3600))
            .await
            .unwrap();

        assert_eq!(cache.entry_count, 0);
        assert_eq!(cache.total.total_requests, 0);
        assert!(cache.by_model.is_empty());
        assert!(cache.by_provider.is_empty());
    }

    #[tokio::test]
    async fn tags_serde_roundtrip() {
        let mut entry = sample_entry();
        entry.tags = vec!["review".to_string(), "security".to_string()];

        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: HistoryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.tags, vec!["review", "security"]);
    }

    #[tokio::test]
    async fn tags_default_when_missing() {
        // Simulate JSON from Phase 1 without tags field
        let json = r#"{
            "timestamp": "2026-03-28T00:00:00Z",
            "provider": "claude",
            "model": "claude-sonnet-4-6",
            "tokens": {"input_tokens": 100, "output_tokens": 200},
            "cost": 0.01,
            "latency_ms": 500,
            "success": true
        }"#;
        let entry: HistoryEntry = serde_json::from_str(json).unwrap();
        assert!(entry.tags.is_empty());
    }

    #[tokio::test]
    async fn load_cache_uses_7d_period() {
        let store = InMemoryTelemetryStore::new();

        // Recent entry
        store.record(&sample_entry()).await.unwrap();

        // Old entry (8 days ago)
        let mut old = sample_entry();
        old.timestamp = Utc::now() - Duration::from_secs(8 * 24 * 3600);
        store.record(&old).await.unwrap();

        let cache = store.load_cache().await.unwrap();
        // Only the recent entry should be included
        assert_eq!(cache.entry_count, 1);
        assert_eq!(cache.period_days, 7);
    }
}
