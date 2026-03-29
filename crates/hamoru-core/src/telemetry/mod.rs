//! Layer 1: Configuration & Telemetry.
//!
//! Provides the `TelemetryStore` trait for recording execution history
//! and querying aggregated metrics. Backed by JSON file in Phase 1,
//! SQLite in Phase 2+.

pub mod json_file;
pub mod memory;
pub mod projection;
pub mod sqlite;

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::provider::TokenUsage;

/// Persistent store for execution history and metrics.
///
/// Implementations record every LLM call and provide aggregated metrics
/// for the Policy Engine's cost calculations and `hamoru plan` predictions.
#[async_trait]
pub trait TelemetryStore: Send + Sync {
    /// Records a single execution history entry.
    async fn record(&self, entry: &HistoryEntry) -> Result<()>;

    /// Queries aggregated metrics over a time period.
    async fn query_metrics(&self, period: Duration) -> Result<Metrics>;

    /// Loads the cached metrics snapshot for Policy Engine use.
    ///
    /// Uses a default period of 7 days.
    async fn load_cache(&self) -> Result<MetricsCache>;

    /// Queries detailed metrics with per-model and per-provider breakdowns.
    async fn query_detailed_metrics(&self, period: Duration) -> Result<MetricsCache>;
}

/// A single execution history record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Timestamp of the execution.
    pub timestamp: DateTime<Utc>,
    /// Provider name (e.g., "claude").
    pub provider: String,
    /// Model identifier (e.g., "claude-sonnet-4-6").
    pub model: String,
    /// Token usage for this execution.
    pub tokens: TokenUsage,
    /// Cost in USD for this execution.
    pub cost: f64,
    /// Response latency in milliseconds.
    pub latency_ms: u64,
    /// Whether the execution completed successfully.
    pub success: bool,
    /// Tags for categorization (e.g., "review", "security").
    ///
    /// Empty until Policy Engine (Phase 3) populates them.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Request ID for audit trail correlation (Phase 5b).
    ///
    /// Set by the API server; `None` for CLI-originated requests.
    #[serde(default)]
    pub request_id: Option<String>,
}

/// Aggregated metrics over a time period.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metrics {
    /// Total number of requests in the period.
    pub total_requests: u64,
    /// Total cost in USD.
    pub total_cost: f64,
    /// Total input tokens consumed.
    pub total_input_tokens: u64,
    /// Total output tokens generated.
    pub total_output_tokens: u64,
    /// Average latency in milliseconds.
    pub avg_latency_ms: f64,
}

/// Per-model metrics breakdown.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelMetrics {
    /// Provider name (e.g., "claude").
    pub provider: String,
    /// Number of requests.
    pub requests: u64,
    /// Total cost in USD.
    pub cost: f64,
    /// Total input tokens.
    pub input_tokens: u64,
    /// Total output tokens.
    pub output_tokens: u64,
    /// Average latency in milliseconds.
    pub avg_latency_ms: f64,
}

/// Cached metrics snapshot for fast Policy Engine lookups.
///
/// Contains aggregated totals plus per-model and per-provider breakdowns.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricsCache {
    /// Aggregated totals.
    pub total: Metrics,
    /// Per-model breakdown (key: model ID).
    pub by_model: HashMap<String, ModelMetrics>,
    /// Per-provider breakdown (key: provider name).
    pub by_provider: HashMap<String, Metrics>,
    /// Number of days covered by this cache.
    pub period_days: u64,
    /// Total number of history entries in the cache period.
    pub entry_count: u64,
}
