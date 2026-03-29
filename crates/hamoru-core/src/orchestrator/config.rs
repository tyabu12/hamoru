//! Workflow YAML configuration types and parser.
//!
//! Defines the serde types for deserializing `hamoru.workflow.yaml` files
//! and validation logic for workflow definitions.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::ConditionMode;
use crate::Result;
use crate::error::HamoruError;

/// Default maximum iterations for a workflow.
fn default_max_iterations() -> u32 {
    50
}

/// Top-level workflow definition parsed from YAML.
#[derive(Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    /// Workflow name.
    pub name: String,
    /// Optional human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// Maximum iterations before the workflow terminates with a warning.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Maximum cost in USD for the entire workflow.
    #[serde(default)]
    pub max_cost: Option<f64>,
    /// Workflow-level default condition evaluation mode.
    #[serde(default)]
    pub condition_mode: Option<ConditionMode>,
    /// Steps in the workflow.
    pub steps: Vec<StepConfig>,
}

// Custom Debug for WorkflowConfig omits step instructions to prevent prompt content leakage (Hard Rule 8).
impl fmt::Debug for WorkflowConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkflowConfig")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("max_iterations", &self.max_iterations)
            .field("max_cost", &self.max_cost)
            .field("condition_mode", &self.condition_mode)
            .field("steps", &format!("[{} steps]", self.steps.len()))
            .finish()
    }
}

/// A single step in a workflow YAML.
#[derive(Clone, Serialize, Deserialize)]
pub struct StepConfig {
    /// Step name (used as transition target).
    pub name: String,
    /// Tags for policy-based model selection.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Instruction template (may contain `{task}` and `{previous_output}` placeholders).
    pub instruction: String,
    /// Transitions to other steps or COMPLETE.
    #[serde(default)]
    pub transitions: Vec<TransitionConfig>,
    /// Context policy for this step.
    #[serde(default)]
    pub context_policy: Option<String>,
    /// Number of recent iterations to keep (when `context_policy: keep_last_n`).
    #[serde(default)]
    pub keep_last_n: Option<u32>,
    /// Per-step override of condition evaluation mode.
    #[serde(default)]
    pub condition_mode: Option<ConditionMode>,
}

// Custom Debug for StepConfig omits instruction to prevent prompt content leakage (Hard Rule 8).
impl fmt::Debug for StepConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StepConfig")
            .field("name", &self.name)
            .field("tags", &self.tags)
            .field("instruction", &"<redacted>")
            .field("transitions", &self.transitions)
            .field("context_policy", &self.context_policy)
            .field("keep_last_n", &self.keep_last_n)
            .field("condition_mode", &self.condition_mode)
            .finish()
    }
}

/// A transition between workflow steps in YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionConfig {
    /// Condition value that triggers this transition.
    pub condition: String,
    /// Target step name, or "COMPLETE" to end the workflow.
    pub next: String,
}

