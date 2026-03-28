//! Model scoring algorithms for policy-based selection.
//!
//! Provides quality tier classification and scoring functions that rank models
//! by cost, quality, latency, or a balanced combination.

use crate::provider::ModelInfo;
use crate::telemetry::MetricsCache;

use super::QualityTier;

/// Cost-per-output-token threshold for High tier (Sonnet-class+).
const HIGH_TIER_THRESHOLD: f64 = 10.0 / 1_000_000.0;

/// Cost-per-output-token threshold for Medium tier (Haiku-class).
const MEDIUM_TIER_THRESHOLD: f64 = 2.0 / 1_000_000.0;

/// Weight for quality component in balanced scoring.
const BALANCED_QUALITY_WEIGHT: f64 = 0.4;

/// Weight for cost component in balanced scoring.
const BALANCED_COST_WEIGHT: f64 = 0.35;

/// Weight for latency component in balanced scoring.
const BALANCED_LATENCY_WEIGHT: f64 = 0.25;

/// Default score for models missing latency data in MetricsCache.
const MISSING_LATENCY_DEFAULT_SCORE: f64 = 0.5;

/// Classifies a model's quality tier based on output token cost.
///
/// This heuristic reflects current LLM pricing where cost strongly
/// correlates with capability. Thresholds are calibrated to March 2026
/// pricing (Sonnet ≈ $15/M output, Haiku ≈ $4/M, local = $0).
pub fn quality_tier(model: &ModelInfo) -> QualityTier {
    if model.cost_per_output_token >= HIGH_TIER_THRESHOLD {
        QualityTier::High
    } else if model.cost_per_output_token >= MEDIUM_TIER_THRESHOLD {
        QualityTier::Medium
    } else {
        QualityTier::Low
    }
}

/// Scores models by cost — cheapest gets 1.0, most expensive gets 0.0.
///
/// Equal costs result in all models scoring 1.0. Empty input returns empty.
pub fn score_by_cost(models: &[ModelInfo]) -> Vec<f64> {
    if models.is_empty() {
        return vec![];
    }

    let costs: Vec<f64> = models
        .iter()
        .map(|m| m.cost_per_input_token + m.cost_per_output_token)
        .collect();

    let min = costs.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = costs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;

    if range == 0.0 {
        return vec![1.0; models.len()];
    }

    costs.iter().map(|&c| 1.0 - (c - min) / range).collect()
}

/// Scores models by quality tier — highest tier gets 1.0.
///
/// Within the same tier, more expensive models score higher (cost as proxy
/// for capability within tier). Equal-tier equal-cost models tie at 1.0.
pub fn score_by_quality(models: &[ModelInfo]) -> Vec<f64> {
    if models.is_empty() {
        return vec![];
    }

    // Assign ordinal: Low=0, Medium=1, High=2
    let ordinals: Vec<f64> = models
        .iter()
        .map(|m| match quality_tier(m) {
            QualityTier::Low => 0.0,
            QualityTier::Medium => 1.0,
            QualityTier::High => 2.0,
        })
        .collect();

    // Primary: tier ordinal; tiebreak: cost within tier (higher cost = better)
    let costs: Vec<f64> = models.iter().map(|m| m.cost_per_output_token).collect();

    // Composite score: tier * 1000 + cost (tier dominates)
    let composites: Vec<f64> = ordinals
        .iter()
        .zip(costs.iter())
        .map(|(&ord, &cost)| ord * 1000.0 + cost * 1_000_000.0)
        .collect();

    normalize_max_is_best(&composites)
}

/// Scores models by latency — fastest gets 1.0, slowest gets 0.0.
///
/// Uses historical latency from `MetricsCache`. Models not in the cache
/// receive a default score of 0.5. All-zero latency results in all 1.0.
pub fn score_by_latency(models: &[ModelInfo], metrics: &MetricsCache) -> Vec<f64> {
    if models.is_empty() {
        return vec![];
    }

    let latencies: Vec<Option<f64>> = models
        .iter()
        .map(|m| metrics.by_model.get(&m.id).map(|mm| mm.avg_latency_ms))
        .collect();

    // If all models have known latency
    let known: Vec<f64> = latencies.iter().filter_map(|l| *l).collect();

    if known.is_empty() {
        // No latency data at all — everyone gets default
        return vec![MISSING_LATENCY_DEFAULT_SCORE; models.len()];
    }

    let min = known.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = known.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;

    if range == 0.0 {
        // All known latencies equal
        return latencies
            .iter()
            .map(|l| {
                if l.is_some() {
                    1.0
                } else {
                    MISSING_LATENCY_DEFAULT_SCORE
                }
            })
            .collect();
    }

    latencies
        .iter()
        .map(|l| match l {
            Some(latency) => 1.0 - (latency - min) / range,
            None => MISSING_LATENCY_DEFAULT_SCORE,
        })
        .collect()
}

