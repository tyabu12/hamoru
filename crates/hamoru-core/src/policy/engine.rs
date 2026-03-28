//! Default implementation of the `PolicyEngine` trait.
//!
//! `DefaultPolicyEngine` performs tag-based routing, constraint filtering,
//! and preference-based scoring to select the optimal model for each request.

use crate::Result;
use crate::error::HamoruError;
use crate::provider::ModelInfo;
use crate::telemetry::MetricsCache;

use super::config::PolicyConfig;
use super::scoring::{
    quality_tier, score_balanced, score_by_cost, score_by_latency, score_by_quality,
};
use super::{
    CostCheckResult, CostImpactReport, ModelSelection, ModelShift, PolicyEngine, Priority,
    RoutingRequest,
};

/// Calculates average daily spend from MetricsCache.
///
/// Guards against `period_days == 0` (returns 0.0).
fn daily_spend(cache: &MetricsCache) -> f64 {
    if cache.period_days == 0 {
        0.0
    } else {
        cache.total.total_cost / cache.period_days as f64
    }
}

/// Default policy engine backed by a `PolicyConfig`.
///
/// Synchronous — no I/O. All data arrives via method arguments.
pub struct DefaultPolicyEngine {
    config: PolicyConfig,
}

impl DefaultPolicyEngine {
    /// Creates a new engine from a validated `PolicyConfig`.
    pub fn new(config: PolicyConfig) -> Self {
        Self { config }
    }

    /// Resolves which policy to apply for a given request.
    ///
    /// Priority: explicit `policy_name` > tag routing rules > default rule.
    fn resolve_policy_name(&self, request: &RoutingRequest) -> Result<String> {
        // Explicit policy name override
        if let Some(ref name) = request.policy_name {
            if self.config.policies.iter().any(|p| p.name == *name) {
                return Ok(name.clone());
            }
            return Err(HamoruError::NoModelSatisfiesPolicy {
                policy: name.clone(),
                reason: format!(
                    "Policy '{}' not found. Available policies: {}.",
                    name,
                    self.policy_names_display()
                ),
            });
        }

        // Tag-based routing: any-match semantics, first rule wins
        for rule in &self.config.routing_rules {
            if let Some(ref match_rule) = rule.match_rule {
                let matches = request
                    .tags
                    .iter()
                    .any(|tag| match_rule.tags.iter().any(|rt| rt == tag));
                if matches {
                    // Safe: validated at parse time
                    return Ok(rule.policy.clone().unwrap_or_default());
                }
            }
        }

        // Default rule fallback
        for rule in &self.config.routing_rules {
            if let Some(ref default) = rule.default {
                return Ok(default.policy.clone());
            }
        }

        Err(HamoruError::NoModelSatisfiesPolicy {
            policy: "<unresolved>".to_string(),
            reason: format!(
                "No routing rule matches tags {:?} and no default rule is configured. \
                 Add routing_rules to hamoru.policy.yaml or use -p to specify a policy.",
                request.tags
            ),
        })
    }

    /// Simulates which policy would route to a given model under a config.
    ///
    /// Returns the default policy name (first policy or default rule).
    /// This is a simplified simulation — real routing depends on tags.
    fn simulate_routing_for_model(&self, _model_id: &str, config: &PolicyConfig) -> Option<String> {
        // Check default routing rule first
        for rule in &config.routing_rules {
            if let Some(ref default) = rule.default {
                return Some(default.policy.clone());
            }
        }
        // Fall back to first policy
        config.policies.first().map(|p| p.name.clone())
    }