impl WorkflowConfig {
    /// Validates the workflow configuration for semantic correctness.
    pub fn validate(&self) -> Result<()> {
        let name = &self.name;

        // At least one step
        if self.steps.is_empty() {
            return Err(HamoruError::WorkflowValidationError {
                workflow: name.clone(),
                reason: "Workflow must have at least one step.".to_string(),
            });
        }

        // max_iterations > 0
        if self.max_iterations == 0 {
            return Err(HamoruError::WorkflowValidationError {
                workflow: name.clone(),
                reason: "max_iterations must be greater than 0.".to_string(),
            });
        }

        // max_cost > 0 and finite if present
        if let Some(cost) = self.max_cost
            && (!cost.is_finite() || cost <= 0.0)
        {
            return Err(HamoruError::WorkflowValidationError {
                workflow: name.clone(),
                reason: "max_cost must be a finite value greater than 0.".to_string(),
            });
        }

        // No duplicate step names
        let mut seen = HashSet::new();
        for step in &self.steps {
            if !seen.insert(&step.name) {
                return Err(HamoruError::WorkflowValidationError {
                    workflow: name.clone(),
                    reason: format!("Duplicate step name '{}'.", step.name),
                });
            }
        }

        // All transition targets reference existing steps or "COMPLETE"
        let step_names: HashSet<&str> = self.steps.iter().map(|s| s.name.as_str()).collect();
        for step in &self.steps {
            for transition in &step.transitions {
                if transition.next != "COMPLETE" && !step_names.contains(transition.next.as_str()) {
                    return Err(HamoruError::WorkflowValidationError {
                        workflow: name.clone(),
                        reason: format!(
                            "Step '{}' references unknown target '{}'. Available steps: {:?}.",
                            step.name,
                            transition.next,
                            step_names.iter().collect::<Vec<_>>()
                        ),
                    });
                }
            }
        }

        // keep_last_n requires context_policy: keep_last_n
        for step in &self.steps {
            if step.keep_last_n.is_some() {
                match &step.context_policy {
                    Some(cp) if cp == "keep_last_n" => {}
                    _ => {
                        return Err(HamoruError::WorkflowValidationError {
                            workflow: name.clone(),
                            reason: format!(
                                "Step '{}' has keep_last_n without context_policy: keep_last_n.",
                                step.name
                            ),
                        });
                    }
                }
            }
        }

        Ok(())
    }
}

/// Parses a workflow definition from a YAML string.
pub fn parse_workflow(yaml: &str) -> Result<WorkflowConfig> {
    let config: WorkflowConfig =
        serde_yaml::from_str(yaml).map_err(|e| HamoruError::WorkflowValidationError {
            workflow: "<unknown>".to_string(),
            reason: format!("YAML parse error: {e}"),
        })?;
    config.validate()?;
    Ok(config)
}