/// Scores models with a balanced combination of quality, cost, and latency.
///
/// Weights: 40% quality + 35% cost + 25% latency.
pub fn score_balanced(models: &[ModelInfo], metrics: &MetricsCache) -> Vec<f64> {
    if models.is_empty() {
        return vec![];
    }

    let quality_scores = score_by_quality(models);
    let cost_scores = score_by_cost(models);
    let latency_scores = score_by_latency(models, metrics);

    quality_scores
        .iter()
        .zip(cost_scores.iter())
        .zip(latency_scores.iter())
        .map(|((&q, &c), &l)| {
            q * BALANCED_QUALITY_WEIGHT + c * BALANCED_COST_WEIGHT + l * BALANCED_LATENCY_WEIGHT
        })
        .collect()
}

/// Normalizes values so the max maps to 1.0 and min to 0.0.
///
/// If all values are equal, returns all 1.0.
fn normalize_max_is_best(values: &[f64]) -> Vec<f64> {
    if values.is_empty() {
        return vec![];
    }

    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;

    if range == 0.0 {
        return vec![1.0; values.len()];
    }

    values.iter().map(|&v| (v - min) / range).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::test_fixtures::*;

    // --- quality_tier tests ---

    #[test]
    fn quality_tier_high_cost_model() {
        let m = model_sonnet(); // output = 15.0/1M
        assert_eq!(quality_tier(&m), QualityTier::High);
    }

    #[test]
    fn quality_tier_medium_cost_model() {
        let m = model_haiku(); // output = 4.0/1M
        assert_eq!(quality_tier(&m), QualityTier::Medium);
    }

    #[test]
    fn quality_tier_free_model() {
        let m = model_llama_70b(); // output = 0.0
        assert_eq!(quality_tier(&m), QualityTier::Low);
    }

    #[test]
    fn quality_tier_exact_threshold_boundary() {
        // Exactly at High threshold
        let m = model_with_cost("at-high", "test", 0.0, HIGH_TIER_THRESHOLD);
        assert_eq!(quality_tier(&m), QualityTier::High);

        // Just below High threshold
        let m = model_with_cost(
            "below-high",
            "test",
            0.0,
            HIGH_TIER_THRESHOLD - f64::EPSILON,
        );
        assert_eq!(quality_tier(&m), QualityTier::Medium);

        // Exactly at Medium threshold
        let m = model_with_cost("at-med", "test", 0.0, MEDIUM_TIER_THRESHOLD);
        assert_eq!(quality_tier(&m), QualityTier::Medium);

        // Just below Medium threshold
        let m = model_with_cost(
            "below-med",
            "test",
            0.0,
            MEDIUM_TIER_THRESHOLD - f64::EPSILON,
        );
        assert_eq!(quality_tier(&m), QualityTier::Low);
    }

    // --- score_by_cost tests ---

    #[test]
    fn score_by_cost_cheapest_wins() {
        let models = vec![model_sonnet(), model_haiku(), model_llama_70b()];
        let scores = score_by_cost(&models);
        // llama (free) should have highest score
        assert!(scores[2] > scores[1]);
        assert!(scores[1] > scores[0]);
        assert!((scores[2] - 1.0).abs() < f64::EPSILON);
        assert!((scores[0] - 0.0).abs() < 0.01); // most expensive ≈ 0
    }

    #[test]
    fn score_by_cost_all_equal() {
        let models = vec![
            model_with_cost("a", "p", 1.0, 2.0),
            model_with_cost("b", "p", 1.0, 2.0),
            model_with_cost("c", "p", 1.0, 2.0),
        ];
        let scores = score_by_cost(&models);
        for s in &scores {
            assert!((s - 1.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn score_by_cost_extreme_range() {
        let models = vec![
            model_with_cost("free", "local", 0.0, 0.0),
            model_with_cost("expensive", "cloud", 0.05, 0.1),
        ];
        let scores = score_by_cost(&models);
        assert!((scores[0] - 1.0).abs() < f64::EPSILON); // free = best
        assert!((scores[1] - 0.0).abs() < f64::EPSILON); // expensive = worst
    }

    // --- score_by_quality tests ---

    #[test]
    fn score_by_quality_expensive_wins() {
        let models = vec![model_llama_70b(), model_haiku(), model_sonnet()];
        let scores = score_by_quality(&models);
        // Sonnet (High) > Haiku (Medium) > Llama (Low)
        assert!(scores[2] > scores[1]);
        assert!(scores[1] > scores[0]);
    }

    #[test]
    fn score_by_quality_same_tier_tiebreak_by_cost() {
        // Two High-tier models with different costs
        let m1 = model_with_cost("cheaper-high", "p", 0.0, 12.0 / 1_000_000.0);
        let m2 = model_with_cost("pricier-high", "p", 0.0, 20.0 / 1_000_000.0);
        let models = vec![m1, m2];
        let scores = score_by_quality(&models);
        // Pricier within same tier scores higher
        assert!(scores[1] > scores[0]);
    }

    // --- score_by_latency tests ---

    #[test]
    fn score_by_latency_fastest_wins() {
        let models = vec![model_sonnet(), model_haiku(), model_llama_70b()];
        let metrics = metrics_cache_with_latency(&[
            ("claude-sonnet-4-6", 2000.0),
            ("claude-haiku-4-5", 500.0),
            ("llama3.3:70b", 100.0),
        ]);
        let scores = score_by_latency(&models, &metrics);
        // Llama (100ms) fastest
        assert!((scores[2] - 1.0).abs() < f64::EPSILON);
        // Sonnet (2000ms) slowest
        assert!((scores[0] - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn score_by_latency_missing_model_gets_default() {
        let models = vec![model_sonnet(), model_llama_70b()];
        // Only sonnet has latency data
        let metrics = metrics_cache_with_latency(&[("claude-sonnet-4-6", 1000.0)]);
        let scores = score_by_latency(&models, &metrics);
        assert!((scores[0] - 1.0).abs() < f64::EPSILON); // only known → best
        assert!((scores[1] - MISSING_LATENCY_DEFAULT_SCORE).abs() < f64::EPSILON);
    }

    #[test]
    fn score_by_latency_all_zero() {
        let models = vec![model_sonnet(), model_haiku()];
        let metrics =
            metrics_cache_with_latency(&[("claude-sonnet-4-6", 0.0), ("claude-haiku-4-5", 0.0)]);
        let scores = score_by_latency(&models, &metrics);
        for s in &scores {
            assert!((s - 1.0).abs() < f64::EPSILON);
        }
    }

    // --- score_balanced tests ---

    #[test]
    fn score_balanced_combines_weights() {
        let models = vec![model_sonnet(), model_llama_70b()];
        let metrics =
            metrics_cache_with_latency(&[("claude-sonnet-4-6", 2000.0), ("llama3.3:70b", 100.0)]);
        let scores = score_balanced(&models, &metrics);
        // Both should have scores between 0 and 1
        assert!(scores[0] > 0.0 && scores[0] < 1.0);
        assert!(scores[1] > 0.0 && scores[1] < 1.0);
        // Sonnet wins on quality, Llama wins on cost+latency
        // With weights 0.4q + 0.35c + 0.25l, Llama should win (0.6 combined weight)
        assert!(scores[1] > scores[0]);
    }

    // --- single candidate ---

    #[test]
    fn single_candidate_scores_one() {
        let models = vec![model_sonnet()];
        let metrics = empty_metrics_cache();

        assert!((score_by_cost(&models)[0] - 1.0).abs() < f64::EPSILON);
        assert!((score_by_quality(&models)[0] - 1.0).abs() < f64::EPSILON);
        // Latency: single model with no data → default 0.5
        assert!(
            (score_by_latency(&models, &metrics)[0] - MISSING_LATENCY_DEFAULT_SCORE).abs()
                < f64::EPSILON
        );
    }
}
