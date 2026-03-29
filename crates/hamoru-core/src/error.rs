//! Unified error type for hamoru.
//!
//! All layers return `Result<T, HamoruError>`. Error variants are organized
//! by the layer that produces them. See design-plan.md Section 9.1.1.

use std::fmt;

use serde::Serialize;

use crate::provider::types::TokenUsage;

/// The result of executing a single workflow step.
///
/// Defined here (rather than in `orchestrator/`) because `HamoruError::MidWorkflowFailure`
/// references it, and placing it in `orchestrator/` would create a circular dependency.
// TODO: Consider moving to a shared `types` module if more cross-cutting types emerge.
#[derive(Clone, Serialize)]
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
    /// Policy that selected this model.
    pub policy_applied: String,
}

// Custom Debug omits `output` to prevent LLM response content leaking via Debug/Display.
impl fmt::Debug for StepResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StepResult")
            .field("step_name", &self.step_name)
            .field("output", &"<redacted>")
            .field("tokens", &self.tokens)
            .field("cost", &self.cost)
            .field("latency_ms", &self.latency_ms)
            .field("model_used", &self.model_used)
            .field("policy_applied", &self.policy_applied)
            .finish()
    }
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
    /// Workflow YAML file is invalid or cannot be parsed.
    #[error("Invalid workflow '{workflow}': {reason}")]
    WorkflowValidationError {
        /// Workflow name or path.
        workflow: String,
        /// Human-readable reason.
        reason: String,
    },

    /// A workflow step's condition evaluation failed.
    #[error("Condition evaluation failed at step '{step}': {reason}")]
    ConditionEvaluationFailed {
        /// Step where evaluation failed.
        step: String,
        /// Human-readable reason with guidance.
        reason: String,
    },

    /// Workflow reached its maximum iteration count.
    ///
    /// Note: In the default orchestrator implementation, max iterations triggers a
    /// `tracing::warn!()` and returns `Ok(ExecutionResult)` with
    /// `TerminationReason::MaxIterationsReached` (per design-plan.md §11.3).
    /// This error variant is retained for potential future strict-mode use.
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
    #[error("Provider failed mid-workflow at step '{step}': {source}")]
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

    // --- API Server errors (Phase 5b) ---
    /// API request rejected: missing or invalid API key.
    ///
    /// Distinct from `CredentialNotFound` (missing provider env var at startup).
    /// This variant is for runtime per-request authentication failures.
    #[error(
        "Authentication failed: {reason}. Provide a valid API key via Authorization: Bearer header."
    )]
    Unauthorized {
        /// Human-readable reason (e.g., "invalid API key", "missing Authorization header").
        reason: String,
    },

    /// API request rate limited: token bucket exhausted.
    ///
    /// Distinct from `CostLimitExceeded` (budget-based limit).
    /// This variant is for request-rate throttling.
    #[error("Rate limit exceeded. Retry after {retry_after_secs} seconds.")]
    RateLimitExceeded {
        /// Seconds until the next token is available.
        retry_after_secs: u64,
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

impl HamoruError {
    /// Whether this error is transient and worth retrying.
    ///
    /// `ProviderUnavailable` is retryable (covers 429, 500, 502, 503).
    /// All other errors are terminal — retrying would not help.
    pub fn is_retryable(&self) -> bool {
        matches!(self, HamoruError::ProviderUnavailable { .. })
    }
}

/// An error whose message has been sanitized to remove credentials.
#[derive(Debug)]
struct SanitizedError(String);

impl fmt::Display for SanitizedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SanitizedError {}

/// Strips credentials and sensitive content from an error message.
///
/// Wraps the original error, replacing its display text with a sanitized version.
/// Used before wrapping external errors into `ProviderRequestFailed` or
/// `MidWorkflowFailure` to prevent credential leakage via error Display.
pub fn sanitize_error(
    error: impl std::error::Error + Send + Sync + 'static,
) -> Box<dyn std::error::Error + Send + Sync> {
    let msg = error.to_string();
    let sanitized = sanitize_message(&msg);
    if sanitized == msg {
        Box::new(error)
    } else {
        Box::new(SanitizedError(sanitized))
    }
}

/// Replaces known credential patterns in a message with `[REDACTED]`.
///
/// Uses simple prefix/substring matching — no regex dependency needed.
/// Pattern ordering matters: more-specific prefixes (e.g., `sk-ant-`) must precede
/// broader ones (e.g., `sk-`) to avoid partial matches.
fn sanitize_message(msg: &str) -> String {
    let mut result = msg.to_string();

    // Token-prefix patterns (alphanum + dash/underscore/dot body)
    let token_char = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.';
    // Anthropic API keys: sk-ant-... (most specific prefix first)
    result = redact_pattern(&result, "sk-ant-", &token_char, 10);
    // Generic sk- keys (OpenAI-style): sk- followed by alphanum
    result = redact_pattern(&result, "sk-", &token_char, 10);

    // Auth scheme patterns (non-whitespace body)
    let non_ws = |c: char| !c.is_ascii_whitespace();
    // Bearer tokens
    result = redact_pattern(&result, "Bearer ", &non_ws, 10);

    // URL-embedded credentials: ://user:pass@
    result = redact_url_credentials(&result);
    // api_key=... query parameters
    result = redact_query_param(&result, "api_key=");

    result
}

/// Redacts tokens matching `prefix` followed by characters satisfying `is_token_char`.
///
/// `min_len`: minimum token-body length (after prefix) to trigger redaction. Avoids
/// false positives on short coincidental prefix matches (e.g., "sk-widget").
/// Tokens whose body starts with `[REDACTED]` are skipped to prevent double-redaction
/// across multiple sanitization passes.
fn redact_pattern(
    msg: &str,
    prefix: &str,
    is_token_char: &impl Fn(char) -> bool,
    min_len: usize,
) -> String {
    let mut result = String::with_capacity(msg.len());
    let mut remaining = msg;

    while let Some(pos) = remaining.find(prefix) {
        result.push_str(&remaining[..pos]);
        let after_prefix = &remaining[pos + prefix.len()..];
        let token_end = after_prefix
            .find(|c: char| !is_token_char(c))
            .unwrap_or(after_prefix.len());
        let token_body = &after_prefix[..token_end];
        if token_end >= min_len && !token_body.starts_with("[REDACTED]") {
            result.push_str("[REDACTED]");
        } else {
            // Too short or already redacted — preserve original text
            result.push_str(&remaining[pos..pos + prefix.len() + token_end]);
        }
        remaining = &after_prefix[token_end..];
    }
    result.push_str(remaining);
    result
}

/// Redacts URL-embedded credentials: `://user:pass@` → `://[REDACTED]@`.
fn redact_url_credentials(msg: &str) -> String {
    let mut result = String::with_capacity(msg.len());
    let mut remaining = msg;

    while let Some(pos) = remaining.find("://") {
        let scheme_end = pos + 3; // skip "://"
        result.push_str(&remaining[..scheme_end]);
        let after = &remaining[scheme_end..];
        // Look for @ within a reasonable distance (before next / or space)
        let segment_end = after
            .find(|c: char| c == '/' || c.is_ascii_whitespace())
            .unwrap_or(after.len());
        let segment = &after[..segment_end];
        if let Some(at_pos) = segment.find('@') {
            // Has credentials if there's a : before @
            if segment[..at_pos].contains(':') {
                result.push_str("[REDACTED]@");
                result.push_str(&segment[at_pos + 1..]);
            } else {
                result.push_str(segment);
            }
        } else {
            result.push_str(segment);
        }
        remaining = &after[segment_end..];
    }
    result.push_str(remaining);
    result
}

/// Redacts a query parameter value: `api_key=secret` → `[REDACTED]`.
fn redact_query_param(msg: &str, param: &str) -> String {
    let mut result = String::with_capacity(msg.len());
    let mut remaining = msg;

    while let Some(pos) = remaining.find(param) {
        result.push_str(&remaining[..pos]);
        let after = &remaining[pos + param.len()..];
        let val_end = after
            .find(|c: char| c == '&' || c.is_ascii_whitespace())
            .unwrap_or(after.len());
        result.push_str("[REDACTED]");
        remaining = &after[val_end..];
    }
    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_validation_error_includes_workflow_and_reason() {
        let e = HamoruError::WorkflowValidationError {
            workflow: "generate-and-review".to_string(),
            reason: "Step 'review' references unknown target 'nonexistent'".to_string(),
        };
        let msg = e.to_string();
        assert!(msg.contains("generate-and-review"));
        assert!(msg.contains("nonexistent"));
    }

    #[test]
    fn condition_evaluation_failed_includes_step_and_guidance() {
        let e = HamoruError::ConditionEvaluationFailed {
            step: "review".to_string(),
            reason:
                "No status found. The model did not call report_status or include a STATUS line."
                    .to_string(),
        };
        let msg = e.to_string();
        assert!(msg.contains("review"));
        assert!(msg.contains("report_status"));
    }

    #[test]
    fn workflow_cost_exceeded_includes_amounts() {
        let e = HamoruError::WorkflowCostExceeded {
            workflow: "gen-review".to_string(),
            spent: 0.52,
            limit: 0.50,
        };
        let msg = e.to_string();
        assert!(msg.contains("0.52"));
        assert!(msg.contains("0.50"));
    }

    #[test]
    fn step_result_debug_omits_output() {
        let step = StepResult {
            step_name: "generate".to_string(),
            output: "SECRET LLM OUTPUT".to_string(),
            tokens: TokenUsage::default(),
            cost: 0.01,
            latency_ms: 100,
            model_used: "test-model".to_string(),
            policy_applied: "cost-optimized".to_string(),
        };
        let debug = format!("{:?}", step);
        assert!(!debug.contains("SECRET LLM OUTPUT"));
        assert!(debug.contains("redacted"));
        assert!(debug.contains("generate"));
        assert!(debug.contains("cost-optimized"));
    }

    #[test]
    fn sanitize_strips_anthropic_api_key() {
        let msg = "Failed: sk-ant-api03-abcdefghijklmnop";
        assert_eq!(sanitize_message(msg), "Failed: [REDACTED]");
    }

    #[test]
    fn sanitize_strips_generic_secret_key() {
        let msg = "Auth error with key sk-proj1234567890abcdefghij";
        assert_eq!(sanitize_message(msg), "Auth error with key [REDACTED]");
    }

    #[test]
    fn sanitize_strips_bearer_token() {
        let msg = "header: Bearer eyJhbGciOiJSUzI1NiJ9.payload";
        assert_eq!(sanitize_message(msg), "header: [REDACTED]");
    }

    #[test]
    fn sanitize_strips_url_credentials() {
        let msg = "connecting to https://admin:s3cret@api.example.com/v1";
        assert_eq!(
            sanitize_message(msg),
            "connecting to https://[REDACTED]@api.example.com/v1"
        );
    }

    #[test]
    fn sanitize_strips_api_key_param() {
        let msg = "GET /v1/models?api_key=sk12345&format=json";
        assert_eq!(
            sanitize_message(msg),
            "GET /v1/models?[REDACTED]&format=json"
        );
    }

    #[test]
    fn sanitize_preserves_clean_message() {
        let msg = "Connection refused: localhost:11434";
        assert_eq!(sanitize_message(msg), msg);
    }

    #[test]
    fn sanitize_error_wraps_when_dirty() {
        let original = std::io::Error::other("failed with key sk-ant-api03-abcdefghijklmnop");
        let sanitized = sanitize_error(original);
        assert!(sanitized.to_string().contains("[REDACTED]"));
        assert!(!sanitized.to_string().contains("sk-ant-api03"));
    }

    #[test]
    fn sanitize_error_preserves_when_clean() {
        let original = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let sanitized = sanitize_error(original);
        assert_eq!(sanitized.to_string(), "refused");
    }
}
