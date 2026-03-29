//! Layer 4: Orchestration Engine.
//!
//! Executes multi-step workflows as a DAG. Each step invokes an LLM via
//! the Policy Engine's model selection, with transitions driven by
//! condition evaluation (tool calling or status line parsing).

pub mod condition;
pub mod config;
pub mod context;
pub mod engine;

use std::fmt;
use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::error::{HamoruError, StepResult};
use crate::policy::PolicyEngine;
use crate::provider::{ProviderRegistry, TokenUsage};
use crate::telemetry::TelemetryStore;

pub use config::{load_workflow, parse_workflow};
pub use engine::DefaultOrchestrationEngine;

/// Workflow execution engine.
///
/// Loads workflow definitions from YAML and executes them step by step,
/// using the Policy Engine for model selection and Telemetry for recording.
#[async_trait]
pub trait OrchestrationEngine: Send + Sync {
    /// Loads a workflow definition from a YAML file.
    fn load_workflow(&self, path: &Path) -> Result<Workflow>;

    /// Executes a workflow with the given task prompt.
    async fn execute(
        &self,
        workflow: &Workflow,
        task: &str,
        policy_engine: &dyn PolicyEngine,
        providers: &ProviderRegistry,
        telemetry: &dyn TelemetryStore,
    ) -> Result<ExecutionResult>;
}

/// A complete workflow definition, validated and ready for execution.
#[derive(Clone, Serialize)]
pub struct Workflow {
    /// Workflow name.
    pub name: String,
    /// Optional human-readable description.
    pub description: Option<String>,
    /// Maximum iterations before the workflow terminates with a warning.
    pub max_iterations: u32,
    /// Maximum cost in USD for the entire workflow.
    pub max_cost: Option<f64>,
    /// Default condition evaluation mode for steps that don't override.
    pub default_condition_mode: ConditionMode,
    /// Steps in the workflow.
    pub steps: Vec<WorkflowStep>,
}

// Custom Debug for Workflow omits step instructions to prevent prompt content leakage (Hard Rule 8).
impl fmt::Debug for Workflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Workflow")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("max_iterations", &self.max_iterations)
            .field("max_cost", &self.max_cost)
            .field("default_condition_mode", &self.default_condition_mode)
            .field("steps", &format!("[{} steps]", self.steps.len()))
            .finish()
    }
}

/// A single step within a workflow.
#[derive(Clone, Serialize)]
pub struct WorkflowStep {
    /// Step name (used as transition target).
    pub name: String,
    /// Tags for policy-based model selection.
    pub tags: Vec<String>,
    /// Instruction template (may contain `{task}` and `{previous_output}` placeholders).
    pub instruction: String,
    /// Transitions to other steps or COMPLETE.
    pub transitions: Vec<Transition>,
    /// Context policy for this step.
    pub context_policy: ContextPolicy,
    /// Condition evaluation mode for this step.
    pub condition_mode: ConditionMode,
}

// Custom Debug for WorkflowStep omits instruction to prevent prompt content leakage (Hard Rule 8).
impl fmt::Debug for WorkflowStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkflowStep")
            .field("name", &self.name)
            .field("tags", &self.tags)
            .field("instruction", &"<redacted>")
            .field("transitions", &self.transitions)
            .field("context_policy", &self.context_policy)
            .field("condition_mode", &self.condition_mode)
            .finish()
    }
}

/// A transition between workflow steps, triggered by condition evaluation.
#[derive(Debug, Clone, Serialize)]
pub struct Transition {
    /// Condition value that triggers this transition (e.g., "approved", "improve").
    pub condition: String,
    /// Where this transition leads.
    pub next: TransitionTarget,
}

/// Where a transition leads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum TransitionTarget {
    /// Transition to a named step.
    Step(String),
    /// Workflow is complete.
    Complete,
}

/// Declarative control over message history before step execution.
///
/// Applied by Layer 4 before each step to manage context window usage.
/// Note: `SummarizeOnOverflow` is NOT here — it requires LLM calls and is
/// handled by Layer 5 inserting summary steps into the DAG at compile time.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextPolicy {
    /// Keep all message history (default).
    #[default]
    KeepAll,
    /// Keep only the last N iteration outputs.
    KeepLastN {
        /// Number of recent iterations to retain.
        n: u32,
    },
}

/// How workflow step transitions evaluate conditions.
///
/// Determines how the orchestrator interprets the LLM's output to decide
/// which transition to follow.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionMode {
    /// Use tool calling with `report_status` tool (default, more robust).
    #[default]
    ToolCalling,
    /// Parse STATUS lines from the LLM's text output (fallback for models
    /// without tool support).
    StatusLine,
}

/// Parsed output from condition evaluation.
///
/// Contains the extracted status, optional reason (tool calling mode only),
/// and the content body.
#[derive(Clone, Serialize)]
pub struct StepOutput {
    /// Complete LLM output including the STATUS line or tool call.
    pub full_content: String,
    /// Extracted status value (e.g., "approved", "improve", "done").
    pub status: String,
    /// Body content excluding the STATUS line.
    pub content: String,
    /// Reason from tool calling mode (None in status line mode).
    pub reason: Option<String>,
}

// Custom Debug for StepOutput omits prompt content (Hard Rule 8).
impl fmt::Debug for StepOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StepOutput")
            .field("full_content", &"<redacted>")
            .field("status", &self.status)
            .field("content", &"<redacted>")
            .field("reason", &self.reason)
            .finish()
    }
}

/// Why the workflow terminated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum TerminationReason {
    /// Workflow completed normally via a COMPLETE transition.
    Completed,
    /// Workflow reached its maximum iteration count.
    /// Per design-plan.md §11.3, this is a warning, not an error.
    MaxIterationsReached {
        /// Maximum iterations allowed.
        max: u32,
    },
}

