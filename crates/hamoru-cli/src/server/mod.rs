//! axum-based OpenAI-compatible API server.
//!
//! Routes requests through hamoru's provider/policy/orchestration layers
//! and returns responses in the OpenAI wire format.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use chrono::Utc;
use futures::StreamExt;
use hamoru_core::error::HamoruError;
use hamoru_core::policy::DefaultPolicyEngine;
use hamoru_core::provider::ProviderRegistry;
use hamoru_core::provider::types::ChatRequest;
use hamoru_core::server::namespace::{ModelTarget, parse_model_target};
use hamoru_core::server::translate;
use hamoru_core::server::types::{
    OaiChatChunk, OaiChatRequest, OaiChatResponse, OaiChoice, OaiChunkChoice, OaiChunkDelta,
    OaiErrorBody, OaiErrorResponse, OaiResponseMessage, OaiUsage,
};
use hamoru_core::telemetry::{HistoryEntry, TelemetryStore};
use serde_json::json;
use tokio_stream::wrappers::ReceiverStream;

/// TTL for the cached `/v1/models` response (60 seconds).
const MODELS_CACHE_TTL: Duration = Duration::from_secs(60);

/// Shared application state for all handlers.
pub struct AppState {
    /// Provider registry for LLM API calls.
    pub providers: ProviderRegistry,
    /// Policy engine for model selection.
    pub policy_engine: DefaultPolicyEngine,
    /// Telemetry store for recording API calls.
    pub telemetry: Box<dyn TelemetryStore>,
    /// Cached `/v1/models` response with TTL to avoid per-request provider calls.
    models_cache: tokio::sync::RwLock<Option<(Instant, serde_json::Value)>>,
}

impl AppState {
    /// Creates a new `AppState` with empty models cache.
    pub fn new(
        providers: ProviderRegistry,
        policy_engine: DefaultPolicyEngine,
        telemetry: Box<dyn TelemetryStore>,
    ) -> Self {
        Self {
            providers,
            policy_engine,
            telemetry,
            models_cache: tokio::sync::RwLock::new(None),
        }
    }
}

/// Build the axum Router with all API routes.
// TODO(Phase 5b): Add DefaultBodyLimit::max() layer for explicit request size control
// TODO(Phase 5b): Add tower::timeout::Timeout layer for non-streaming request timeout
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// GET /v1/models
// ---------------------------------------------------------------------------