/// Loads a workflow definition from a YAML file.
pub fn load_workflow(path: &Path) -> Result<WorkflowConfig> {
    let content =
        std::fs::read_to_string(path).map_err(|e| HamoruError::WorkflowValidationError {
            workflow: path.display().to_string(),
            reason: format!("Failed to read workflow file: {e}"),
        })?;
    let config: WorkflowConfig =
        serde_yaml::from_str(&content).map_err(|e| HamoruError::WorkflowValidationError {
            workflow: path.display().to_string(),
            reason: format!("YAML parse error: {e}"),
        })?;
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANONICAL_YAML: &str = r#"
name: generate-and-review
description: Code generation → review → revision loop
max_iterations: 10
max_cost: 1.00

steps:
  - name: generate
    tags: [generation]
    instruction: |
      {task}
    transitions:
      - condition: done
        next: review

  - name: review
    tags: [review, architecture]
    instruction: |
      Please review the following output:
      {previous_output}
    transitions:
      - condition: approved
        next: COMPLETE
      - condition: improve
        next: generate
"#;

    #[test]
    fn parse_canonical_workflow() {
        let config = parse_workflow(CANONICAL_YAML).unwrap();
        assert_eq!(config.name, "generate-and-review");
        assert_eq!(
            config.description.as_deref(),
            Some("Code generation → review → revision loop")
        );
        assert_eq!(config.max_iterations, 10);
        assert_eq!(config.max_cost, Some(1.00));
        assert_eq!(config.steps.len(), 2);

        let generate = &config.steps[0];
        assert_eq!(generate.name, "generate");
        assert_eq!(generate.tags, vec!["generation"]);
        assert_eq!(generate.transitions.len(), 1);
        assert_eq!(generate.transitions[0].condition, "done");
        assert_eq!(generate.transitions[0].next, "review");

        let review = &config.steps[1];
        assert_eq!(review.name, "review");
        assert_eq!(review.tags, vec!["review", "architecture"]);
        assert_eq!(review.transitions.len(), 2);
        assert_eq!(review.transitions[0].next, "COMPLETE");
        assert_eq!(review.transitions[1].next, "generate");
    }

    #[test]
    fn parse_minimal_workflow() {
        let yaml = r#"
name: simple
steps:
  - name: run
    instruction: "Do the thing"
"#;
        let config = parse_workflow(yaml).unwrap();
        assert_eq!(config.name, "simple");
        assert_eq!(config.max_iterations, 50); // default
        assert!(config.max_cost.is_none());
        assert!(config.description.is_none());
        assert_eq!(config.steps.len(), 1);
        assert!(config.steps[0].transitions.is_empty());
    }

    #[test]
    fn parse_with_all_optional_fields() {
        let yaml = r#"
name: full
max_iterations: 5
max_cost: 2.50
condition_mode: status_line
steps:
  - name: step1
    tags: [review]
    instruction: "Do it"
    context_policy: keep_last_n
    keep_last_n: 3
    condition_mode: tool_calling
    transitions:
      - condition: done
        next: COMPLETE
"#;
        let config = parse_workflow(yaml).unwrap();
        assert_eq!(config.max_iterations, 5);
        assert_eq!(config.max_cost, Some(2.50));
        assert_eq!(config.condition_mode, Some(ConditionMode::StatusLine));

        let step = &config.steps[0];
        assert_eq!(step.context_policy.as_deref(), Some("keep_last_n"));
        assert_eq!(step.keep_last_n, Some(3));
        assert_eq!(step.condition_mode, Some(ConditionMode::ToolCalling));
    }

    #[test]
    fn validation_empty_steps() {
        let yaml = r#"
name: empty
steps: []
"#;
        let err = parse_workflow(yaml).unwrap_err();
        assert!(err.to_string().contains("at least one step"));
    }

    #[test]
    fn validation_duplicate_step_names() {
        let yaml = r#"
name: dup
steps:
  - name: step1
    instruction: "first"
  - name: step1
    instruction: "second"
"#;
        let err = parse_workflow(yaml).unwrap_err();
        assert!(err.to_string().contains("Duplicate step name 'step1'"));
    }

    #[test]
    fn validation_invalid_transition_target() {
        let yaml = r#"
name: bad-target
steps:
  - name: step1
    instruction: "do"
    transitions:
      - condition: done
        next: nonexistent
"#;
        let err = parse_workflow(yaml).unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn validation_orphan_keep_last_n() {
        let yaml = r#"
name: orphan
steps:
  - name: step1
    instruction: "do"
    keep_last_n: 3
"#;
        let err = parse_workflow(yaml).unwrap_err();
        assert!(
            err.to_string()
                .contains("keep_last_n without context_policy")
        );
    }

    #[test]
    fn validation_max_iterations_zero() {
        let yaml = r#"
name: zero-iter
max_iterations: 0
steps:
  - name: step1
    instruction: "do"
"#;
        let err = parse_workflow(yaml).unwrap_err();
        assert!(
            err.to_string()
                .contains("max_iterations must be greater than 0")
        );
    }

    #[test]
    fn validation_negative_max_cost() {
        let yaml = r#"
name: neg-cost
max_cost: -1.0
steps:
  - name: step1
    instruction: "do"
"#;
        let err = parse_workflow(yaml).unwrap_err();
        assert!(
            err.to_string()
                .contains("max_cost must be a finite value greater than 0")
        );
    }

    #[test]
    fn validation_nan_max_cost() {
        let mut config = WorkflowConfig {
            name: "nan".to_string(),
            description: None,
            max_iterations: 10,
            max_cost: Some(f64::NAN),
            condition_mode: None,
            steps: vec![StepConfig {
                name: "s1".to_string(),
                tags: vec![],
                instruction: "do".to_string(),
                transitions: vec![],
                context_policy: None,
                keep_last_n: None,
                condition_mode: None,
            }],
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("finite"));

        config.max_cost = Some(f64::INFINITY);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("finite"));
    }

    #[test]
    fn serialize_deserialize_roundtrip() {
        let config = parse_workflow(CANONICAL_YAML).unwrap();
        let serialized = serde_yaml::to_string(&config).unwrap();
        let deserialized: WorkflowConfig = serde_yaml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.name, config.name);
        assert_eq!(deserialized.max_iterations, config.max_iterations);
        assert_eq!(deserialized.steps.len(), config.steps.len());
    }
}
