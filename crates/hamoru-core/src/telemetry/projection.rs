//! Telemetry-based cost projection.
//!
//! Pure functions that take a `MetricsCache` and produce cost projections.
//! Used by `hamoru plan` to estimate daily costs and model usage patterns.
//! Policy-aware cost impact prediction is deferred to Phase 3 (PolicyEngine).

use serde::Serialize;

use super::MetricsCache;

/// Cost projection based on historical telemetry data.
#[derive(Debug, Clone, Serialize)]
pub struct CostProjection {
    /// Estimated daily cost in USD.
    pub daily_cost: f64,
    /// Estimated daily request count.
    pub daily_requests: f64,
    /// Per-model cost breakdown, sorted by cost descending.
    pub top_models: Vec<ModelCostSummary>,
    /// Confidence score (0.0 to 1.0) based on data volume.
    pub confidence: f64,
    /// Number of days of data used for the projection.
    pub data_period_days: u64,
}

/// Per-model cost summary for projections.
#[derive(Debug, Clone, Serialize)]
pub struct ModelCostSummary {
    /// Model identifier.
    pub model: String,
    /// Provider name.
    pub provider: String,
    /// Estimated daily request count.
    pub daily_requests: f64,
    /// Estimated daily cost in USD.
    pub daily_cost: f64,
    /// Percentage of total cost.
    pub pct_of_total: f64,
}

/// Projects costs based on historical telemetry data.
///
/// This is a pure function with no I/O. The projection extrapolates
/// daily averages from the cache period.
pub fn project_costs(cache: &MetricsCache) -> CostProjection {
    let period_days = if cache.period_days == 0 {
        1
    } else {
        cache.period_days
    };

    if cache.entry_count == 0 {
        return CostProjection {
            daily_cost: 0.0,
            daily_requests: 0.0,
            top_models: vec![],
            confidence: 0.0,
            data_period_days: period_days,
        };
    }

    let daily_cost = cache.total.total_cost / period_days as f64;
    let daily_requests = cache.total.total_requests as f64 / period_days as f64;

    let mut top_models: Vec<ModelCostSummary> = cache
        .by_model
        .iter()
        .map(|(model, metrics)| {
            let model_daily_cost = metrics.cost / period_days as f64;
            let model_daily_requests = metrics.requests as f64 / period_days as f64;
            let pct_of_total = if cache.total.total_cost > 0.0 {
                (metrics.cost / cache.total.total_cost) * 100.0
            } else {
                0.0
            };
            ModelCostSummary {
                model: model.clone(),
                provider: metrics.provider.clone(),
                daily_requests: model_daily_requests,
                daily_cost: model_daily_cost,
                pct_of_total,
            }
        })
        .collect();

    // Sort by cost descending
    top_models.sort_by(|a, b| {
        b.daily_cost
            .partial_cmp(&a.daily_cost)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let confidence = calculate_confidence(period_days, cache.entry_count);

    CostProjection {
        daily_cost,
        daily_requests,
        top_models,
        confidence,
        data_period_days: period_days,
    }
}

/// Calculates confidence score based on data volume.
///
/// - <1 day of data: 30%
/// - 1-3 days: 60%
/// - 3-7 days: 80%
/// - 7+ days: 90%+
///
/// Entry count also factors in — more data points increase confidence.
fn calculate_confidence(period_days: u64, entry_count: u64) -> f64 {
    let base = match period_days {
        0 => 0.1,
        1..=2 => 0.3,
        3..=6 => 0.6,
        _ => 0.8,
    };

    // Bonus for entry count (up to +0.15)
    let entry_bonus = (entry_count as f64 / 100.0).min(0.15);

    (base + entry_bonus).min(0.95)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::telemetry::{Metrics, ModelMetrics};

    #[test]
    fn project_costs_empty_cache() {
        let cache = MetricsCache::default();
        let proj = project_costs(&cache);

        assert_eq!(proj.daily_cost, 0.0);
        assert_eq!(proj.daily_requests, 0.0);
        assert!(proj.top_models.is_empty());
        assert_eq!(proj.confidence, 0.0);
    }

    #[test]
    fn project_costs_single_model() {
        let mut by_model = HashMap::new();
        by_model.insert(
            "claude-sonnet-4-6".to_string(),
            ModelMetrics {
                provider: "claude".to_string(),
                requests: 70,
                cost: 7.0,
                input_tokens: 7000,
                output_tokens: 14000,
                avg_latency_ms: 500.0,
            },
        );

        let cache = MetricsCache {
            total: Metrics {
                total_requests: 70,
                total_cost: 7.0,
                total_input_tokens: 7000,
                total_output_tokens: 14000,
                avg_latency_ms: 500.0,
            },
            by_model,
            by_provider: HashMap::new(),
            period_days: 7,
            entry_count: 70,
        };

        let proj = project_costs(&cache);
        assert!((proj.daily_cost - 1.0).abs() < f64::EPSILON);
        assert!((proj.daily_requests - 10.0).abs() < f64::EPSILON);
        assert_eq!(proj.top_models.len(), 1);
        assert!((proj.top_models[0].pct_of_total - 100.0).abs() < f64::EPSILON);
        assert!(proj.confidence >= 0.8);
    }

    #[test]
    fn project_costs_multi_model_sorted_by_cost() {
        let mut by_model = HashMap::new();
        by_model.insert(
            "llama3.3:70b".to_string(),
            ModelMetrics {
                provider: "ollama".to_string(),
                requests: 50,
                cost: 0.5,
                input_tokens: 5000,
                output_tokens: 10000,
                avg_latency_ms: 200.0,
            },
        );
        by_model.insert(
            "claude-sonnet-4-6".to_string(),
            ModelMetrics {
                provider: "claude".to_string(),
                requests: 20,
                cost: 5.0,
                input_tokens: 2000,
                output_tokens: 4000,
                avg_latency_ms: 800.0,
            },
        );

        let cache = MetricsCache {
            total: Metrics {
                total_requests: 70,
                total_cost: 5.5,
                total_input_tokens: 7000,
                total_output_tokens: 14000,
                avg_latency_ms: 400.0,
            },
            by_model,
            by_provider: HashMap::new(),
            period_days: 7,
            entry_count: 70,
        };

        let proj = project_costs(&cache);
        assert_eq!(proj.top_models.len(), 2);
        // Most expensive model first
        assert_eq!(proj.top_models[0].model, "claude-sonnet-4-6");
        assert_eq!(proj.top_models[1].model, "llama3.3:70b");
    }

    #[test]
    fn confidence_scales_with_period() {
        assert!(calculate_confidence(0, 10) < calculate_confidence(1, 10));
        assert!(calculate_confidence(2, 10) < calculate_confidence(4, 10));
        assert!(calculate_confidence(5, 10) < calculate_confidence(8, 10));
    }

    #[test]
    fn confidence_entry_bonus() {
        let low = calculate_confidence(7, 5);
        let high = calculate_confidence(7, 200);
        assert!(high > low);
        // Max confidence is 0.95
        assert!(high <= 0.95);
    }
}