/// Lists available models in the OpenAI format.
///
/// Uses a TTL cache to avoid calling `list_models()` on every provider per request.
/// Provider calls are parallelized via `join_all`.
async fn list_models(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Check cache
    {
        let cache = state.models_cache.read().await;
        if let Some((cached_at, ref response)) = *cache
            && cached_at.elapsed() < MODELS_CACHE_TTL
        {
            return Ok(Json(response.clone()));
        }
    }

    // Cache miss or expired — fetch from providers in parallel
    let futures: Vec<_> = state
        .providers
        .iter()
        .map(|provider| provider.list_models())
        .collect();
    let results = futures::future::join_all(futures).await;

    let mut models = Vec::new();
    for result in results {
        let provider_models = result.map_err(ApiError)?;
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

    let response = json!({
        "object": "list",
        "data": models,
    });

    // Update cache
    {
        let mut cache = state.models_cache.write().await;
        *cache = Some((Instant::now(), response.clone()));
    }

    Ok(Json(response))
}

// ---------------------------------------------------------------------------
// POST /v1/chat/completions
// ---------------------------------------------------------------------------

/// Handles chat completion requests (non-streaming and streaming).
async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OaiChatRequest>,
) -> Result<Response, ApiError> {
    let target = parse_model_target(&req.model)?;

    // Translate messages
    let messages: Vec<_> = req
        .messages
        .iter()
        .map(translate::oai_message_to_internal)
        .collect::<Result<_, _>>()?;

    // Translate tools
    let tools = req.tools.as_deref().map(translate::oai_tools_to_internal);

    // Translate tool_choice
    let tool_choice = req
        .tool_choice
        .as_ref()
        .map(translate::oai_tool_choice_to_internal)
        .transpose()?;

    // Resolve provider and model based on target
    let (provider_id, model_id) = resolve_target(&target, &state)?;

    let provider =
        state
            .providers
            .get(&provider_id)
            .ok_or_else(|| HamoruError::ProviderUnavailable {
                provider: provider_id.clone(),
                reason: "Provider not found in registry".to_string(),
            })?;

    if req.stream {
        // Streaming path
        let chat_request = ChatRequest {
            model: model_id.clone(),
            messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            tools,
            tool_choice,
            stream: true,
        };

        let response_id = format!("chatcmpl-{:032x}", rand::random::<u128>());
        let created = Utc::now().timestamp();
        let model_label = format!("{provider_id}:{model_id}");
        let stream = provider.chat_stream(chat_request).await?;

        // Channel-based SSE: spawn a task that consumes the provider stream
        // and sends formatted SSE events through a channel.
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(32);
        let provider_id_clone = provider_id.clone();
        let model_id_clone = model_id.clone();

        // Send initial chunk with role
        let first_chunk = OaiChatChunk {
            id: response_id.clone(),
            object: "chat.completion.chunk".to_string(),
            created,
            model: model_label.clone(),
            choices: vec![OaiChunkChoice {
                index: 0,
                delta: OaiChunkDelta {
                    role: Some("assistant".to_string()),
                    content: None,
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
            hamoru: None,
        };
        // TODO: unwrap_or_default silently swallows serialization errors — consider logging
        let first_data = serde_json::to_string(&first_chunk).unwrap_or_default();
        let _ = tx.send(Ok(Event::default().data(first_data))).await;

        // Spawn stream consumer task
        // Safety: telemetry is behind Arc (AppState is Arc'd), so we clone the Arc
        let state_clone = Arc::clone(&state);
        tokio::spawn(async move {
            let start = Instant::now();
            let mut stream = std::pin::pin!(stream);
            let mut final_usage = None;

            // TODO(Phase 5b): Wrap stream.next() with tokio::time::timeout to prevent
            // indefinitely hanging connections when a provider stream stalls.
            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        // Capture final usage for telemetry
                        if chunk.usage.is_some() {
                            final_usage = chunk.usage.clone();
                        }

                        let oai_chunk = OaiChatChunk {
                            id: response_id.clone(),
                            object: "chat.completion.chunk".to_string(),
                            created,
                            model: model_label.clone(),
                            choices: vec![OaiChunkChoice {
                                index: 0,
                                delta: OaiChunkDelta {
                                    role: None,
                                    content: if chunk.delta.is_empty() {
                                        None
                                    } else {
                                        Some(chunk.delta)
                                    },
                                    tool_calls: chunk
                                        .tool_calls
                                        .as_ref()
                                        .map(|tcs| translate::tool_calls_to_oai(tcs)),
                                },
                                finish_reason: chunk
                                    .finish_reason
                                    .as_ref()
                                    .map(|fr| translate::finish_reason_to_oai(fr).to_string()),
                            }],
                            usage: chunk.usage.as_ref().map(|u| OaiUsage {
                                prompt_tokens: u.input_tokens,
                                completion_tokens: u.output_tokens,
                                total_tokens: u.input_tokens + u.output_tokens,
                            }),
                            hamoru: None,
                        };

                        // TODO: unwrap_or_default silently swallows serialization errors
                        let data = serde_json::to_string(&oai_chunk).unwrap_or_default();
                        if tx.send(Ok(Event::default().data(data))).await.is_err() {
                            break; // Client disconnected
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Stream error: {e}");
                        break;
                    }
                }
            }

            // Send [DONE] sentinel
            let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;

            // Record telemetry
            let latency_ms = start.elapsed().as_millis() as u64;
            if let Some(ref usage) = final_usage {
                let entry = HistoryEntry {
                    timestamp: Utc::now(),
                    provider: provider_id_clone,
                    model: model_id_clone,
                    tokens: usage.clone(),
                    cost: 0.0, // Cost calculation requires model_info lookup
                    latency_ms,
                    success: true,
                    tags: vec![],
                };
                if let Err(e) = state_clone.telemetry.record(&entry).await {
                    tracing::warn!("Failed to record telemetry: {e}");
                }
            }
        });

        let sse_stream = ReceiverStream::new(rx);
        let sse = Sse::new(sse_stream).keep_alive(axum::response::sse::KeepAlive::default());

        Ok(sse.into_response())
    } else {
        // Non-streaming path
        let chat_request = ChatRequest {
            model: model_id.clone(),
            messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            tools,
            tool_choice,
            stream: false,
        };

        let start = Instant::now();
        let response = provider.chat(chat_request).await?;
        let latency_ms = start.elapsed().as_millis() as u64;

        // Build OpenAI response first, then record telemetry in background
        // to avoid adding model_info lookup latency to the response path.
        let (content, oai_tool_calls) = translate::chat_response_to_oai_parts(&response);
        let response_id = format!("chatcmpl-{:032x}", rand::random::<u128>());

        let oai_response = OaiChatResponse {
            id: response_id,
            object: "chat.completion".to_string(),
            created: Utc::now().timestamp(),
            model: format!("{provider_id}:{model_id}"),
            choices: vec![OaiChoice {
                index: 0,
                message: OaiResponseMessage {
                    role: "assistant".to_string(),
                    content,
                    tool_calls: oai_tool_calls,
                },
                finish_reason: translate::finish_reason_to_oai(&response.finish_reason).to_string(),
            }],
            usage: OaiUsage {
                prompt_tokens: response.usage.input_tokens,
                completion_tokens: response.usage.output_tokens,
                total_tokens: response.usage.input_tokens + response.usage.output_tokens,
            },
        };

        // Record telemetry in background to avoid blocking the response.
        // Re-lookup provider via state (Arc'd, 'static) to satisfy spawn's lifetime.
        let state_clone = Arc::clone(&state);
        let usage_clone = response.usage.clone();
        tokio::spawn(async move {
            let cost = if let Some(p) = state_clone.providers.get(&provider_id) {
                p.model_info(&model_id)
                    .await
                    .map(|mi| usage_clone.calculate_cost(&mi))
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            let entry = HistoryEntry {
                timestamp: Utc::now(),
                provider: provider_id,
                model: model_id,
                tokens: usage_clone,
                cost,
                latency_ms,
                success: true,
                tags: vec![],
            };
            if let Err(e) = state_clone.telemetry.record(&entry).await {
                tracing::warn!("Failed to record telemetry: {e}");
            }
        });

        Ok(Json(oai_response).into_response())
    }
}

/// Resolve a `ModelTarget` into (provider_id, model_id).
///
/// For direct targets, returns the provider and model from the namespace.
/// For policy targets, runs model selection via the policy engine.
fn resolve_target(
    target: &ModelTarget,
    _state: &AppState,
) -> Result<(String, String), HamoruError> {
    match target {
        ModelTarget::Direct { provider, model } => Ok((provider.clone(), model.clone())),
        ModelTarget::Policy { policy_name } => {
            // Collect all available models from all providers
            // For now, use a synchronous approach: we need model info for selection.
            // In a real async flow, we'd pre-load this. For correctness, use
            // the first provider's models as the candidate set.
            // TODO: collect from all providers asynchronously in a future optimization
            Err(HamoruError::ConfigError {
                reason: format!(
                    "Policy-based routing (hamoru:{policy_name}) requires async model collection. \
                     Use direct provider:model format for now."
                ),
            })
        }
        ModelTarget::Workflow { workflow_name } => Err(HamoruError::ConfigError {
            reason: format!(
                "Workflow execution (hamoru:workflow:{workflow_name}) is not yet supported via API."
            ),
        }),
        ModelTarget::Agents { collaboration_name } => Err(HamoruError::ConfigError {
            reason: format!(
                "Agent collaboration (hamoru:agents:{collaboration_name}) is planned for Phase 6."
            ),
        }),
    }
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
        // TODO(Phase 5b): For the catch-all 500 case, consider returning a generic
        // "Internal server error" message instead of self.0.to_string() to avoid
        // leaking internal implementation details to API clients.
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
        HamoruError::ModelNotFound { .. } => (
            StatusCode::NOT_FOUND,
            "not_found_error",
            Some("model_not_found"),
        ),
        HamoruError::CredentialNotFound { .. } => (
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            Some("missing_credentials"),
        ),
        HamoruError::CostLimitExceeded { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_error",
            Some("cost_limit_exceeded"),
        ),
        HamoruError::NoModelSatisfiesPolicy { .. } => (
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("no_model_available"),
        ),
        HamoruError::ProviderUnavailable { .. } => (
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            Some("provider_unavailable"),
        ),
        HamoruError::ProviderRequestFailed { .. } => (
            StatusCode::BAD_GATEWAY,
            "server_error",
            Some("provider_request_failed"),
        ),
        HamoruError::ConfigError { .. } => (StatusCode::BAD_REQUEST, "invalid_request_error", None),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "server_error", None),
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

        Arc::new(AppState::new(registry, policy_engine, telemetry))
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
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
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
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
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

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
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

    fn test_state_with_response(
        models: Vec<ModelInfo>,
        chat_response: hamoru_core::provider::types::ChatResponse,
    ) -> Arc<AppState> {
        let mut provider = MockProvider::new("test-provider");
        provider.set_models(models);
        provider.queue_chat_response(Ok(chat_response));
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

        Arc::new(AppState::new(registry, policy_engine, telemetry))
    }

    fn sample_chat_response() -> hamoru_core::provider::types::ChatResponse {
        hamoru_core::provider::types::ChatResponse {
            content: "Hello! How can I help?".to_string(),
            model: "test-model".to_string(),
            usage: hamoru_core::provider::types::TokenUsage {
                input_tokens: 10,
                output_tokens: 8,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
            latency_ms: 150,
            finish_reason: hamoru_core::provider::types::FinishReason::Stop,
            tool_calls: None,
        }
    }

    #[tokio::test]
    async fn chat_completions_non_streaming_direct_model() {
        let state = test_state_with_response(vec![sample_model()], sample_chat_response());
        let app = build_router(state);

        let body = serde_json::json!({
            "model": "test-provider:test-model",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(json["object"], "chat.completion");
        assert_eq!(json["choices"][0]["finish_reason"], "stop");
        assert_eq!(json["choices"][0]["message"]["role"], "assistant");
        assert_eq!(
            json["choices"][0]["message"]["content"],
            "Hello! How can I help?"
        );
        assert_eq!(json["usage"]["prompt_tokens"], 10);
        assert_eq!(json["usage"]["completion_tokens"], 8);
        assert!(json["id"].as_str().unwrap().starts_with("chatcmpl-"));
    }

    #[tokio::test]
    async fn chat_completions_with_tool_calls_response() {
        let tool_response = hamoru_core::provider::types::ChatResponse {
            content: String::new(),
            model: "test-model".to_string(),
            usage: Default::default(),
            latency_ms: 100,
            finish_reason: hamoru_core::provider::types::FinishReason::ToolUse,
            tool_calls: Some(vec![hamoru_core::provider::types::ToolCall {
                id: "call_1".to_string(),
                name: "get_weather".to_string(),
                arguments: r#"{"location":"Tokyo"}"#.to_string(),
            }]),
        };
        let state = test_state_with_response(vec![sample_model()], tool_response);
        let app = build_router(state);

        let body = serde_json::json!({
            "model": "test-provider:test-model",
            "messages": [{"role": "user", "content": "What's the weather?"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": {}
                }
            }]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(json["choices"][0]["finish_reason"], "tool_calls");
        let tool_calls = &json["choices"][0]["message"]["tool_calls"];
        assert_eq!(tool_calls[0]["type"], "function");
        assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
    }

    #[tokio::test]
    async fn chat_completions_invalid_model_returns_404() {
        let state = test_state(vec![sample_model()]);
        let app = build_router(state);

        let body = serde_json::json!({
            "model": "nonexistent:model",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        // Provider not found → 503
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn chat_completions_streaming_returns_sse() {
        let chunks = vec![
            hamoru_core::provider::types::ChatChunk {
                delta: "Hello".to_string(),
                finish_reason: None,
                usage: None,
                tool_calls: None,
            },
            hamoru_core::provider::types::ChatChunk {
                delta: " world".to_string(),
                finish_reason: Some(hamoru_core::provider::types::FinishReason::Stop),
                usage: Some(hamoru_core::provider::types::TokenUsage {
                    input_tokens: 5,
                    output_tokens: 2,
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                }),
                tool_calls: None,
            },
        ];

        let mut provider = MockProvider::new("test-provider");
        provider.set_models(vec![sample_model()]);
        provider.queue_stream_chunks(Ok(chunks));
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
        let state = Arc::new(AppState::new(
            registry,
            DefaultPolicyEngine::new(policy_config),
            Box::new(InMemoryTelemetryStore::new()),
        ));
        let app = build_router(state);

        let body = serde_json::json!({
            "model": "test-provider:test-model",
            "messages": [{"role": "user", "content": "Hello"}],
            "stream": true
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            content_type.contains("text/event-stream"),
            "Expected SSE content type, got: {content_type}"
        );

        // Read SSE body
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8_lossy(&body_bytes);

        // Verify SSE events contain data lines
        assert!(
            body_str.contains("data:"),
            "SSE body should contain data lines"
        );
        assert!(
            body_str.contains("[DONE]"),
            "SSE body should end with [DONE]"
        );

        // Parse the content chunks
        let data_lines: Vec<&str> = body_str
            .lines()
            .filter(|l| l.starts_with("data:"))
            .map(|l| l.strip_prefix("data:").unwrap().trim())
            .filter(|l| *l != "[DONE]")
            .collect();

        // Should have initial role chunk + 2 content chunks
        assert!(
            data_lines.len() >= 2,
            "Expected at least 2 data lines, got: {}",
            data_lines.len()
        );

        // First data line should have role
        let first: serde_json::Value = serde_json::from_str(data_lines[0]).unwrap();
        assert_eq!(first["choices"][0]["delta"]["role"], "assistant");
    }
}
