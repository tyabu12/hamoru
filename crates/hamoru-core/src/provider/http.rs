//! Shared HTTP client utilities for provider adapters.
//!
//! Common patterns for building HTTP clients, mapping errors, and determining
//! retryability. Used by both Anthropic and Ollama providers.

use std::time::Duration;

use reqwest::StatusCode;

use crate::error::HamoruError;

/// Default timeout for provider HTTP requests (2 minutes).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Default connect timeout (10 seconds).
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds a reqwest `Client` with standard hamoru settings.
pub fn build_client(timeout: Duration) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .build()
}

/// Maps an HTTP status code to the appropriate `HamoruError`.
///
/// For retryable status codes (429, 500, 502, 503), returns `ProviderUnavailable`
/// which is retryable. For terminal errors (401, 403, 404), returns the
/// corresponding non-retryable error variant.
pub fn map_http_error(status: StatusCode, body: &str, provider: &str, model: &str) -> HamoruError {
    match status.as_u16() {
        401 | 403 => HamoruError::CredentialNotFound {
            provider: provider.to_string(),
        },
        404 => HamoruError::ModelNotFound {
            provider: provider.to_string(),
            model: model.to_string(),
        },
        429 => HamoruError::ProviderUnavailable {
            provider: provider.to_string(),
            reason: format!("Rate limited (HTTP 429). {body}"),
        },
        500 | 502 | 503 => HamoruError::ProviderUnavailable {
            provider: provider.to_string(),
            reason: format!("Server error (HTTP {status}). {body}"),
        },
        _ => HamoruError::ProviderUnavailable {
            provider: provider.to_string(),
            reason: format!("HTTP {status}: {body}"),
        },
    }
}

/// Whether an HTTP status code represents a transient/retryable error.
pub fn is_retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_http_error_401_returns_credential_not_found() {
        let err = map_http_error(StatusCode::UNAUTHORIZED, "bad key", "claude", "test");
        match err {
            HamoruError::CredentialNotFound { provider } => {
                assert_eq!(provider, "claude");
            }
            e => panic!("expected CredentialNotFound, got {e:?}"),
        }
    }

    #[test]
    fn map_http_error_404_returns_model_not_found() {
        let err = map_http_error(StatusCode::NOT_FOUND, "not found", "claude", "bad-model");
        match err {
            HamoruError::ModelNotFound { provider, model } => {
                assert_eq!(provider, "claude");
                assert_eq!(model, "bad-model");
            }
            e => panic!("expected ModelNotFound, got {e:?}"),
        }
    }

    #[test]
    fn map_http_error_429_returns_retryable() {
        let err = map_http_error(StatusCode::TOO_MANY_REQUESTS, "slow down", "claude", "test");
        assert!(err.is_retryable());
    }

    #[test]
    fn map_http_error_500_returns_retryable() {
        let err = map_http_error(StatusCode::INTERNAL_SERVER_ERROR, "oops", "claude", "test");
        assert!(err.is_retryable());
    }

    #[test]
    fn is_retryable_status_correct() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(!is_retryable_status(StatusCode::OK));
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable_status(StatusCode::NOT_FOUND));
    }

    #[test]
    fn error_messages_include_remediation_context() {
        let err = map_http_error(
            StatusCode::TOO_MANY_REQUESTS,
            "Retry after 30s",
            "claude",
            "test",
        );
        let msg = err.to_string();
        assert!(msg.contains("Rate limited"));
        assert!(msg.contains("Retry after 30s"));
    }
}
