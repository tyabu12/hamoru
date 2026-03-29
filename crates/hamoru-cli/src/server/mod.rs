//! axum-based OpenAI-compatible API server.
//!
//! Routes requests through hamoru's provider/policy/orchestration layers
//! and returns responses in the OpenAI wire format.


use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use hamoru_core::error::HamoruError;
use hamoru_core::policy::DefaultPolicyEngine;
use hamoru_core::provider::ProviderRegistry;
use hamoru_core::server::types::{OaiErrorBody, OaiErrorResponse};
use hamoru_core::telemetry::TelemetryStore;
use serde_json::json;

/// Shared application state for all handlers.
pub struct AppState {
    /// Provider registry for LLM API calls.
    pub providers: ProviderRegistry,
    /// Policy engine for model selection.
    pub policy_engine: DefaultPolicyEngine,
    /// Telemetry store for recording API calls.
    pub telemetry: Box<dyn TelemetryStore>,
}

/// Build the axum Router with all API routes.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/models", get(list_models))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// GET /v1/models
// ---------------------------------------------------------------------------

/// Lists available models in the OpenAI format.
///
/// Includes direct provider models and policy-based virtual models.
async fn list_models(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut models = Vec::new();

    // Direct provider models: <provider>:<model>
    for provider in state.providers.iter() {
        let provider_models = provider.list_models().await.map_err(ApiError)?;
        for model_info in &provider_models {
            let model_id = format!("{}:{}", model_info.provider, model_info.id);
            models.push(json!({
                "id": model_id,
                "object": "model",
                "created": 0,
                "owned_by": format!("hamoru:{}", model_info.provider),
            }));
        }
    }

    // Policy-based virtual models: hamoru:<policy>
    for policy in state.policy_engine.list_policies() {
        let model_id = format!("hamoru:{}", policy);
        models.push(json!({
            "id": model_id,
            "object": "model",
            "created": 0,
            "owned_by": "hamoru",
        }));
    }

    Ok(Json(json!({
        "object": "list",
        "data": models,
    })))
}

// ---------------------------------------------------------------------------
// Error handling: HamoruError → OpenAI JSON error + HTTP status
// ---------------------------------------------------------------------------

/// Wrapper that converts `HamoruError` into OpenAI-compatible JSON error responses.
pub struct ApiError(pub HamoruError);

impl From<HamoruError> for ApiError {
    fn from(err: HamoruError) -> Self {
        ApiError(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_type, code) = classify_error(&self.0);
        let body = OaiErrorResponse {
            error: OaiErrorBody {
                message: self.0.to_string(),
                error_type: error_type.to_string(),
                code: code.map(|c| c.to_string()),
            },
        };
        (status, Json(body)).into_response()
    }
}

