//! Layer 1: Configuration & Telemetry.
//!
//! Provides the `TelemetryStore` trait for recording execution history
//! and querying aggregated metrics. Backed by SQLite in Phase 2+.

use std::time::Duration;

use async_trait::async_trait;

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
    async fn load_cache(&self) -> Result<MetricsCache>;
}

/// A single execution history record.
// TODO: Finalize fields in Phase 2.
#[derive(Debug, Clone, Default)]
pub struct HistoryEntry {
    /// Token usage for this execution.
    pub tokens: TokenUsage,
}

/// Aggregated metrics over a time period.
// TODO: Finalize fields in Phase 2.
#[derive(Debug, Clone, Default)]
pub struct Metrics;

/// Cached metrics snapshot for fast Policy Engine lookups.
// TODO: Finalize fields in Phase 2.
#[derive(Debug, Clone, Default)]
pub struct MetricsCache;
