//! Policy configuration types and YAML parsing.
//!
//! Parses `hamoru.policy.yaml` into typed configuration for the Policy Engine.
//! All types derive both `Serialize` and `Deserialize` for round-trip support.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::error::HamoruError;
use crate::provider::Capability;

/// Top-level policy configuration parsed from `hamoru.policy.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Named policy definitions.
    pub policies: Vec<PolicyDefinition>,
    /// Rules mapping task tags to policies.
    #[serde(default)]
    pub routing_rules: Vec<RoutingRule>,
    /// Global cost guardrails.
    #[serde(default)]
    pub cost_limits: Option<CostLimits>,
}

impl PolicyConfig {
    /// Validates the policy configuration for internal consistency.
    ///
    /// Checks: at least one policy, no duplicates, routing rules reference
    /// existing policies, at most one default rule, and structural correctness.
    pub fn validate(&self) -> Result<()> {
        if self.policies.is_empty() {
            return Err(HamoruError::ConfigError {
                reason: "Policy config must define at least one policy.".to_string(),
            });
        }

        // Check duplicate policy names
        let mut seen = HashSet::new();
        for policy in &self.policies {
            if !seen.insert(&policy.name) {
                return Err(HamoruError::ConfigError {
                    reason: format!(
                        "Duplicate policy name '{}'. Each policy must have a unique name.",
                        policy.name
                    ),
                });
            }
        }

        let policy_names: HashSet<&str> = self.policies.iter().map(|p| p.name.as_str()).collect();
        let mut default_count = 0;

        for (i, rule) in self.routing_rules.iter().enumerate() {
            let has_match = rule.match_rule.is_some();
            let has_default = rule.default.is_some();

            if has_match == has_default {
                return Err(HamoruError::ConfigError {
                    reason: format!(
                        "Routing rule {} must have exactly one of 'match' or 'default'.",
                        i + 1
                    ),
                });
            }

            if has_match {
                // Match rules must have a top-level `policy` field
                let policy_name =
                    rule.policy
                        .as_deref()
                        .ok_or_else(|| HamoruError::ConfigError {
                            reason: format!(
                                "Routing rule {} has 'match' but no 'policy' field. \
                             Add policy: <name> alongside the match.",
                                i + 1
                            ),
                        })?;
                if !policy_names.contains(policy_name) {
                    return Err(HamoruError::ConfigError {
                        reason: format!(
                            "Routing rule {} references undefined policy '{}'. \
                             Available policies: {}.",
                            i + 1,
                            policy_name,
                            policy_names.iter().copied().collect::<Vec<_>>().join(", ")
                        ),
                    });
                }
            }

            if has_default {
                default_count += 1;
                if default_count > 1 {
                    return Err(HamoruError::ConfigError {
                        reason: "At most one default routing rule is allowed.".to_string(),
                    });
                }
                // Validate the default rule's policy reference
                let default_policy = &rule.default.as_ref().unwrap().policy;
                if !policy_names.contains(default_policy.as_str()) {
                    return Err(HamoruError::ConfigError {
                        reason: format!(
                            "Default routing rule references undefined policy '{}'. \
                             Available policies: {}.",
                            default_policy,
                            policy_names.iter().copied().collect::<Vec<_>>().join(", ")
                        ),
                    });
                }
                // Default rules should not have a top-level `policy` field
                if rule.policy.is_some() {
                    return Err(HamoruError::ConfigError {
                        reason: format!(
                            "Routing rule {} is a default rule but also has a top-level \
                             'policy' field. Remove the top-level policy; the default's \
                             policy is inside the 'default' object.",
                            i + 1
                        ),
                    });
                }
            }
        }

        Ok(())
    }
}

/// A named policy with constraints and preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDefinition {
    /// Unique policy name (e.g., "cost-optimized", "quality-first").
    pub name: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// Constraints that filter eligible models.
    #[serde(default)]
    pub constraints: PolicyConstraints,
    /// Scoring preferences for ranking eligible models.
    pub preferences: PolicyPreferences,
}