    fn policy_names_display(&self) -> String {
        self.config
            .policies
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl PolicyEngine for DefaultPolicyEngine {
    fn select_model(
        &self,
        request: &RoutingRequest,
        available_models: &[ModelInfo],
        metrics_cache: &MetricsCache,
    ) -> Result<ModelSelection> {
        let policy_name = self.resolve_policy_name(request)?;
        let policy = self
            .config
            .policies
            .iter()
            .find(|p| p.name == policy_name)
            .unwrap(); // Safe: resolve_policy_name verified existence

        // Filter by capabilities
        let mut candidates: Vec<&ModelInfo> = available_models.iter().collect();
        if let Some(ref required_caps) = policy.constraints.required_capabilities {
            candidates.retain(|m| required_caps.iter().all(|rc| m.capabilities.contains(rc)));
        }

        // Filter by max_cost_per_request (estimate using sum of per-token costs)
        if let Some(max_cost) = policy.constraints.max_cost_per_request {
            // Rough estimate: assume 1000 input + 1000 output tokens
            candidates.retain(|m| {
                let estimate = m.cost_per_input_token * 1000.0 + m.cost_per_output_token * 1000.0;
                estimate <= max_cost
            });
        }

        // Filter by max_latency_ms (from historical data)
        if let Some(max_latency) = policy.constraints.max_latency_ms {
            candidates.retain(|m| {
                metrics_cache
                    .by_model
                    .get(&m.id)
                    .is_none_or(|mm| mm.avg_latency_ms <= max_latency as f64)
            });
        }

        // Filter by min_quality_tier
        if let Some(min_tier) = policy.constraints.min_quality_tier {
            candidates.retain(|m| quality_tier(m) >= min_tier);
        }

        if candidates.is_empty() {
            return Err(HamoruError::NoModelSatisfiesPolicy {
                policy: policy_name,
                reason: "No models match the policy constraints. \
                         Check required_capabilities, max_cost_per_request, \
                         max_latency_ms, and min_quality_tier in your policy."
                    .to_string(),
            });
        }

        // Score by priority
        let candidate_infos: Vec<ModelInfo> = candidates.iter().map(|m| (*m).clone()).collect();
        let scores = match policy.preferences.priority {
            Priority::Cost => score_by_cost(&candidate_infos),
            Priority::Quality => score_by_quality(&candidate_infos),
            Priority::Latency => score_by_latency(&candidate_infos, metrics_cache),
            Priority::Balanced => score_balanced(&candidate_infos, metrics_cache),
        };

        // Select the model with the highest score
        let best_idx = scores
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap(); // Safe: candidates is non-empty

        let best = &candidates[best_idx];
        let estimated_cost = if request.estimated_input_tokens.is_some()
            || request.estimated_output_tokens.is_some()
        {
            let input = request.estimated_input_tokens.unwrap_or(0) as f64;
            let output = request.estimated_output_tokens.unwrap_or(0) as f64;
            Some(best.cost_per_input_token * input + best.cost_per_output_token * output)
        } else {
            None
        };

        Ok(ModelSelection {
            provider: best.provider.clone(),
            model: best.id.clone(),
            policy_applied: policy_name,
            reason: format!(
                "{} priority, {} model(s) evaluated",
                format!("{:?}", policy.preferences.priority).to_lowercase(),
                candidates.len()
            ),
            estimated_cost,
            score: scores[best_idx],
        })
    }

    fn select_fallback_model(
        &self,
        original: &ModelSelection,
        _error: &HamoruError,
        available_models: &[ModelInfo],
    ) -> Result<Option<ModelSelection>> {
        // Exclude the original model and re-run selection with the same policy
        let filtered: Vec<ModelInfo> = available_models
            .iter()
            .filter(|m| m.id != original.model)
            .cloned()
            .collect();

        if filtered.is_empty() {
            return Ok(None);
        }

        let request = RoutingRequest {
            policy_name: Some(original.policy_applied.clone()),
            ..Default::default()
        };

        // Use empty metrics for fallback (historical data may be misleading)
        let empty_cache = MetricsCache::default();
        match self.select_model(&request, &filtered, &empty_cache) {
            Ok(selection) => Ok(Some(selection)),
            Err(_) => Ok(None),
        }
    }

    fn check_cost_limits(
        &self,
        estimated_cost: f64,
        metrics_cache: &MetricsCache,
    ) -> Result<CostCheckResult> {
        let limits = match &self.config.cost_limits {
            Some(l) => l,
            None => {
                return Ok(CostCheckResult {
                    allowed: true,
                    ..Default::default()
                });
            }
        };

        let alert_threshold = limits.alert_threshold.unwrap_or(0.8);

        // Check per_request limit
        if let Some(max) = limits.max_cost_per_request
            && estimated_cost > max
        {
            return Ok(CostCheckResult {
                allowed: false,
                limit_exceeded: Some("per_request".to_string()),
                current_spend: estimated_cost,
                max_allowed: max,
                ..Default::default()
            });
        }

        // Check per_day limit using MetricsCache
        if let Some(max_daily) = limits.max_cost_per_day {
            let daily = daily_spend(metrics_cache);
            let projected = daily + estimated_cost;
            if projected > max_daily {
                return Ok(CostCheckResult {
                    allowed: false,
                    limit_exceeded: Some("per_day".to_string()),
                    current_spend: daily,
                    max_allowed: max_daily,
                    ..Default::default()
                });
            }
            // Check alert threshold
            if daily >= alert_threshold * max_daily {
                return Ok(CostCheckResult {
                    allowed: true,
                    current_spend: daily,
                    max_allowed: max_daily,
                    alert: true,
                    alert_message: Some(format!(
                        "Daily spend ${:.4} is at {:.0}% of ${:.2} limit.",
                        daily,
                        (daily / max_daily) * 100.0,
                        max_daily
                    )),
                    ..Default::default()
                });
            }
        }

        // Check per_workflow limit
        if let Some(max) = limits.max_cost_per_workflow
            && estimated_cost > max
        {
            return Ok(CostCheckResult {
                allowed: false,
                limit_exceeded: Some("per_workflow".to_string()),
                current_spend: estimated_cost,
                max_allowed: max,
                ..Default::default()
            });
        }

        // Check per_collaboration limit
        if let Some(max) = limits.max_cost_per_collaboration
            && estimated_cost > max
        {
            return Ok(CostCheckResult {
                allowed: false,
                limit_exceeded: Some("per_collaboration".to_string()),
                current_spend: estimated_cost,
                max_allowed: max,
                ..Default::default()
            });
        }

        Ok(CostCheckResult {
            allowed: true,
            ..Default::default()
        })
    }

    fn simulate_cost_impact(
        &self,
        current_config: &PolicyConfig,
        proposed_config: &PolicyConfig,
        metrics_cache: &MetricsCache,
    ) -> Result<CostImpactReport> {
        use std::time::Duration;

        let mut shifts = Vec::new();
        let mut total_delta = 0.0;

        // For each model with historical traffic, simulate routing under both configs
        for (model_id, model_metrics) in &metrics_cache.by_model {
            let daily_requests = if metrics_cache.period_days > 0 {
                model_metrics.requests as f64 / metrics_cache.period_days as f64
            } else {
                0.0
            };

            if daily_requests == 0.0 {
                continue;
            }

            // Determine which policy would route to this model under each config
            let current_policy = self.simulate_routing_for_model(model_id, current_config);
            let proposed_policy = self.simulate_routing_for_model(model_id, proposed_config);

            if current_policy != proposed_policy {
                // Traffic shifts — estimate cost difference
                let daily_cost = if metrics_cache.period_days > 0 {
                    model_metrics.cost / metrics_cache.period_days as f64
                } else {
                    0.0
                };

                shifts.push(ModelShift {
                    from_model: model_id.clone(),
                    to_model: format!("(routed by {})", proposed_policy.unwrap_or_default()),
                    estimated_percentage: (daily_requests
                        / (metrics_cache.entry_count as f64
                            / metrics_cache.period_days.max(1) as f64))
                        * 100.0,
                    cost_delta: daily_cost * 0.1, // Conservative 10% cost change estimate
                });
                total_delta += daily_cost * 0.1;
            }
        }

        let confidence = f64::min(1.0, metrics_cache.entry_count as f64 / 100.0)
            * f64::min(1.0, metrics_cache.period_days as f64 / 7.0);

        Ok(CostImpactReport {
            estimated_daily_change: total_delta,
            model_shift: shifts,
            confidence,
            period_used: Duration::from_secs(metrics_cache.period_days * 86400),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::config::{
        CostLimits, DefaultPolicy, MatchRule, PolicyConstraints, PolicyDefinition,
        PolicyPreferences, RoutingRule,
    };
    use crate::policy::test_fixtures::*;
    use crate::provider::Capability;

    fn cost_optimized_policy() -> PolicyDefinition {
        PolicyDefinition {
            name: "cost-optimized".to_string(),
            description: Some("Cost-focused".to_string()),
            constraints: PolicyConstraints::default(),
            preferences: PolicyPreferences {
                priority: Priority::Cost,
            },
        }
    }

    fn quality_first_policy() -> PolicyDefinition {
        PolicyDefinition {
            name: "quality-first".to_string(),
            description: Some("Quality-focused".to_string()),
            constraints: PolicyConstraints {
                min_quality_tier: Some(super::super::QualityTier::High),
                ..Default::default()
            },
            preferences: PolicyPreferences {
                priority: Priority::Quality,
            },
        }
    }

    fn basic_config() -> PolicyConfig {
        PolicyConfig {
            policies: vec![cost_optimized_policy(), quality_first_policy()],
            routing_rules: vec![
                RoutingRule {
                    match_rule: Some(MatchRule {
                        tags: vec!["review".to_string(), "architecture".to_string()],
                    }),
                    default: None,
                    policy: Some("quality-first".to_string()),
                },
                RoutingRule {
                    match_rule: None,
                    default: Some(DefaultPolicy {
                        policy: "cost-optimized".to_string(),
                    }),
                    policy: None,
                },
            ],
            cost_limits: None,
        }
    }

    fn all_models() -> Vec<ModelInfo> {
        vec![model_sonnet(), model_haiku(), model_llama_70b()]
    }

    // --- select_model tests ---

    #[test]
    fn explicit_policy_name_selects_correct_model() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let request = RoutingRequest {
            policy_name: Some("cost-optimized".to_string()),
            ..Default::default()
        };
        let selection = engine
            .select_model(&request, &all_models(), &empty_metrics_cache())
            .unwrap();
        // Cost-optimized should pick the cheapest (llama)
        assert_eq!(selection.model, "llama3.3:70b");
        assert_eq!(selection.policy_applied, "cost-optimized");
    }

    #[test]
    fn tag_routing_matches_first_rule() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let request = RoutingRequest {
            tags: vec!["review".to_string()],
            ..Default::default()
        };
        let selection = engine
            .select_model(&request, &all_models(), &empty_metrics_cache())
            .unwrap();
        // "review" tag → quality-first → min_quality_tier: High → sonnet
        assert_eq!(selection.model, "claude-sonnet-4-6");
        assert_eq!(selection.policy_applied, "quality-first");
    }

    #[test]
    fn default_rule_applied_when_no_tags_match() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let request = RoutingRequest {
            tags: vec!["unknown-tag".to_string()],
            ..Default::default()
        };
        let selection = engine
            .select_model(&request, &all_models(), &empty_metrics_cache())
            .unwrap();
        assert_eq!(selection.policy_applied, "cost-optimized");
        assert_eq!(selection.model, "llama3.3:70b");
    }

    #[test]
    fn unknown_policy_name_returns_error() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let request = RoutingRequest {
            policy_name: Some("nonexistent".to_string()),
            ..Default::default()
        };
        let err = engine
            .select_model(&request, &all_models(), &empty_metrics_cache())
            .unwrap_err();
        match err {
            HamoruError::NoModelSatisfiesPolicy { policy, reason } => {
                assert_eq!(policy, "nonexistent");
                assert!(reason.contains("not found"));
            }
            other => panic!("Expected NoModelSatisfiesPolicy, got: {other:?}"),
        }
    }

    #[test]
    fn no_models_survive_filtering_returns_error() {
        // quality-first requires High tier, but only give low-tier models
        let engine = DefaultPolicyEngine::new(basic_config());
        let request = RoutingRequest {
            policy_name: Some("quality-first".to_string()),
            ..Default::default()
        };
        let models = vec![model_llama_70b()]; // Low tier only
        let err = engine
            .select_model(&request, &models, &empty_metrics_cache())
            .unwrap_err();
        match err {
            HamoruError::NoModelSatisfiesPolicy { policy, .. } => {
                assert_eq!(policy, "quality-first");
            }
            other => panic!("Expected NoModelSatisfiesPolicy, got: {other:?}"),
        }
    }

    #[test]
    fn capability_filter_vision_required() {
        let mut config = basic_config();
        config.policies.push(PolicyDefinition {
            name: "vision".to_string(),
            description: None,
            constraints: PolicyConstraints {
                required_capabilities: Some(vec![Capability::Vision]),
                ..Default::default()
            },
            preferences: PolicyPreferences {
                priority: Priority::Quality,
            },
        });
        let engine = DefaultPolicyEngine::new(config);
        let request = RoutingRequest {
            policy_name: Some("vision".to_string()),
            ..Default::default()
        };
        let selection = engine
            .select_model(&request, &all_models(), &empty_metrics_cache())
            .unwrap();
        // Only sonnet has Vision capability
        assert_eq!(selection.model, "claude-sonnet-4-6");
    }

    #[test]
    fn cost_constraint_filters_expensive_models() {
        let config = PolicyConfig {
            policies: vec![PolicyDefinition {
                name: "cheap".to_string(),
                description: None,
                constraints: PolicyConstraints {
                    max_cost_per_request: Some(0.001), // Very low limit
                    ..Default::default()
                },
                preferences: PolicyPreferences {
                    priority: Priority::Cost,
                },
            }],
            routing_rules: vec![],
            cost_limits: None,
        };
        let engine = DefaultPolicyEngine::new(config);
        let request = RoutingRequest {
            policy_name: Some("cheap".to_string()),
            ..Default::default()
        };
        let selection = engine
            .select_model(&request, &all_models(), &empty_metrics_cache())
            .unwrap();
        // Only llama survives the cost filter
        assert_eq!(selection.model, "llama3.3:70b");
    }

    #[test]
    fn latency_constraint_filters_slow_models() {
        let config = PolicyConfig {
            policies: vec![PolicyDefinition {
                name: "fast".to_string(),
                description: None,
                constraints: PolicyConstraints {
                    max_latency_ms: Some(500),
                    ..Default::default()
                },
                preferences: PolicyPreferences {
                    priority: Priority::Latency,
                },
            }],
            routing_rules: vec![],
            cost_limits: None,
        };
        let engine = DefaultPolicyEngine::new(config);
        let request = RoutingRequest {
            policy_name: Some("fast".to_string()),
            ..Default::default()
        };
        let metrics = metrics_cache_with_latency(&[
            ("claude-sonnet-4-6", 2000.0),
            ("claude-haiku-4-5", 800.0),
            ("llama3.3:70b", 100.0),
        ]);
        let selection = engine
            .select_model(&request, &all_models(), &metrics)
            .unwrap();
        // Only llama is under 500ms
        assert_eq!(selection.model, "llama3.3:70b");
    }

    #[test]
    fn quality_tier_constraint_filters_low_tier() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let request = RoutingRequest {
            policy_name: Some("quality-first".to_string()),
            ..Default::default()
        };
        let selection = engine
            .select_model(&request, &all_models(), &empty_metrics_cache())
            .unwrap();
        // quality-first has min_quality_tier: High → only sonnet
        assert_eq!(selection.model, "claude-sonnet-4-6");
    }