/// The result of executing a complete workflow.
#[derive(Clone, Serialize)]
pub struct ExecutionResult {
    /// Results from each step that was executed.
    pub steps_executed: Vec<StepResult>,
    /// Total cost in USD across all steps.
    pub total_cost: f64,
    /// Total token usage across all steps.
    pub total_tokens: TokenUsage,
    /// Total wall-clock latency in milliseconds.
    pub total_latency_ms: u64,
    /// Final output text from the last step.
    pub final_output: String,
    /// Why the workflow terminated.
    pub terminated_reason: TerminationReason,
}

// Custom Debug for ExecutionResult omits final_output to prevent prompt content leakage (Hard Rule 8).
impl fmt::Debug for ExecutionResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutionResult")
            .field(
                "steps_executed",
                &format!("[{} steps]", self.steps_executed.len()),
            )
            .field("total_cost", &self.total_cost)
            .field("total_tokens", &self.total_tokens)
            .field("total_latency_ms", &self.total_latency_ms)
            .field("final_output", &"<redacted>")
            .field("terminated_reason", &self.terminated_reason)
            .finish()
    }
}

/// Converts a `WorkflowConfig` into a validated runtime `Workflow`.
impl TryFrom<config::WorkflowConfig> for Workflow {
    type Error = HamoruError;

    fn try_from(config: config::WorkflowConfig) -> std::result::Result<Self, Self::Error> {
        let default_mode = config.condition_mode.clone().unwrap_or_default();

        let steps = config
            .steps
            .into_iter()
            .map(|step| {
                let condition_mode = step.condition_mode.unwrap_or_else(|| default_mode.clone());

                let context_policy = match step.context_policy.as_deref() {
                    Some("keep_last_n") => ContextPolicy::KeepLastN {
                        n: step.keep_last_n.unwrap_or(1),
                    },
                    Some("keep_all") | None => ContextPolicy::KeepAll,
                    Some(other) => {
                        return Err(HamoruError::WorkflowValidationError {
                            workflow: config.name.clone(),
                            reason: format!(
                                "Unknown context_policy '{}' on step '{}'. \
                                 Valid values: keep_all, keep_last_n.",
                                other, step.name
                            ),
                        });
                    }
                };

                let transitions = step
                    .transitions
                    .into_iter()
                    .map(|t| Transition {
                        condition: t.condition,
                        next: if t.next == "COMPLETE" {
                            TransitionTarget::Complete
                        } else {
                            TransitionTarget::Step(t.next)
                        },
                    })
                    .collect();

                Ok(WorkflowStep {
                    name: step.name,
                    tags: step.tags,
                    instruction: step.instruction,
                    transitions,
                    context_policy,
                    condition_mode,
                })
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(Workflow {
            name: config.name,
            description: config.description,
            max_iterations: config.max_iterations,
            max_cost: config.max_cost,
            default_condition_mode: default_mode,
            steps,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::parse_workflow;

    #[test]
    fn conversion_happy_path() {
        let yaml = r#"
name: test
steps:
  - name: step1
    tags: [review]
    instruction: "Do {task}"
    transitions:
      - condition: done
        next: COMPLETE
"#;
        let config = parse_workflow(yaml).unwrap();
        let workflow = Workflow::try_from(config).unwrap();
        assert_eq!(workflow.name, "test");
        assert_eq!(workflow.steps.len(), 1);
        assert_eq!(workflow.steps[0].name, "step1");
        assert_eq!(workflow.steps[0].tags, vec!["review"]);
        assert_eq!(workflow.default_condition_mode, ConditionMode::ToolCalling);
    }

    #[test]
    fn complete_target_parsing() {
        let yaml = r#"
name: test
steps:
  - name: s1
    instruction: "do"
    transitions:
      - condition: approved
        next: COMPLETE
      - condition: improve
        next: s1
"#;
        let config = parse_workflow(yaml).unwrap();
        let workflow = Workflow::try_from(config).unwrap();
        assert_eq!(
            workflow.steps[0].transitions[0].next,
            TransitionTarget::Complete
        );
        assert_eq!(
            workflow.steps[0].transitions[1].next,
            TransitionTarget::Step("s1".to_string())
        );
    }

    #[test]
    fn condition_mode_inheritance() {
        let yaml = r#"
name: test
condition_mode: status_line
steps:
  - name: s1
    instruction: "do"
  - name: s2
    instruction: "do"
    condition_mode: tool_calling
"#;
        let config = parse_workflow(yaml).unwrap();
        let workflow = Workflow::try_from(config).unwrap();
        // s1 inherits workflow-level default
        assert_eq!(workflow.steps[0].condition_mode, ConditionMode::StatusLine);
        // s2 overrides
        assert_eq!(workflow.steps[1].condition_mode, ConditionMode::ToolCalling);
    }

    #[test]
    fn context_policy_merging() {
        let yaml = r#"
name: test
steps:
  - name: s1
    instruction: "do"
    context_policy: keep_last_n
    keep_last_n: 3
  - name: s2
    instruction: "do"
"#;
        let config = parse_workflow(yaml).unwrap();
        let workflow = Workflow::try_from(config).unwrap();
        assert_eq!(
            workflow.steps[0].context_policy,
            ContextPolicy::KeepLastN { n: 3 }
        );
        assert_eq!(workflow.steps[1].context_policy, ContextPolicy::KeepAll);
    }

    #[test]
    fn step_without_transitions() {
        let yaml = r#"
name: test
steps:
  - name: single
    instruction: "just do it"
"#;
        let config = parse_workflow(yaml).unwrap();
        let workflow = Workflow::try_from(config).unwrap();
        assert!(workflow.steps[0].transitions.is_empty());
    }
}
