//! Unified error type for hamoru.
//!
//! All layers return `Result<T, HamoruError>`. Error variants are organized
//! by the layer that produces them. See design-plan.md Section 9.1.1.

use crate::provider::types::TokenUsage;

/// The result of executing a single workflow step.
///
/// Defined here (rather than in `orchestrator/`) because `HamoruError::MidWorkflowFailure`
/// references it, and placing it in `orchestrator/` would create a circular dependency.
// TODO: Consider moving to a shared `types` module if more cross-cutting types emerge.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// Name of the step that was executed.
    pub step_name: String,
    /// Text output produced by the step.
    pub output: String,
    /// Token usage for this step.
    pub tokens: TokenUsage,
    /// Cost in USD for this step.
    pub cost: f64,
    /// Latency in milliseconds.
    pub latency_ms: u64,
    /// Model that was used for this step.
    pub model_used: String,
}

/// Unified error type for all hamoru operations.
///
/// Each variant includes enough context to tell the user what happened
/// AND what to do next (see design-plan.md Section 11.3).
#[derive(Debug, thiserror::Error)]
pub enum HamoruError {
    // --- Provider errors (Phase 1) ---
    /// A provider is not reachable or not configured.
    #[error("Provider '{provider}' is unavailable: {reason}")]
    ProviderUnavailable {
        /// Provider identifier.
        provider: String,
        /// Human-readable reason.
        reason: String,
    },

    /// Requested model does not exist in the provider's catalog.
    #[error("Model '{model}' not found in provider '{provider}'")]
    ModelNotFound {
        /// Provider identifier.
        provider: String,
        /// Model identifier.
        model: String,
    },

    /// Provider API call failed after retries.
    // SECURITY: sanitize source error to strip credentials before wrapping.
    #[error("Provider request failed after {attempts} retries: {source}")]
    ProviderRequestFailed {
        /// Number of attempts made.
        attempts: u32,
        /// Underlying error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    // --- Telemetry errors (Phase 2) ---
    /// Telemetry storage operation failed.
    #[error("Telemetry store error: {reason}")]
    TelemetryError {
        /// Human-readable reason.
        reason: String,
    },

    /// Telemetry sync (push/pull) failed.
    #[error("Telemetry sync failed: {source}")]
    TelemetrySyncFailed {
        /// Underlying error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    // --- Policy errors (Phase 3) ---
    /// No model in any provider satisfies the given policy constraints.
    #[error("No model satisfies policy '{policy}': {reason}")]
    NoModelSatisfiesPolicy {
        /// Policy name.
        policy: String,
        /// Human-readable reason.
        reason: String,
    },

    /// A cost limit has been exceeded.
    #[error("Cost limit exceeded: {limit} (current: ${current:.4}, max: ${max:.4})")]
    CostLimitExceeded {
        /// Which limit was exceeded (e.g., "per_request", "per_day").
        limit: String,
        /// Current accumulated cost.
        current: f64,
        /// Maximum allowed cost.
        max: f64,
    },

    // --- Orchestration errors (Phase 4) ---
    /// Workflow reached its maximum iteration count.
    #[error("Workflow '{workflow}' reached max iterations ({max})")]
    MaxIterationsReached {
        /// Workflow name.
        workflow: String,
        /// Maximum iterations allowed.
        max: u32,
    },

    /// Workflow exceeded its cost budget.
    #[error("Workflow '{workflow}' exceeded cost limit (${spent:.4} / ${limit:.4})")]
    WorkflowCostExceeded {
        /// Workflow name.
        workflow: String,
        /// Amount spent so far.
        spent: f64,
        /// Maximum allowed cost.
        limit: f64,
    },

    /// A provider failed in the middle of a multi-step workflow.
    // SECURITY: sanitize source error to strip credentials before wrapping.
    #[error("Provider failed mid-workflow at step '{step}'")]
    MidWorkflowFailure {
        /// Step where the failure occurred.
        step: String,
        /// Results from steps that completed before the failure.
        partial_results: Vec<StepResult>,
        /// Underlying error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    // --- Agent Collaboration errors (Phase 6) ---
    /// A harness constraint was violated during agent collaboration.
    #[error("Collaboration '{name}' harness constraint violated: {constraint}")]
    HarnessViolation {
        /// Collaboration name.
        name: String,
        /// Which constraint was violated.
        constraint: String,
    },

    /// Quality gate was not passed after maximum iterations.
    #[error("Quality gate not passed after {iterations} iterations in '{name}'")]
    QualityGateNotPassed {
        /// Collaboration name.
        name: String,
        /// Number of iterations attempted.
        iterations: u32,
    },

    // --- Config errors ---
    /// Configuration file is invalid or malformed.
    #[error("Invalid configuration: {reason}")]
    ConfigError {
        /// Human-readable reason.
        reason: String,
    },

    /// Required credential (API key) is not set.
    #[error("Credential not found for provider '{provider}'")]
    CredentialNotFound {
        /// Provider that needs the credential.
        provider: String,
    },
}
