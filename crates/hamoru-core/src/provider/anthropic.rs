//! Anthropic (Claude) provider implementation.
//!
//! Implements `LlmProvider` using the Anthropic Messages API directly
//! via reqwest + serde. No third-party abstraction libraries.

use std::pin::Pin;
use std::time::Instant;

use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::LlmProvider;
use super::catalog;
use super::http::{DEFAULT_TIMEOUT, build_client, map_http_error};
use super::types::*;
use crate::Result;
use crate::config::{ModelEntry, ProviderType};
use crate::error::HamoruError;

/// Anthropic Messages API version header.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Default max tokens when not specified and model not in catalog.
const DEFAULT_MAX_TOKENS: u64 = 4096;

// ---------------------------------------------------------------------------
// Internal API types — NEVER leak outside this module
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicResponseContent>,
    model: String,
    usage: AnthropicUsage,
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicResponseContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
    id: Option<String>,
    name: Option<String>,
    input: Option<serde_json::Value>,
}

#[derive(Deserialize, Clone)]
struct AnthropicUsage {
    /// Input tokens (absent in `message_delta` events, present in `message_start`).
    #[serde(default)]
    input_tokens: u64,
    /// Output tokens.
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
}

// ---------------------------------------------------------------------------
// Conversion functions
// ---------------------------------------------------------------------------

/// Builds an Anthropic API request from a shared `ChatRequest`.
///
/// Extracts the first System message into the top-level `system` field
/// (Anthropic API requires system prompt at top level, not in messages array).
fn build_anthropic_request(request: &ChatRequest, default_max_tokens: u64) -> AnthropicRequest {
    let mut system_text: Option<String> = None;
    let mut messages = Vec::new();

    for msg in &request.messages {
        if msg.role == Role::System {
            // Extract system message to top-level field
            system_text = Some(message_content_to_text(&msg.content));
            continue;
        }

        let role = match msg.role {
            Role::User | Role::Tool => "user",
            Role::Assistant => "assistant",
            Role::System => unreachable!(), // handled above
        };

        let content = match &msg.content {
            MessageContent::Text(text) => serde_json::Value::String(text.clone()),
            MessageContent::Parts(parts) => {
                let blocks: Vec<serde_json::Value> = parts
                    .iter()
                    .map(|part| match part {
                        ContentPart::Text { text } => serde_json::json!({
                            "type": "text",
                            "text": text,
                        }),
                        ContentPart::ImageBase64 { media_type, data } => serde_json::json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": media_type,
                                "data": data,
                            }
                        }),
                        ContentPart::ImageUrl { url } => serde_json::json!({
                            "type": "image",
                            "source": {
                                "type": "url",
                                "url": url,
                            }
                        }),
                        ContentPart::ToolUse { id, name, input } => serde_json::json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input,
                        }),
                        ContentPart::ToolResult {
                            tool_use_id,
                            content,
                        } => serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": content,
                        }),
                    })
                    .collect();
                serde_json::Value::Array(blocks)
            }
        };

        messages.push(AnthropicMessage {
            role: role.to_string(),
            content,
        });
    }

    let max_tokens = request.max_tokens.unwrap_or(default_max_tokens);

    let tools = request.tools.as_ref().map(|tools| {
        tools
            .iter()
            .map(|t| AnthropicTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            })
            .collect()
    });

    let tool_choice = request.tool_choice.as_ref().map(|tc| match tc {
        ToolChoice::Auto => serde_json::json!({"type": "auto"}),
        // Anthropic has no "none" equivalent; map to "auto" and let the model decide.
        // The provider will still respect the absence of tool_choice if tools are omitted.
        ToolChoice::None => serde_json::json!({"type": "auto"}),
        ToolChoice::Required => serde_json::json!({"type": "any"}),
        ToolChoice::Tool { name } => serde_json::json!({"type": "tool", "name": name}),
    });

    AnthropicRequest {
        model: request.model.clone(),
        messages,
        system: system_text,
        max_tokens,
        temperature: request.temperature,
        stream: request.stream,
        tools,
        tool_choice,
    }
}