/// Constraints that narrow the set of eligible models.
///
/// All fields are optional — omitted constraints are not enforced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicyConstraints {
    /// Maximum cost per request in USD.
    pub max_cost_per_request: Option<f64>,
    /// Maximum acceptable latency in milliseconds.
    pub max_latency_ms: Option<u64>,
    /// Minimum quality tier (models below this tier are excluded).
    pub min_quality_tier: Option<QualityTier>,
    /// Required model capabilities (models must support ALL listed).
    pub required_capabilities: Option<Vec<Capability>>,
}

/// Quality tier for cost-based model classification.
///
/// Ordered: `Low < Medium < High`. Derived from output token cost thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QualityTier {
    /// Free or very cheap models (local inference, etc.).
    Low,
    /// Mid-range models (e.g., Haiku-class).
    Medium,
    /// Premium models (e.g., Sonnet-class and above).
    High,
}

/// How to rank eligible models after constraint filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Prefer the cheapest model.
    Cost,
    /// Prefer the highest-quality model.
    Quality,
    /// Prefer the lowest-latency model.
    Latency,
    /// Balanced weighting of quality, cost, and latency.
    Balanced,
}

/// Scoring preferences within a policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyPreferences {
    /// Primary ranking criterion.
    pub priority: Priority,
}

/// A routing rule that maps tags (or a default) to a policy.
///
/// Exactly one of `match_rule` or `default` must be set.
/// For match rules, `policy` is the top-level policy name.
/// For default rules, the policy is inside `default.policy`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    /// Match clause — present for tag-based rules.
    #[serde(rename = "match", default)]
    pub match_rule: Option<MatchRule>,
    /// Default clause — present for the fallback rule.
    #[serde(default)]
    pub default: Option<DefaultPolicy>,
    /// Policy name for match rules (top-level, not inside `match`).
    #[serde(default)]
    pub policy: Option<String>,
}

/// Tag matching criteria within a routing rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchRule {
    /// Tags to match against (any-match semantics).
    pub tags: Vec<String>,
}

/// Default routing rule fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultPolicy {
    /// Policy to apply when no match rules hit.
    pub policy: String,
}

/// Global cost guardrails applied across all requests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostLimits {
    /// Maximum cost for a single request in USD.
    pub max_cost_per_request: Option<f64>,
    /// Maximum cost for an entire workflow execution in USD.
    pub max_cost_per_workflow: Option<f64>,
    /// Maximum cost for an agent collaboration session in USD.
    pub max_cost_per_collaboration: Option<f64>,
    /// Maximum daily cost in USD.
    pub max_cost_per_day: Option<f64>,
    /// Alert threshold as a fraction (e.g., 0.8 = alert at 80% of limit).
    pub alert_threshold: Option<f64>,
}

/// Parses a YAML string into a validated `PolicyConfig`.
pub fn parse_policy_config(yaml: &str) -> Result<PolicyConfig> {
    let config: PolicyConfig =
        serde_yaml::from_str(yaml).map_err(|e| HamoruError::ConfigError {
            reason: format!("Invalid policy YAML: {e}"),
        })?;
    config.validate()?;
    Ok(config)
}