    #[test]
    fn uses_historical_latency_for_scoring() {
        let config = PolicyConfig {
            policies: vec![PolicyDefinition {
                name: "fast".to_string(),
                description: None,
                constraints: PolicyConstraints::default(),
                preferences: PolicyPreferences {
                    priority: Priority::Latency,
                },
            }],
            routing_rules: vec![],
            cost_limits: None,
        };
        let engine = DefaultPolicyEngine::new(config);
        let request = RoutingRequest {
            policy_name: Some("fast".to_string()),
            ..Default::default()
        };
        let metrics = metrics_cache_with_latency(&[
            ("claude-sonnet-4-6", 2000.0),
            ("claude-haiku-4-5", 300.0),
            ("llama3.3:70b", 100.0),
        ]);
        let selection = engine
            .select_model(&request, &all_models(), &metrics)
            .unwrap();
        assert_eq!(selection.model, "llama3.3:70b");
    }

    #[test]
    fn reason_includes_policy_name_and_priority() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let request = RoutingRequest {
            policy_name: Some("cost-optimized".to_string()),
            ..Default::default()
        };
        let selection = engine
            .select_model(&request, &all_models(), &empty_metrics_cache())
            .unwrap();
        assert!(selection.reason.contains("cost"));
        assert_eq!(selection.policy_applied, "cost-optimized");
    }

    #[test]
    fn multi_provider_candidates_all_considered() {
        let config = PolicyConfig {
            policies: vec![PolicyDefinition {
                name: "best-quality".to_string(),
                description: None,
                constraints: PolicyConstraints::default(),
                preferences: PolicyPreferences {
                    priority: Priority::Quality,
                },
            }],
            routing_rules: vec![],
            cost_limits: None,
        };
        let engine = DefaultPolicyEngine::new(config);
        let request = RoutingRequest {
            policy_name: Some("best-quality".to_string()),
            ..Default::default()
        };
        // Models from different providers
        let models = vec![model_sonnet(), model_llama_70b()];
        let selection = engine
            .select_model(&request, &models, &empty_metrics_cache())
            .unwrap();
        // Sonnet (High tier, claude provider) beats llama (Low tier, ollama)
        assert_eq!(selection.provider, "claude");
        assert_eq!(selection.model, "claude-sonnet-4-6");
    }

    // --- select_fallback_model tests ---

    #[test]
    fn fallback_excludes_original_model() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let original = ModelSelection {
            provider: "ollama".to_string(),
            model: "llama3.3:70b".to_string(),
            policy_applied: "cost-optimized".to_string(),
            reason: String::new(),
            estimated_cost: None,
            score: 1.0,
        };
        let fallback = engine
            .select_fallback_model(
                &original,
                &HamoruError::ProviderUnavailable {
                    provider: "ollama".to_string(),
                    reason: "test".to_string(),
                },
                &all_models(),
            )
            .unwrap();
        let fb = fallback.unwrap();
        assert_ne!(fb.model, "llama3.3:70b");
    }

    #[test]
    fn fallback_returns_none_when_single_model() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let original = ModelSelection {
            provider: "ollama".to_string(),
            model: "llama3.3:70b".to_string(),
            policy_applied: "cost-optimized".to_string(),
            reason: String::new(),
            estimated_cost: None,
            score: 1.0,
        };
        let models = vec![model_llama_70b()]; // Only the original
        let fallback = engine
            .select_fallback_model(
                &original,
                &HamoruError::ProviderUnavailable {
                    provider: "ollama".to_string(),
                    reason: "test".to_string(),
                },
                &models,
            )
            .unwrap();
        assert!(fallback.is_none());
    }

    #[test]
    fn fallback_picks_next_best() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let original = ModelSelection {
            provider: "ollama".to_string(),
            model: "llama3.3:70b".to_string(),
            policy_applied: "cost-optimized".to_string(),
            reason: String::new(),
            estimated_cost: None,
            score: 1.0,
        };
        let fallback = engine
            .select_fallback_model(
                &original,
                &HamoruError::ProviderUnavailable {
                    provider: "ollama".to_string(),
                    reason: "test".to_string(),
                },
                &all_models(),
            )
            .unwrap()
            .unwrap();
        // Next cheapest after llama is haiku
        assert_eq!(fallback.model, "claude-haiku-4-5");
    }

    #[test]
    fn fallback_preserves_policy_constraints() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let original = ModelSelection {
            provider: "claude".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            policy_applied: "quality-first".to_string(),
            reason: String::new(),
            estimated_cost: None,
            score: 1.0,
        };
        // quality-first requires High tier — no other High tier model
        let fallback = engine
            .select_fallback_model(
                &original,
                &HamoruError::ProviderUnavailable {
                    provider: "claude".to_string(),
                    reason: "test".to_string(),
                },
                &all_models(),
            )
            .unwrap();
        // No High-tier fallback exists → None
        assert!(fallback.is_none());
    }

    // --- check_cost_limits tests ---

    fn config_with_limits(limits: CostLimits) -> PolicyConfig {
        PolicyConfig {
            policies: vec![cost_optimized_policy()],
            routing_rules: vec![],
            cost_limits: Some(limits),
        }
    }

    #[test]
    fn cost_check_no_limits_configured() {
        let engine = DefaultPolicyEngine::new(basic_config()); // no cost_limits
        let result = engine
            .check_cost_limits(1.0, &empty_metrics_cache())
            .unwrap();
        assert!(result.allowed);
        assert!(result.limit_exceeded.is_none());
    }

    #[test]
    fn cost_check_under_daily_limit() {
        let config = config_with_limits(CostLimits {
            max_cost_per_day: Some(10.0),
            ..Default::default()
        });
        let engine = DefaultPolicyEngine::new(config);
        let result = engine
            .check_cost_limits(0.01, &empty_metrics_cache())
            .unwrap();
        assert!(result.allowed);
    }

    #[test]
    fn cost_check_exceeds_daily_limit() {
        let config = config_with_limits(CostLimits {
            max_cost_per_day: Some(1.0),
            ..Default::default()
        });
        let engine = DefaultPolicyEngine::new(config);
        // Create metrics showing $0.9/day spend over 7 days
        let cache = MetricsCache {
            total: Metrics {
                total_cost: 6.3,
                ..Default::default()
            },
            period_days: 7,
            ..Default::default()
        };

        let result = engine.check_cost_limits(0.2, &cache).unwrap();
        assert!(!result.allowed);
        assert_eq!(result.limit_exceeded.as_deref(), Some("per_day"));
    }

    #[test]
    fn cost_check_exceeds_per_request_limit() {
        let config = config_with_limits(CostLimits {
            max_cost_per_request: Some(0.05),
            ..Default::default()
        });
        let engine = DefaultPolicyEngine::new(config);
        let result = engine
            .check_cost_limits(0.10, &empty_metrics_cache())
            .unwrap();
        assert!(!result.allowed);
        assert_eq!(result.limit_exceeded.as_deref(), Some("per_request"));
    }

    #[test]
    fn cost_check_alert_threshold() {
        let config = config_with_limits(CostLimits {
            max_cost_per_day: Some(10.0),
            alert_threshold: Some(0.8), // Alert at 80%
            ..Default::default()
        });
        let engine = DefaultPolicyEngine::new(config);
        // Daily spend = 8.5 (85% of 10.0) → alert
        let cache = MetricsCache {
            total: Metrics {
                total_cost: 59.5,
                ..Default::default()
            },
            period_days: 7,
            ..Default::default()
        };

        let result = engine.check_cost_limits(0.01, &cache).unwrap();
        assert!(result.allowed);
        assert!(result.alert);
        assert!(result.alert_message.is_some());
    }

    #[test]
    fn cost_check_uses_metrics_daily_spend() {
        let config = config_with_limits(CostLimits {
            max_cost_per_day: Some(5.0),
            ..Default::default()
        });
        let engine = DefaultPolicyEngine::new(config);
        let cache = MetricsCache {
            total: Metrics {
                total_cost: 28.0,
                ..Default::default()
            },
            period_days: 7,
            ..Default::default()
        };

        // 4.0 + 0.5 = 4.5 < 5.0 → allowed
        let result = engine.check_cost_limits(0.5, &cache).unwrap();
        assert!(result.allowed);

        // 4.0 + 1.5 = 5.5 > 5.0 → denied
        let result = engine.check_cost_limits(1.5, &cache).unwrap();
        assert!(!result.allowed);
    }

    #[test]
    fn cost_check_period_days_zero() {
        let config = config_with_limits(CostLimits {
            max_cost_per_day: Some(1.0),
            ..Default::default()
        });
        let engine = DefaultPolicyEngine::new(config);
        let cache = MetricsCache {
            total: Metrics {
                total_cost: 100.0,
                ..Default::default()
            },
            period_days: 0,
            ..Default::default()
        };

        // daily_spend = 0.0, so 0.0 + 0.5 = 0.5 < 1.0 → allowed
        let result = engine.check_cost_limits(0.5, &cache).unwrap();
        assert!(result.allowed);
    }

    // --- simulate_cost_impact tests ---

    use crate::telemetry::Metrics;
    use std::collections::HashMap;

    fn metrics_with_models(entries: &[(&str, u64, f64)]) -> MetricsCache {
        let mut by_model = HashMap::new();
        let mut total_cost = 0.0;
        let mut total_requests = 0u64;
        for &(id, requests, cost) in entries {
            total_cost += cost;
            total_requests += requests;
            by_model.insert(
                id.to_string(),
                crate::telemetry::ModelMetrics {
                    provider: String::new(),
                    requests,
                    cost,
                    input_tokens: 0,
                    output_tokens: 0,
                    avg_latency_ms: 0.0,
                },
            );
        }
        MetricsCache {
            total: Metrics {
                total_requests,
                total_cost,
                ..Default::default()
            },
            by_model,
            period_days: 7,
            entry_count: total_requests,
            ..Default::default()
        }
    }

    #[test]
    fn simulate_identical_configs_zero_delta() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let config = basic_config();
        let cache = metrics_with_models(&[("llama3.3:70b", 100, 0.1)]);
        let report = engine
            .simulate_cost_impact(&config, &config, &cache)
            .unwrap();
        assert!((report.estimated_daily_change - 0.0).abs() < f64::EPSILON);
        assert!(report.model_shift.is_empty());
    }

    #[test]
    fn simulate_policy_change_shifts_traffic() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let current = basic_config();
        // Proposed: change default from cost-optimized to quality-first
        let mut proposed = basic_config();
        if let Some(rule) = proposed
            .routing_rules
            .iter_mut()
            .find(|r| r.default.is_some())
        {
            rule.default = Some(DefaultPolicy {
                policy: "quality-first".to_string(),
            });
        }
        let cache = metrics_with_models(&[("llama3.3:70b", 100, 0.7)]);
        let report = engine
            .simulate_cost_impact(&current, &proposed, &cache)
            .unwrap();
        assert!(!report.model_shift.is_empty());
        assert!(report.estimated_daily_change > 0.0);
    }

    #[test]
    fn simulate_empty_metrics_low_confidence() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let config = basic_config();
        let cache = empty_metrics_cache();
        let report = engine
            .simulate_cost_impact(&config, &config, &cache)
            .unwrap();
        assert!((report.confidence - 0.0).abs() < f64::EPSILON);
        assert!((report.estimated_daily_change - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn simulate_cost_delta_sign_positive() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let current = basic_config();
        let mut proposed = basic_config();
        if let Some(rule) = proposed
            .routing_rules
            .iter_mut()
            .find(|r| r.default.is_some())
        {
            rule.default = Some(DefaultPolicy {
                policy: "quality-first".to_string(),
            });
        }
        let cache = metrics_with_models(&[("llama3.3:70b", 50, 0.5)]);
        let report = engine
            .simulate_cost_impact(&current, &proposed, &cache)
            .unwrap();
        // Shifting from cost-optimized to quality-first → positive delta
        assert!(report.estimated_daily_change >= 0.0);
    }

    #[test]
    fn simulate_model_shift_records() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let current = basic_config();
        let mut proposed = basic_config();
        if let Some(rule) = proposed
            .routing_rules
            .iter_mut()
            .find(|r| r.default.is_some())
        {
            rule.default = Some(DefaultPolicy {
                policy: "quality-first".to_string(),
            });
        }
        let cache = metrics_with_models(&[("llama3.3:70b", 70, 0.7)]);
        let report = engine
            .simulate_cost_impact(&current, &proposed, &cache)
            .unwrap();
        for shift in &report.model_shift {
            assert!(!shift.from_model.is_empty());
            assert!(!shift.to_model.is_empty());
        }
    }

    #[test]
    fn simulate_missing_model_in_available_skips() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let config = basic_config();
        // Model in metrics but wouldn't be in available_models
        let cache = metrics_with_models(&[("unknown-model", 10, 0.1)]);
        let report = engine
            .simulate_cost_impact(&config, &config, &cache)
            .unwrap();
        // Should not crash
        assert!(report.model_shift.is_empty());
    }

    #[test]
    fn simulate_confidence_scales_with_entry_count() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let config = basic_config();

        // Low entry count → low confidence
        let cache = MetricsCache {
            entry_count: 10,
            period_days: 7,
            ..Default::default()
        };
        let report = engine
            .simulate_cost_impact(&config, &config, &cache)
            .unwrap();
        assert!(report.confidence < 0.2);

        // High entry count → higher confidence
        let cache = MetricsCache {
            entry_count: 200,
            period_days: 7,
            ..Default::default()
        };
        let report = engine
            .simulate_cost_impact(&config, &config, &cache)
            .unwrap();
        assert!(report.confidence >= 0.9);
    }

    #[test]
    fn simulate_confidence_scales_with_period_days() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let config = basic_config();

        // 1 day → low confidence
        let cache = MetricsCache {
            entry_count: 100,
            period_days: 1,
            ..Default::default()
        };
        let report = engine
            .simulate_cost_impact(&config, &config, &cache)
            .unwrap();
        let conf_1d = report.confidence;

        // 7 days → full confidence
        let cache = MetricsCache {
            entry_count: 100,
            period_days: 7,
            ..Default::default()
        };
        let report = engine
            .simulate_cost_impact(&config, &config, &cache)
            .unwrap();
        let conf_7d = report.confidence;

        assert!(conf_7d > conf_1d);
    }

    #[test]
    fn simulate_multi_model_traffic() {
        let engine = DefaultPolicyEngine::new(basic_config());
        let current = basic_config();
        let mut proposed = basic_config();
        if let Some(rule) = proposed
            .routing_rules
            .iter_mut()
            .find(|r| r.default.is_some())
        {
            rule.default = Some(DefaultPolicy {
                policy: "quality-first".to_string(),
            });
        }
        let cache =
            metrics_with_models(&[("llama3.3:70b", 100, 1.0), ("claude-haiku-4-5", 50, 2.5)]);
        let report = engine
            .simulate_cost_impact(&current, &proposed, &cache)
            .unwrap();
        // Both models may or may not shift depending on routing
        assert!(report.estimated_daily_change >= 0.0);
    }

    #[test]
    fn simulate_same_policy_no_shifts() {
        let config = PolicyConfig {
            policies: vec![cost_optimized_policy()],
            routing_rules: vec![RoutingRule {
                match_rule: None,
                default: Some(DefaultPolicy {
                    policy: "cost-optimized".to_string(),
                }),
                policy: None,
            }],
            cost_limits: None,
        };
        let engine = DefaultPolicyEngine::new(config.clone());
        let cache = metrics_with_models(&[("llama3.3:70b", 100, 1.0)]);
        let report = engine
            .simulate_cost_impact(&config, &config, &cache)
            .unwrap();
        assert!(report.model_shift.is_empty());
        assert!((report.estimated_daily_change - 0.0).abs() < f64::EPSILON);
    }
}