/// Map `HamoruError` variants to (HTTP status, error type, optional code).
fn classify_error(err: &HamoruError) -> (StatusCode, &'static str, Option<&'static str>) {
    match err {
        HamoruError::ModelNotFound { .. } => {
            (StatusCode::NOT_FOUND, "not_found_error", Some("model_not_found"))
        }
        HamoruError::CredentialNotFound { .. } => {
            (StatusCode::UNAUTHORIZED, "authentication_error", Some("missing_credentials"))
        }
        HamoruError::CostLimitExceeded { .. } => {
            (StatusCode::TOO_MANY_REQUESTS, "rate_limit_error", Some("cost_limit_exceeded"))
        }
        HamoruError::NoModelSatisfiesPolicy { .. } => {
            (StatusCode::BAD_REQUEST, "invalid_request_error", Some("no_model_available"))
        }
        HamoruError::ProviderUnavailable { .. } => {
            (StatusCode::SERVICE_UNAVAILABLE, "server_error", Some("provider_unavailable"))
        }
        HamoruError::ProviderRequestFailed { .. } => {
            (StatusCode::BAD_GATEWAY, "server_error", Some("provider_request_failed"))
        }
        HamoruError::ConfigError { .. } => {
            (StatusCode::BAD_REQUEST, "invalid_request_error", None)
        }
        _ => {
            (StatusCode::INTERNAL_SERVER_ERROR, "server_error", None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use hamoru_core::policy::config::{
        PolicyConfig, PolicyDefinition, PolicyPreferences, Priority,
    };
    use hamoru_core::provider::mock::MockProvider;
    use hamoru_core::provider::types::ModelInfo;
    use hamoru_core::telemetry::memory::InMemoryTelemetryStore;
    use tower::ServiceExt;

    fn test_state(models: Vec<ModelInfo>) -> Arc<AppState> {
        let mut provider = MockProvider::new("test-provider");
        provider.set_models(models);
        let mut registry = ProviderRegistry::new();
        registry.register(Box::new(provider));

        let policy_config = PolicyConfig {
            policies: vec![PolicyDefinition {
                name: "cost-optimized".to_string(),
                description: None,
                constraints: Default::default(),
                preferences: PolicyPreferences {
                    priority: Priority::Cost,
                },
            }],
            ..Default::default()
        };
        let policy_engine = DefaultPolicyEngine::new(policy_config);
        let telemetry = Box::new(InMemoryTelemetryStore::new());

        Arc::new(AppState {
            providers: registry,
            policy_engine,
            telemetry,
        })
    }

    fn sample_model() -> ModelInfo {
        ModelInfo {
            id: "test-model".to_string(),
            provider: "test-provider".to_string(),
            context_window: 100_000,
            cost_per_input_token: 3.0 / 1_000_000.0,
            cost_per_output_token: 15.0 / 1_000_000.0,
            cost_per_cached_input_token: None,
            capabilities: vec![],
            max_output_tokens: Some(4096),
        }
    }

    #[tokio::test]
    async fn list_models_returns_provider_and_policy_models() {
        let state = test_state(vec![sample_model()]);
        let app = build_router(state);

        let req = Request::builder()
            .uri("/v1/models")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["object"], "list");
        let data = json["data"].as_array().unwrap();

        // Should have at least 1 provider model + 1 policy model
        assert!(data.len() >= 2);

        // Check provider model ID format
        let provider_model = data.iter().find(|m| m["id"] == "test-provider:test-model");
        assert!(provider_model.is_some(), "Provider model not found");

        // Check policy model ID format
        let policy_model = data.iter().find(|m| m["id"] == "hamoru:cost-optimized");
        assert!(policy_model.is_some(), "Policy model not found");
    }

    #[tokio::test]
    async fn list_models_empty_providers() {
        let state = test_state(vec![]);
        let app = build_router(state);

        let req = Request::builder()
            .uri("/v1/models")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // Should still have policy model
        let data = json["data"].as_array().unwrap();
        assert!(data.iter().any(|m| m["id"] == "hamoru:cost-optimized"));
    }

    #[tokio::test]
    async fn error_model_not_found_returns_404() {
        let err = ApiError(HamoruError::ModelNotFound {
            provider: "test".to_string(),
            model: "nonexistent".to_string(),
        });
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["type"], "not_found_error");
        assert_eq!(json["error"]["code"], "model_not_found");
    }

    #[tokio::test]
    async fn error_credential_not_found_returns_401() {
        let err = ApiError(HamoruError::CredentialNotFound {
            provider: "anthropic".to_string(),
        });
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn error_cost_limit_returns_429() {
        let err = ApiError(HamoruError::CostLimitExceeded {
            limit: "daily".to_string(),
            current: 10.0,
            max: 5.0,
        });
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn error_provider_unavailable_returns_503() {
        let err = ApiError(HamoruError::ProviderUnavailable {
            provider: "anthropic".to_string(),
            reason: "timeout".to_string(),
        });
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn unknown_route_returns_404() {
        let state = test_state(vec![]);
        let app = build_router(state);

        let req = Request::builder()
            .uri("/v1/nonexistent")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