/// Converts an Anthropic API response into a shared `ChatResponse`.
fn parse_anthropic_response(response: AnthropicResponse, latency_ms: u64) -> ChatResponse {
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for block in &response.content {
        match block.content_type.as_str() {
            "text" => {
                if let Some(text) = &block.text {
                    text_parts.push(text.as_str());
                }
            }
            "tool_use" => {
                if let (Some(id), Some(name), Some(input)) = (&block.id, &block.name, &block.input)
                {
                    tool_calls.push(ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: input.to_string(),
                    });
                }
            }
            _ => {} // ignore unknown block types
        }
    }

    let finish_reason = response
        .stop_reason
        .as_deref()
        .map(map_stop_reason)
        .unwrap_or(FinishReason::Stop);

    ChatResponse {
        content: text_parts.join(""),
        model: response.model,
        usage: TokenUsage {
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            cache_creation_input_tokens: response.usage.cache_creation_input_tokens,
            cache_read_input_tokens: response.usage.cache_read_input_tokens,
        },
        latency_ms,
        finish_reason,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
    }
}

/// Maps Anthropic stop_reason strings to shared FinishReason.
fn map_stop_reason(reason: &str) -> FinishReason {
    match reason {
        "end_turn" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "tool_use" => FinishReason::ToolUse,
        _ => FinishReason::Stop, // conservative default
    }
}

/// Extracts plain text from MessageContent (flattens Parts to text-only).
fn message_content_to_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text(text) => text.clone(),
        MessageContent::Parts(parts) => parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
    }
}

// ---------------------------------------------------------------------------
// SSE streaming types and parser
// ---------------------------------------------------------------------------

/// A single SSE event from the Anthropic streaming API.
#[derive(Deserialize)]
struct SseEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    delta: Option<SseDelta>,
    #[serde(default)]
    message: Option<SseMessageWrapper>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    error: Option<SseError>,
}

#[derive(Deserialize)]
struct SseDelta {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct SseMessageWrapper {
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct SseError {
    message: String,
}

/// SSE parser state for `futures::stream::unfold`.
struct SseState<S> {
    /// Inner byte stream from reqwest.
    stream: S,
    /// Buffer for incomplete lines.
    buffer: String,
    /// Input tokens from `message_start` event, carried to final chunk.
    input_tokens: u64,
    /// Provider ID for error messages.
    provider_id: String,
    /// Whether the stream has finished.
    finished: bool,
}

/// Parses an Anthropic SSE byte stream into a stream of `ChatChunk`s.
///
/// Handles event types: `message_start`, `content_block_delta`,
/// `message_delta`, `message_stop`, `ping`, and `error`.
fn parse_sse_stream(
    byte_stream: impl Stream<Item = reqwest::Result<bytes::Bytes>> + Send + 'static,
    provider_id: String,
) -> Pin<Box<dyn Stream<Item = Result<ChatChunk>> + Send>> {
    let state = SseState {
        stream: Box::pin(byte_stream),
        buffer: String::new(),
        input_tokens: 0,
        provider_id,
        finished: false,
    };

    Box::pin(stream::unfold(state, |mut state| async move {
        if state.finished {
            return None;
        }

        loop {
            // Try to extract a complete SSE event from the buffer
            if let Some(chunk_result) = extract_next_event(&mut state) {
                return Some((chunk_result, state));
            }

            // Read more bytes from the inner stream
            match state.stream.next().await {
                Some(Ok(bytes)) => {
                    // Append new bytes to buffer
                    match std::str::from_utf8(&bytes) {
                        Ok(text) => state.buffer.push_str(text),
                        Err(e) => {
                            state.finished = true;
                            return Some((
                                Err(HamoruError::ProviderUnavailable {
                                    provider: state.provider_id.clone(),
                                    reason: format!("Invalid UTF-8 in SSE stream: {e}"),
                                }),
                                state,
                            ));
                        }
                    }
                }
                Some(Err(e)) => {
                    state.finished = true;
                    return Some((
                        Err(HamoruError::ProviderUnavailable {
                            provider: state.provider_id.clone(),
                            reason: format!("SSE stream interrupted: {e}"),
                        }),
                        state,
                    ));
                }
                None => {
                    // Stream ended without message_stop
                    return None;
                }
            }
        }
    }))
}

/// Tries to extract the next SSE event from the buffer and convert it to a ChatChunk.
/// Returns `None` if no complete event is available yet.
fn extract_next_event<S>(state: &mut SseState<S>) -> Option<Result<ChatChunk>> {
    // SSE events are delimited by double newlines
    while let Some(boundary) = state.buffer.find("\n\n") {
        let event_block = state.buffer[..boundary].to_string();
        state.buffer = state.buffer[boundary + 2..].to_string();

        // Collect data lines from the event block
        let mut data_parts = Vec::new();
        for line in event_block.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                data_parts.push(data);
            }
            // Skip "event:", "id:", ":" (comment), and other lines
        }

