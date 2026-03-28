//! Layer 5: Agent Collaboration Engine.
//!
//! Declarative agent coordination — the core differentiator of hamoru.
//! Compiles collaboration patterns (Generator/Evaluator, Pipeline, Debate, etc.)
//! into Layer 4 `Workflow` types and delegates execution. Layer 5 must NOT
//! have its own execution loop.
//!
//! **These trait definitions are provisional.** They will be redesigned at Phase 6
//! start based on Layer 4 implementation experience. See design-plan.md Section 9.1.3.

// TODO: Redesign at Phase 6 start (Section 9.1.3).
// Specifically re-evaluate:
// - Delegation method to OrchestrationEngine (trait parameter vs internal field)
// - Internal representation of collaboration patterns
// - Relationship between CollaborationResult and ExecutionResult

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;

use crate::Result;
use crate::orchestrator::{OrchestrationEngine, Workflow};
use crate::policy::PolicyEngine;
use crate::provider::{ProviderRegistry, TokenUsage};
use crate::telemetry::TelemetryStore;

/// Engine for declarative agent collaboration.
///
/// Compiles collaboration patterns into Layer 4 workflows and delegates
/// execution to the `OrchestrationEngine`.
///
/// **Provisional** — will be redesigned at Phase 6 start.
#[async_trait]
pub trait AgentCollaborationEngine: Send + Sync {
    /// Loads agent definitions from a YAML configuration file.
    fn load_agents(&self, path: &Path) -> Result<AgentConfig>;

    /// Compiles a collaboration pattern into a Layer 4 Workflow.
    ///
    /// This conversion logic is the core of Layer 5. Execution is fully
    /// delegated to Layer 4.
    fn compile(&self, collaboration: &Collaboration, task: &str) -> Result<Workflow>;

    /// Helper that runs `compile()` → `OrchestrationEngine::execute()` in sequence.
    async fn execute_collaboration(
        &self,
        collaboration: &Collaboration,
        task: &str,
        policy_engine: &dyn PolicyEngine,
        orchestration_engine: &dyn OrchestrationEngine,
        providers: &ProviderRegistry,
        telemetry: &dyn TelemetryStore,
    ) -> Result<CollaborationResult>;
}

/// Agent configuration (parsed from YAML).
// TODO: Finalize fields in Phase 6.
#[derive(Debug, Clone, Default)]
pub struct AgentConfig;

/// A single agent definition — the Rust representation of a YAML agent entry.
// TODO: Finalize fields in Phase 6.
#[derive(Debug, Clone, Default)]
pub struct AgentDefinition;

/// A collaboration definition combining agents with a pattern and constraints.
// TODO: Finalize fields in Phase 6.
#[derive(Debug, Clone, Default)]
pub struct Collaboration;

/// Built-in collaboration patterns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollaborationPattern {
    /// Generator produces output, Evaluator reviews in a loop.
    GeneratorEvaluator,
    /// Sequential pipeline of agents.
    Pipeline,
    /// Pipeline with parallel review stage.
    PipelineWithParallelReview,
    /// Multiple agents debate to reach agreement.
    Debate,
    /// Independent generation followed by majority vote or best selection.
    Consensus,
}

/// Constraints applied to an agent collaboration session.
#[derive(Debug, Clone)]
pub struct HarnessConstraints {
    /// Maximum cost in USD for the entire collaboration.
    pub cost_limit: Option<f64>,
    /// Maximum wall-clock time for the entire collaboration.
    pub timeout: Option<Duration>,
    /// Maximum iterations for Generator/Evaluator loops.
    pub max_iterations: Option<u32>,
    /// Quality gate that must be satisfied.
    pub quality_gate: Option<QualityGate>,
    /// Strategy for managing context growth across iterations.
    pub context_management: Option<ContextManagement>,
}

/// Strategy for managing context window growth in iterative collaborations.
///
/// Related to but distinct from Layer 4's `ContextPolicy`:
/// - `KeepLastN` maps to `ContextPolicy::KeepLastN` at compile time.
/// - `SummarizeOnOverflow` inserts summarization steps into the DAG at compile time.
#[derive(Debug, Clone, PartialEq)]
pub enum ContextManagement {
    /// Keep only the last N iteration outputs.
    KeepLastN {
        /// Number of recent iterations to retain.
        n: u32,
    },
    /// Insert a summarization step when token count exceeds threshold.
    SummarizeOnOverflow {
        /// Token count threshold that triggers summarization.
        max_context_tokens: u64,
        /// Tags for Policy Engine to select the summarization model.
        summary_tags: Vec<String>,
    },
}

/// Quality criteria that must be met before a collaboration completes.
#[derive(Debug, Clone, PartialEq)]
pub enum QualityGate {
    /// The evaluator agent must return "approved".
    EvaluatorMustApprove,
    /// All parallel reviewers must approve.
    AllMustApprove,
    /// Majority of reviewers must approve.
    Majority,
    /// Output must meet a minimum quality score.
    ScoreThreshold {
        /// Minimum acceptable score.
        min_score: f64,
    },
}

/// Result of executing a complete agent collaboration.
#[derive(Debug, Clone)]
pub struct CollaborationResult {
    /// Execution record for each agent involved.
    pub agents_used: Vec<AgentExecution>,
    /// Number of Generator/Evaluator loop iterations.
    pub iterations: u32,
    /// Total cost in USD across all agents.
    pub total_cost: f64,
    /// Total token usage across all agents.
    pub total_tokens: TokenUsage,
    /// Total wall-clock latency in milliseconds.
    pub total_latency_ms: u64,
    /// Final output text.
    pub final_output: String,
    /// Whether the quality gate was satisfied.
    pub quality_gate_passed: bool,
    /// Harness constraint fulfillment report.
    pub harness_report: HarnessReport,
}

/// Execution record for a single agent within a collaboration.
#[derive(Debug, Clone)]
pub struct AgentExecution {
    /// Name of the agent.
    pub agent_name: String,
    /// Model selected by the Policy Engine.
    pub model_used: String,
    /// Name of the policy that was applied.
    pub policy_applied: String,
    /// Cost in USD for this agent's execution.
    pub cost: f64,
    /// Token usage for this agent's execution.
    pub tokens: TokenUsage,
    /// Latency in milliseconds.
    pub latency_ms: u64,
}

/// Report on harness constraint usage during a collaboration.
#[derive(Debug, Clone)]
pub struct HarnessReport {
    /// Total cost consumed.
    pub cost_used: f64,
    /// Cost limit that was configured, if any.
    pub cost_limit: Option<f64>,
    /// Total elapsed time.
    pub time_elapsed: Duration,
    /// Timeout that was configured, if any.
    pub timeout: Option<Duration>,
    /// Number of iterations consumed.
    pub iterations_used: u32,
    /// Maximum iterations that were configured, if any.
    pub max_iterations: Option<u32>,
}
