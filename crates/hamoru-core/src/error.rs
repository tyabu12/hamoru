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
// TODO(Phase 4): Replace derived `Debug` with a custom impl that omits `output` field
//   to prevent LLM response content leaking via error Display/Debug. See design-plan.md §5.4.
#[derive(Debug, Clone, Serialize)]
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
fn sanitize_message(msg: &str) -> String {
    let mut result = msg.to_string();

    // Anthropic API keys: sk-ant-...
    result = redact_token_pattern(&result, "sk-ant-");
    // Generic sk- keys (OpenAI-style): sk- followed by 20+ alphanum
    result = redact_token_pattern(&result, "sk-");
    // Bearer tokens
    result = redact_bearer(&result);
    // URL-embedded credentials: ://user:pass@
    result = redact_url_credentials(&result);
    // api_key=... query parameters
    result = redact_query_param(&result, "api_key=");

    result
}

/// Redacts tokens starting with a given prefix (e.g., "sk-ant-", "sk-").
fn redact_token_pattern(msg: &str, prefix: &str) -> String {
    let mut result = String::with_capacity(msg.len());
    let mut remaining = msg;

    while let Some(pos) = remaining.find(prefix) {
        result.push_str(&remaining[..pos]);
        let after_prefix = &remaining[pos + prefix.len()..];
        // Token continues while alphanumeric, dash, or underscore
        let token_end = after_prefix
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_' && c != '.')
            .unwrap_or(after_prefix.len());
        if token_end >= 10 {
            result.push_str("[REDACTED]");
        } else {
            // Too short to be a real key — preserve it
            result.push_str(&remaining[pos..pos + prefix.len() + token_end]);
        }
        remaining = &after_prefix[token_end..];
    }
    result.push_str(remaining);
    result
}

/// Redacts "Bearer <token>" patterns.
fn redact_bearer(msg: &str) -> String {
    let mut result = String::with_capacity(msg.len());
    let mut remaining = msg;

    while let Some(pos) = remaining.find("Bearer ") {
        result.push_str(&remaining[..pos]);
        let after = &remaining[pos + 7..]; // skip "Bearer "
        let token_end = after
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(after.len());
        if token_end >= 10 {
            result.push_str("[REDACTED]");
        } else {
            result.push_str(&remaining[pos..pos + 7 + token_end]);
        }
        remaining = &after[token_end..];
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