        if data_parts.is_empty() {
            continue;
        }

        let data = data_parts.join("\n");

        // Parse the JSON data
        let event: SseEvent = match serde_json::from_str(&data) {
            Ok(e) => e,
            Err(_) => continue, // skip unparseable events
        };

        match event.event_type.as_str() {
            "message_start" => {
                // Store input tokens for later
                if let Some(msg) = &event.message
                    && let Some(usage) = &msg.usage
                {
                    state.input_tokens = usage.input_tokens;
                }
                // No chunk to yield
            }
            "content_block_start" | "content_block_stop" => {
                // No chunk to yield
            }
            "content_block_delta" => {
                if let Some(delta) = &event.delta
                    && let Some(text) = &delta.text
                    && !text.is_empty()
                {
                    return Some(Ok(ChatChunk {
                        delta: text.clone(),
                        finish_reason: None,
                        usage: None,
                        tool_calls: None,
                    }));
                }
            }
            "message_delta" => {
                // Final chunk: contains usage and stop_reason
                let finish_reason = event.stop_reason.as_deref().map(map_stop_reason);
                let usage = event.usage.map(|u| TokenUsage {
                    input_tokens: state.input_tokens,
                    output_tokens: u.output_tokens,
                    cache_creation_input_tokens: u.cache_creation_input_tokens,
                    cache_read_input_tokens: u.cache_read_input_tokens,
                });
                return Some(Ok(ChatChunk {
                    delta: String::new(),
                    finish_reason,
                    usage,
                    tool_calls: None,
                }));
            }
            "message_stop" => {
                state.finished = true;
                return None;
            }
            "ping" => {
                // Ignore
            }
            "error" => {
                state.finished = true;
                let msg = event
                    .error
                    .map(|e| e.message)
                    .unwrap_or_else(|| "Unknown SSE error".to_string());
                return Some(Err(HamoruError::ProviderUnavailable {
                    provider: state.provider_id.clone(),
                    reason: msg,
                }));
            }
            _ => {
                // Ignore unknown event types
            }
        }
    }

    None // No complete event in buffer
}

// ---------------------------------------------------------------------------
// Provider implementation
// ---------------------------------------------------------------------------

/// Anthropic provider for Claude models.
///
/// Communicates with the Anthropic Messages API (`/v1/messages`).
pub struct AnthropicProvider {
    /// HTTP client for API requests.
    client: reqwest::Client,
    /// Provider name from config (used as provider ID).
    name: String,
    /// API key for authentication (from environment variable).
    api_key: String,
    /// Base URL for the Anthropic API.
    base_url: String,
    /// Config model entries for filtering (empty = all catalog models).
    model_entries: Vec<ModelEntry>,
}

