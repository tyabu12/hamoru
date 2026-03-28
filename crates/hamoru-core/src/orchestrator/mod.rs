//! Layer 4: Orchestration Engine.
//!
//! Executes multi-step workflows as a DAG. Each step invokes an LLM via
//! the Policy Engine's model selection, with transitions driven by
//! condition evaluation (tool calling or status line parsing).

use std::path::Path;

use async_trait::async_trait;

use crate::Result;
use crate::error::StepResult;
use crate::policy::PolicyEngine;
use crate::provider::{ProviderRegistry, TokenUsage};
use crate::telemetry::TelemetryStore;

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

/// A complete workflow definition (parsed from YAML).
// TODO: Finalize fields in Phase 4a.
#[derive(Debug, Clone, Default)]
pub struct Workflow;

/// A single step within a workflow.
// TODO: Finalize fields in Phase 4a.
#[derive(Debug, Clone, Default)]
pub struct WorkflowStep;

/// A transition between workflow steps, triggered by condition evaluation.
// TODO: Finalize fields in Phase 4a.
#[derive(Debug, Clone, Default)]
pub struct Transition;

/// Declarative control over message history before step execution.
///
/// Applied by Layer 4 before each step to manage context window usage.
/// Note: `SummarizeOnOverflow` is NOT here — it requires LLM calls and is
/// handled by Layer 5 inserting summary steps into the DAG at compile time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConditionMode {
    /// Use tool calling with `report_status` tool (default, more robust).
    #[default]
    ToolCalling,
    /// Parse STATUS lines from the LLM's text output (fallback for models
    /// without tool support).
    StatusLine,
}

/// Parsed output from STATUS line condition evaluation (v1 fallback).
///
/// Used when `ConditionMode::StatusLine` is active. The parser scans the
/// last N lines in reverse, matching the first STATUS line found.
#[derive(Debug, Clone)]
pub struct StepOutput {
    /// Complete LLM output including the STATUS line.
    pub full_content: String,
    /// Extracted status value (e.g., "approved", "improve", "done").
    pub status: String,
    /// Body content excluding the STATUS line.
    pub content: String,
}

/// The result of executing a complete workflow.
#[derive(Debug, Clone)]
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
}