/// Loads and validates a `PolicyConfig` from a YAML file.
pub fn load_policy_config(path: &Path) -> Result<PolicyConfig> {
    let content = std::fs::read_to_string(path).map_err(|e| HamoruError::ConfigError {
        reason: format!(
            "Failed to read policy config '{}': {}. \
                 Run 'hamoru init' to create one.",
            path.display(),
            e
        ),
    })?;
    parse_policy_config(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_full_config() {
        let yaml = r#"
policies:
  - name: cost-optimized
    description: Cost-focused routing
    constraints:
      max_cost_per_request: 0.01
      max_latency_ms: 5000
    preferences:
      priority: cost
  - name: quality-first
    description: Quality-focused
    constraints:
      min_quality_tier: high
    preferences:
      priority: quality
routing_rules:
  - match:
      tags: [review, architecture]
    policy: quality-first
  - default:
      policy: cost-optimized
cost_limits:
  max_cost_per_workflow: 1.00
  max_cost_per_collaboration: 2.00
  max_cost_per_day: 10.00
  alert_threshold: 0.8
"#;
        let config = parse_policy_config(yaml).unwrap();
        assert_eq!(config.policies.len(), 2);
        assert_eq!(config.policies[0].name, "cost-optimized");
        assert_eq!(config.policies[1].name, "quality-first");
        assert_eq!(config.routing_rules.len(), 2);
        let limits = config.cost_limits.unwrap();
        assert_eq!(limits.max_cost_per_day, Some(10.00));
        assert_eq!(limits.alert_threshold, Some(0.8));
    }

    #[test]
    fn parse_minimal_config() {
        let yaml = r#"
policies:
  - name: default
    preferences:
      priority: balanced
"#;
        let config = parse_policy_config(yaml).unwrap();
        assert_eq!(config.policies.len(), 1);
        assert!(config.routing_rules.is_empty());
        assert!(config.cost_limits.is_none());
        assert!(config.policies[0].description.is_none());
        assert!(
            config.policies[0]
                .constraints
                .max_cost_per_request
                .is_none()
        );
    }

    #[test]
    fn parse_config_with_cost_limits() {
        let yaml = r#"
policies:
  - name: p1
    preferences:
      priority: cost
cost_limits:
  max_cost_per_request: 0.05
  max_cost_per_workflow: 1.50
  max_cost_per_collaboration: 3.00
  max_cost_per_day: 20.00
  alert_threshold: 0.75
"#;
        let config = parse_policy_config(yaml).unwrap();
        let limits = config.cost_limits.unwrap();
        assert_eq!(limits.max_cost_per_request, Some(0.05));
        assert_eq!(limits.max_cost_per_workflow, Some(1.50));
        assert_eq!(limits.max_cost_per_collaboration, Some(3.00));
        assert_eq!(limits.max_cost_per_day, Some(20.00));
        assert_eq!(limits.alert_threshold, Some(0.75));
    }

    #[test]
    fn parse_match_and_default_routing_rules() {
        let yaml = r#"
policies:
  - name: fast
    preferences:
      priority: latency
  - name: cheap
    preferences:
      priority: cost
routing_rules:
  - match:
      tags: [urgent, realtime]
    policy: fast
  - default:
      policy: cheap
"#;
        let config = parse_policy_config(yaml).unwrap();
        assert_eq!(config.routing_rules.len(), 2);

        let rule0 = &config.routing_rules[0];
        assert!(rule0.match_rule.is_some());
        assert_eq!(
            rule0.match_rule.as_ref().unwrap().tags,
            vec!["urgent", "realtime"]
        );
        assert_eq!(rule0.policy.as_deref(), Some("fast"));

        let rule1 = &config.routing_rules[1];
        assert!(rule1.default.is_some());
        assert_eq!(rule1.default.as_ref().unwrap().policy, "cheap");
    }

    #[test]
    fn match_form_policy_at_top_level() {
        let yaml = r#"
policies:
  - name: quality-first
    preferences:
      priority: quality
routing_rules:
  - match:
      tags: [review]
    policy: quality-first
"#;
        let config = parse_policy_config(yaml).unwrap();
        let rule = &config.routing_rules[0];
        assert!(rule.match_rule.is_some());
        assert_eq!(rule.policy.as_deref(), Some("quality-first"));
        assert!(rule.default.is_none());
    }

    #[test]
    fn default_form_nested_policy() {
        let yaml = r#"
policies:
  - name: fallback
    preferences:
      priority: cost
routing_rules:
  - default:
      policy: fallback
"#;
        let config = parse_policy_config(yaml).unwrap();
        let rule = &config.routing_rules[0];
        assert!(rule.default.is_some());
        assert_eq!(rule.default.as_ref().unwrap().policy, "fallback");
        assert!(rule.policy.is_none());
        assert!(rule.match_rule.is_none());
    }

    #[test]
    fn parse_required_capabilities() {
        let yaml = r#"
policies:
  - name: vision-capable
    constraints:
      required_capabilities: [Vision]
    preferences:
      priority: quality
"#;
        let config = parse_policy_config(yaml).unwrap();
        let caps = config.policies[0]
            .constraints
            .required_capabilities
            .as_ref()
            .unwrap();
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0], Capability::Vision);
    }

    #[test]
    fn parse_quality_tiers() {
        let yaml = r#"
policies:
  - name: low-tier
    constraints:
      min_quality_tier: low
    preferences:
      priority: cost
  - name: med-tier
    constraints:
      min_quality_tier: medium
    preferences:
      priority: balanced
  - name: high-tier
    constraints:
      min_quality_tier: high
    preferences:
      priority: quality
"#;
        let config = parse_policy_config(yaml).unwrap();
        assert_eq!(
            config.policies[0].constraints.min_quality_tier,
            Some(QualityTier::Low)
        );
        assert_eq!(
            config.policies[1].constraints.min_quality_tier,
            Some(QualityTier::Medium)
        );
        assert_eq!(
            config.policies[2].constraints.min_quality_tier,
            Some(QualityTier::High)
        );
        // Verify ordering
        assert!(QualityTier::Low < QualityTier::Medium);
        assert!(QualityTier::Medium < QualityTier::High);
    }

    #[test]
    fn parse_all_priority_variants() {
        for (yaml_val, expected) in [
            ("cost", Priority::Cost),
            ("quality", Priority::Quality),
            ("latency", Priority::Latency),
            ("balanced", Priority::Balanced),
        ] {
            let yaml =
                format!("policies:\n  - name: p\n    preferences:\n      priority: {yaml_val}\n");
            let config = parse_policy_config(&yaml).unwrap();
            assert_eq!(config.policies[0].preferences.priority, expected);
        }
    }

    #[test]
    fn invalid_yaml_returns_config_error() {
        let yaml = "not: [valid: yaml: {{";
        let err = parse_policy_config(yaml).unwrap_err();
        match err {
            HamoruError::ConfigError { reason } => {
                assert!(reason.contains("Invalid policy YAML"));
            }
            other => panic!("Expected ConfigError, got: {other:?}"),
        }
    }

    #[test]
    fn unknown_priority_returns_error() {
        let yaml = r#"
policies:
  - name: p
    preferences:
      priority: turbo
"#;
        let err = parse_policy_config(yaml).unwrap_err();
        match err {
            HamoruError::ConfigError { reason } => {
                assert!(reason.contains("Invalid policy YAML"));
            }
            other => panic!("Expected ConfigError, got: {other:?}"),
        }
    }

    #[test]
    fn routing_rule_references_undefined_policy() {
        let yaml = r#"
policies:
  - name: existing
    preferences:
      priority: cost
routing_rules:
  - match:
      tags: [test]
    policy: nonexistent
"#;
        let err = parse_policy_config(yaml).unwrap_err();
        match err {
            HamoruError::ConfigError { reason } => {
                assert!(reason.contains("nonexistent"));
                assert!(reason.contains("undefined policy"));
            }
            other => panic!("Expected ConfigError, got: {other:?}"),
        }
    }

    #[test]
    fn duplicate_policy_names_error() {
        let yaml = r#"
policies:
  - name: same-name
    preferences:
      priority: cost
  - name: same-name
    preferences:
      priority: quality
"#;
        let err = parse_policy_config(yaml).unwrap_err();
        match err {
            HamoruError::ConfigError { reason } => {
                assert!(reason.contains("Duplicate policy name"));
                assert!(reason.contains("same-name"));
            }
            other => panic!("Expected ConfigError, got: {other:?}"),
        }
    }

    #[test]
    fn multiple_default_rules_error() {
        let yaml = r#"
policies:
  - name: a
    preferences:
      priority: cost
  - name: b
    preferences:
      priority: quality
routing_rules:
  - default:
      policy: a
  - default:
      policy: b
"#;
        let err = parse_policy_config(yaml).unwrap_err();
        match err {
            HamoruError::ConfigError { reason } => {
                assert!(reason.contains("At most one default"));
            }
            other => panic!("Expected ConfigError, got: {other:?}"),
        }
    }

    #[test]
    fn empty_policies_list_error() {
        let yaml = "policies: []\n";
        let err = parse_policy_config(yaml).unwrap_err();
        match err {
            HamoruError::ConfigError { reason } => {
                assert!(reason.contains("at least one policy"));
            }
            other => panic!("Expected ConfigError, got: {other:?}"),
        }
    }
}