// Manual Debug impl to redact the API key — SECURITY: never log credentials.
impl std::fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl AnthropicProvider {
    /// Creates a new Anthropic provider.
    ///
    /// # Arguments
    /// * `name` - Provider ID from config (returned by `id()`).
    /// * `api_key` - Anthropic API key.
    /// * `base_url` - Base URL for the Anthropic API.
    /// * `model_entries` - Config model entries for filtering (empty = all).
    pub fn new(
        name: String,
        api_key: String,
        base_url: String,
        model_entries: Vec<ModelEntry>,
    ) -> Result<Self> {
        let client = build_client(DEFAULT_TIMEOUT).map_err(|e| HamoruError::ConfigError {
            reason: format!("Failed to build HTTP client: {e}"),
        })?;
        Ok(Self {
            client,
            name,
            api_key,
            base_url,
            model_entries,
        })
    }

    /// Looks up the max output tokens for a model from the catalog.
    fn get_max_output_tokens(&self, model: &str) -> u64 {
        let models = catalog::default_models(&ProviderType::Anthropic);
        models
            .iter()
            .find(|m| m.id == model)
            .and_then(|m| m.max_output_tokens)
            .unwrap_or(DEFAULT_MAX_TOKENS)
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn id(&self) -> &str {
        &self.name
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let mut models = catalog::default_models(&ProviderType::Anthropic);
        // Set provider field to this instance's name
        for model in &mut models {
            model.provider = self.name.clone();
        }
        catalog::apply_overrides(&mut models, &self.model_entries);
        Ok(models)
    }

    #[instrument(skip_all, fields(provider = "anthropic", model = %request.model))]
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let default_max = self.get_max_output_tokens(&request.model);
        let api_request = build_anthropic_request(&request, default_max);

        let start = Instant::now();
        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&api_request)
            .send()
            .await
            .map_err(|e| HamoruError::ProviderUnavailable {
                provider: self.name.clone(),
                reason: format!("Failed to reach Anthropic API: {e}. Check network connectivity."),
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(map_http_error(status, &body, &self.name, &request.model));
        }

        let latency_ms = start.elapsed().as_millis() as u64;
        let api_response: AnthropicResponse =
            response
                .json()
                .await
                .map_err(|e| HamoruError::ProviderUnavailable {
                    provider: self.name.clone(),
                    reason: format!("Failed to parse Anthropic response: {e}"),
                })?;

        Ok(parse_anthropic_response(api_response, latency_ms))
    }

    #[instrument(skip_all, fields(provider = "anthropic", model = %request.model))]
    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk>> + Send>>> {
        let default_max = self.get_max_output_tokens(&request.model);
        let mut api_request = build_anthropic_request(&request, default_max);
        api_request.stream = true;

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&api_request)
            .send()
            .await
            .map_err(|e| HamoruError::ProviderUnavailable {
                provider: self.name.clone(),
                reason: format!("Failed to reach Anthropic API: {e}. Check network connectivity."),
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(map_http_error(status, &body, &self.name, &request.model));
        }

        Ok(parse_sse_stream(response.bytes_stream(), self.name.clone()))
    }

    async fn model_info(&self, model: &str) -> Result<ModelInfo> {
        let models = self.list_models().await?;
        models
            .into_iter()
            .find(|m| m.id == model)
            .ok_or_else(|| HamoruError::ModelNotFound {
                provider: self.name.clone(),
                model: model.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Request conversion tests
    // -----------------------------------------------------------------------

    #[test]
    fn build_request_simple_text() {
        let request = ChatRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("Hello".to_string()),
            }],
            temperature: Some(0.7),
            max_tokens: Some(100),
            tools: None,
            tool_choice: None,
            stream: false,
        };

        let api_req = build_anthropic_request(&request, DEFAULT_MAX_TOKENS);
        assert_eq!(api_req.model, "claude-sonnet-4-6");
        assert_eq!(api_req.messages.len(), 1);
        assert_eq!(api_req.messages[0].role, "user");
        assert_eq!(
            api_req.messages[0].content,
            serde_json::Value::String("Hello".to_string())
        );
        assert_eq!(api_req.max_tokens, 100);
        assert_eq!(api_req.temperature, Some(0.7));
        assert!(api_req.system.is_none());
        assert!(!api_req.stream);
    }

    #[test]
    fn build_request_extracts_system_message() {
        let request = ChatRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: MessageContent::Text("You are helpful.".to_string()),
                },
                Message {
                    role: Role::User,
                    content: MessageContent::Text("Hi".to_string()),
                },
            ],
            temperature: None,
            max_tokens: None,
            tools: None,
            tool_choice: None,
            stream: false,
        };

        let api_req = build_anthropic_request(&request, DEFAULT_MAX_TOKENS);
        assert_eq!(api_req.system.as_deref(), Some("You are helpful."));
        assert_eq!(api_req.messages.len(), 1); // system removed from messages
        assert_eq!(api_req.messages[0].role, "user");
    }

    #[test]
    fn build_request_multimodal_image() {
        let request = ChatRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Parts(vec![
                    ContentPart::Text {
                        text: "What is this?".to_string(),
                    },
                    ContentPart::ImageBase64 {
                        media_type: "image/png".to_string(),
                        data: "iVBOR...".to_string(),
                    },
                ]),
            }],
            temperature: None,
            max_tokens: Some(500),
            tools: None,
            tool_choice: None,
            stream: false,
        };

        let api_req = build_anthropic_request(&request, DEFAULT_MAX_TOKENS);
        let content = &api_req.messages[0].content;
        let blocks = content.as_array().expect("should be array");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["type"], "base64");
        assert_eq!(blocks[1]["source"]["media_type"], "image/png");
    }

    #[test]
    fn build_request_max_tokens_from_param() {
        let request = ChatRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: Some(256),
            tools: None,
            tool_choice: None,
            stream: false,
        };

        let api_req = build_anthropic_request(&request, 16_384);
        assert_eq!(api_req.max_tokens, 256); // explicit param wins
    }

    #[test]
    fn build_request_max_tokens_uses_default() {
        let request = ChatRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None, // not specified
            tools: None,
            tool_choice: None,
            stream: false,
        };

        let api_req = build_anthropic_request(&request, 16_384);
        assert_eq!(api_req.max_tokens, 16_384); // default from catalog
    }

    #[test]
    fn build_request_with_tools() {
        let request = ChatRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("Check status".to_string()),
            }],
            temperature: None,
            max_tokens: Some(100),
            tools: Some(vec![Tool {
                name: "report_status".to_string(),
                description: "Report task status".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }]),
            tool_choice: None,
            stream: false,
        };

        let api_req = build_anthropic_request(&request, DEFAULT_MAX_TOKENS);
        let tools = api_req.tools.expect("should have tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "report_status");
    }

    #[test]
    fn build_request_tool_choice_required() {
        let request = ChatRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: Some(100),
            tools: Some(vec![Tool {
                name: "report_status".to_string(),
                description: "Report status".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }]),
            tool_choice: Some(ToolChoice::Required),
            stream: false,
        };

        let api_req = build_anthropic_request(&request, DEFAULT_MAX_TOKENS);
        let tc = api_req.tool_choice.expect("should have tool_choice");
        assert_eq!(tc["type"], "any");
    }

    #[test]
    fn build_request_tool_choice_specific_tool() {
        let request = ChatRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: Some(100),
            tools: Some(vec![Tool {
                name: "report_status".to_string(),
                description: "Report status".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }]),
            tool_choice: Some(ToolChoice::Tool {
                name: "report_status".to_string(),
            }),
            stream: false,
        };

        let api_req = build_anthropic_request(&request, DEFAULT_MAX_TOKENS);
        let tc = api_req.tool_choice.expect("should have tool_choice");
        assert_eq!(tc["type"], "tool");
        assert_eq!(tc["name"], "report_status");
    }

    #[test]
    fn build_request_no_tool_choice_omits_field() {
        let request = ChatRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: Some(100),
            tools: None,
            tool_choice: None,
            stream: false,
        };

        let api_req = build_anthropic_request(&request, DEFAULT_MAX_TOKENS);
        assert!(api_req.tool_choice.is_none());
    }

    // -----------------------------------------------------------------------
    // Response conversion tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_response_text() {
        let response = AnthropicResponse {
            content: vec![AnthropicResponseContent {
                content_type: "text".to_string(),
                text: Some("Hello! How can I help?".to_string()),
                id: None,
                name: None,
                input: None,
            }],
            model: "claude-sonnet-4-6".to_string(),
            usage: AnthropicUsage {
                input_tokens: 10,
                output_tokens: 8,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
            stop_reason: Some("end_turn".to_string()),
        };

        let chat_resp = parse_anthropic_response(response, 150);
        assert_eq!(chat_resp.content, "Hello! How can I help?");
        assert_eq!(chat_resp.model, "claude-sonnet-4-6");
        assert_eq!(chat_resp.usage.input_tokens, 10);
        assert_eq!(chat_resp.usage.output_tokens, 8);
        assert_eq!(chat_resp.latency_ms, 150);
        assert_eq!(chat_resp.finish_reason, FinishReason::Stop);
        assert!(chat_resp.tool_calls.is_none());
    }

    #[test]
    fn parse_response_tool_use() {
        let response = AnthropicResponse {
            content: vec![
                AnthropicResponseContent {
                    content_type: "text".to_string(),
                    text: Some("Let me check.".to_string()),
                    id: None,
                    name: None,
                    input: None,
                },
                AnthropicResponseContent {
                    content_type: "tool_use".to_string(),
                    text: None,
                    id: Some("call_123".to_string()),
                    name: Some("report_status".to_string()),
                    input: Some(serde_json::json!({"status": "pass"})),
                },
            ],
            model: "claude-sonnet-4-6".to_string(),
            usage: AnthropicUsage {
                input_tokens: 20,
                output_tokens: 15,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
            stop_reason: Some("tool_use".to_string()),
        };

        let chat_resp = parse_anthropic_response(response, 200);
        assert_eq!(chat_resp.content, "Let me check.");
        assert_eq!(chat_resp.finish_reason, FinishReason::ToolUse);
        let tool_calls = chat_resp.tool_calls.expect("should have tool calls");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_123");
        assert_eq!(tool_calls[0].name, "report_status");
        assert!(tool_calls[0].arguments.contains("pass"));
    }

    #[test]
    fn parse_stop_reasons() {
        assert_eq!(map_stop_reason("end_turn"), FinishReason::Stop);
        assert_eq!(map_stop_reason("max_tokens"), FinishReason::Length);
        assert_eq!(map_stop_reason("tool_use"), FinishReason::ToolUse);
        assert_eq!(map_stop_reason("unknown_reason"), FinishReason::Stop);
    }

    #[test]
    fn parse_response_with_cache_usage() {
        let response = AnthropicResponse {
            content: vec![AnthropicResponseContent {
                content_type: "text".to_string(),
                text: Some("Cached response".to_string()),
                id: None,
                name: None,
                input: None,
            }],
            model: "claude-sonnet-4-6".to_string(),
            usage: AnthropicUsage {
                input_tokens: 5,
                output_tokens: 3,
                cache_creation_input_tokens: Some(100),
                cache_read_input_tokens: Some(50),
            },
            stop_reason: Some("end_turn".to_string()),
        };

        let chat_resp = parse_anthropic_response(response, 80);
        assert_eq!(chat_resp.usage.cache_creation_input_tokens, Some(100));
        assert_eq!(chat_resp.usage.cache_read_input_tokens, Some(50));
    }

    // -----------------------------------------------------------------------
    // Provider method tests (no network)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_models_uses_catalog() {
        let provider = AnthropicProvider::new(
            "claude".into(),
            "test-key".into(),
            "http://test".into(),
            vec![],
        )
        .expect("provider creation should succeed");

        let models = provider.list_models().await.expect("should succeed");
        assert!(!models.is_empty());
        for model in &models {
            assert_eq!(model.provider, "claude");
        }
        // Should include known models from catalog
        assert!(models.iter().any(|m| m.id == "claude-sonnet-4-6"));
    }

    #[tokio::test]
    async fn list_models_with_filter() {
        let provider = AnthropicProvider::new(
            "claude".into(),
            "test-key".into(),
            "http://test".into(),
            vec![ModelEntry::Simple("claude-sonnet-4-6".to_string())],
        )
        .expect("provider creation should succeed");

        let models = provider.list_models().await.expect("should succeed");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "claude-sonnet-4-6");
    }

    #[tokio::test]
    async fn model_info_not_found() {
        let provider = AnthropicProvider::new(
            "claude".into(),
            "test-key".into(),
            "http://test".into(),
            vec![],
        )
        .expect("provider creation should succeed");

        let result = provider.model_info("nonexistent-model").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            HamoruError::ModelNotFound { provider, model } => {
                assert_eq!(provider, "claude");
                assert_eq!(model, "nonexistent-model");
            }
            e => panic!("expected ModelNotFound, got {e:?}"),
        }
    }

    #[test]
    fn id_returns_config_name() {
        let provider = AnthropicProvider::new(
            "my-claude".into(),
            "key".into(),
            "http://test".into(),
            vec![],
        )
        .expect("provider creation should succeed");
        assert_eq!(provider.id(), "my-claude");
    }

    #[test]
    fn debug_redacts_api_key() {
        let provider = AnthropicProvider::new(
            "claude".into(),
            "super-secret".into(),
            "http://test".into(),
            vec![],
        )
        .expect("provider creation should succeed");
        let debug_str = format!("{provider:?}");
        assert!(debug_str.contains("[REDACTED]"));
        assert!(!debug_str.contains("super-secret"));
    }

    #[test]
    fn get_max_output_tokens_from_catalog() {
        let provider =
            AnthropicProvider::new("claude".into(), "key".into(), "http://test".into(), vec![])
                .expect("provider creation should succeed");
        // claude-sonnet-4-6 has max_output_tokens: 16384 in catalog
        assert_eq!(provider.get_max_output_tokens("claude-sonnet-4-6"), 16_384);
    }

    #[test]
    fn get_max_output_tokens_fallback() {
        let provider =
            AnthropicProvider::new("claude".into(), "key".into(), "http://test".into(), vec![])
                .expect("provider creation should succeed");
        assert_eq!(
            provider.get_max_output_tokens("unknown-model"),
            DEFAULT_MAX_TOKENS
        );
    }

    // -----------------------------------------------------------------------
    // SSE parser tests
    // -----------------------------------------------------------------------

    /// Helper: wraps raw SSE text into a byte stream.
    fn make_sse_bytes(
        raw: &str,
    ) -> impl Stream<Item = reqwest::Result<bytes::Bytes>> + Send + 'static {
        let data = bytes::Bytes::from(raw.to_string());
        futures::stream::once(async move { Ok(data) })
    }

    /// Helper: wraps multiple SSE chunks (simulating split byte boundaries).
    fn make_split_sse_bytes(
        chunks: Vec<&str>,
    ) -> impl Stream<Item = reqwest::Result<bytes::Bytes>> + Send + 'static {
        futures::stream::iter(
            chunks
                .into_iter()
                .map(|c| Ok(bytes::Bytes::from(c.to_string())))
                .collect::<Vec<_>>(),
        )
    }

    #[tokio::test]
    async fn parse_sse_simple_text() {
        let raw = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"stop_reason\":\"end_turn\",\"usage\":{\"output_tokens\":5}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );

        let stream = parse_sse_stream(make_sse_bytes(raw), "test".into());
        let chunks: Vec<_> = stream.collect().await;

        assert_eq!(chunks.len(), 3); // 2 deltas + 1 message_delta
        // First chunk: "Hello"
        let c0 = chunks[0].as_ref().expect("should be Ok");
        assert_eq!(c0.delta, "Hello");
        assert!(c0.finish_reason.is_none());
        assert!(c0.usage.is_none());

        // Second chunk: " world"
        let c1 = chunks[1].as_ref().expect("should be Ok");
        assert_eq!(c1.delta, " world");

        // Final chunk: usage + finish_reason
        let c2 = chunks[2].as_ref().expect("should be Ok");
        assert_eq!(c2.delta, "");
        assert_eq!(c2.finish_reason, Some(FinishReason::Stop));
        let usage = c2.usage.as_ref().expect("should have usage");
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
    }

    #[tokio::test]
    async fn parse_sse_error_event() {
        let raw = concat!(
            "event: error\n",
            "data: {\"type\":\"error\",\"error\":{\"message\":\"rate limited\"}}\n\n",
        );

        let stream = parse_sse_stream(make_sse_bytes(raw), "test".into());
        let chunks: Vec<_> = stream.collect().await;

        assert_eq!(chunks.len(), 1);
        let err = chunks[0].as_ref().unwrap_err();
        match err {
            HamoruError::ProviderUnavailable { reason, .. } => {
                assert!(reason.contains("rate limited"));
            }
            e => panic!("expected ProviderUnavailable, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn parse_sse_ping_ignored() {
        let raw = concat!(
            "event: ping\n",
            "data: {\"type\":\"ping\"}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hi\"}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );

        let stream = parse_sse_stream(make_sse_bytes(raw), "test".into());
        let chunks: Vec<_> = stream.collect().await;

        assert_eq!(chunks.len(), 1); // only the content delta
        assert_eq!(chunks[0].as_ref().unwrap().delta, "Hi");
    }

    #[tokio::test]
    async fn parse_sse_split_bytes() {
        // Data split across two byte chunks, mid-event
        let chunks_data = vec![
            "event: content_block_delta\ndata: {\"type\":\"content_block_del",
            "ta\",\"delta\":{\"text\":\"split\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ];

        let stream = parse_sse_stream(make_split_sse_bytes(chunks_data), "test".into());
        let chunks: Vec<_> = stream.collect().await;

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].as_ref().unwrap().delta, "split");
    }

    #[tokio::test]
    async fn parse_sse_final_chunk_has_usage_and_finish_reason() {
        let raw = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":42,\"output_tokens\":0}}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"stop_reason\":\"max_tokens\",\"usage\":{\"output_tokens\":100}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );

        let stream = parse_sse_stream(make_sse_bytes(raw), "test".into());
        let chunks: Vec<_> = stream.collect().await;

        assert_eq!(chunks.len(), 1);
        let c = chunks[0].as_ref().unwrap();
        assert_eq!(c.finish_reason, Some(FinishReason::Length));
        let usage = c.usage.as_ref().unwrap();
        assert_eq!(usage.input_tokens, 42);
        assert_eq!(usage.output_tokens, 100);
    }

    #[tokio::test]
    async fn parse_sse_comment_lines_ignored() {
        let raw = concat!(
            ": this is a comment\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"ok\"}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );

        let stream = parse_sse_stream(make_sse_bytes(raw), "test".into());
        let chunks: Vec<_> = stream.collect().await;

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].as_ref().unwrap().delta, "ok");
    }
}
