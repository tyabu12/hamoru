//! Ollama (local LLM) provider implementation.
//!
//! Implements `LlmProvider` for locally-hosted models via the Ollama API.
//! No API key required — communicates with a local Ollama server.

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

// ---------------------------------------------------------------------------
// Internal API types — NEVER leak outside this module
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u64>,
}

#[derive(Deserialize)]
struct OllamaResponse {
    model: String,
    message: OllamaResponseMessage,
    #[allow(dead_code)] // Read but not directly used in non-streaming
    done: bool,
    #[serde(default)]
    eval_count: Option<u64>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModelTag>,
}

#[derive(Deserialize)]
struct OllamaModelTag {
    name: String,
}

// ---------------------------------------------------------------------------
// Conversion functions
// ---------------------------------------------------------------------------

/// Builds an Ollama API request from a shared `ChatRequest`.
///
/// System messages are kept in the messages array (Ollama supports them natively).
/// Multimodal content is flattened to text-only (Ollama base API doesn't support images).
fn build_ollama_request(request: &ChatRequest) -> OllamaRequest {
    let messages: Vec<OllamaMessage> = request
        .messages
        .iter()
        .map(|msg| {
            let role = match msg.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "user", // Ollama doesn't have a tool role
            };
            let content = match &msg.content {
                MessageContent::Text(text) => text.clone(),
                MessageContent::Parts(parts) => parts
                    .iter()
                    .filter_map(|p| match p {
                        ContentPart::Text { text } => Some(text.as_str()),
                        _ => None, // skip images
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            };
            OllamaMessage {
                role: role.to_string(),
                content,
            }
        })
        .collect();

    let options = if request.temperature.is_some() || request.max_tokens.is_some() {
        Some(OllamaOptions {
            temperature: request.temperature,
            num_predict: request.max_tokens,
        })
    } else {
        None
    };

    OllamaRequest {
        model: request.model.clone(),
        messages,
        stream: request.stream,
        options,
    }
}

/// Converts an Ollama API response into a shared `ChatResponse`.
fn parse_ollama_response(response: OllamaResponse, latency_ms: u64) -> ChatResponse {
    ChatResponse {
        content: response.message.content,
        model: response.model,
        usage: TokenUsage {
            input_tokens: response.prompt_eval_count.unwrap_or(0),
            output_tokens: response.eval_count.unwrap_or(0),
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        },
        latency_ms,
        finish_reason: FinishReason::Stop,
        tool_calls: None, // Ollama doesn't support tool calling in Phase 1
    }
}

// ---------------------------------------------------------------------------
// NDJSON streaming parser
// ---------------------------------------------------------------------------

/// NDJSON parser state for `futures::stream::unfold`.
struct NdjsonState<S> {
    stream: S,
    buffer: String,
    provider_id: String,
}

/// Parses an Ollama NDJSON byte stream into a stream of `ChatChunk`s.
///
/// Each line is a complete JSON object. When `done: true`, the final chunk
/// includes token usage statistics.
fn parse_ndjson_stream(
    byte_stream: impl Stream<Item = reqwest::Result<bytes::Bytes>> + Send + 'static,
    provider_id: String,
) -> Pin<Box<dyn Stream<Item = Result<ChatChunk>> + Send>> {
    let state = NdjsonState {
        stream: Box::pin(byte_stream),
        buffer: String::new(),
        provider_id,
    };

    Box::pin(stream::unfold(state, |mut state| async move {
        loop {
            // Try to extract a complete line from the buffer
            if let Some(newline_pos) = state.buffer.find('\n') {
                let line = state.buffer[..newline_pos].to_string();
                state.buffer = state.buffer[newline_pos + 1..].to_string();

                if line.trim().is_empty() {
                    continue;
                }

                let chunk: OllamaResponse = match serde_json::from_str(&line) {
                    Ok(c) => c,
                    Err(e) => {
                        return Some((
                            Err(HamoruError::ProviderUnavailable {
                                provider: state.provider_id.clone(),
                                reason: format!("Failed to parse NDJSON line: {e}"),
                            }),
                            state,
                        ));
                    }
                };

                if chunk.done {
                    // Final chunk with usage
                    let chat_chunk = ChatChunk {
                        delta: chunk.message.content,
                        finish_reason: Some(FinishReason::Stop),
                        usage: Some(TokenUsage {
                            input_tokens: chunk.prompt_eval_count.unwrap_or(0),
                            output_tokens: chunk.eval_count.unwrap_or(0),
                            cache_creation_input_tokens: None,
                            cache_read_input_tokens: None,
                        }),
                        tool_calls: None,
                    };
                    return Some((Ok(chat_chunk), state));
                }

                // Intermediate chunk
                let chat_chunk = ChatChunk {
                    delta: chunk.message.content,
                    finish_reason: None,
                    usage: None,
                    tool_calls: None,
                };
                return Some((Ok(chat_chunk), state));
            }

            // Read more bytes
            match state.stream.next().await {
                Some(Ok(bytes)) => match std::str::from_utf8(&bytes) {
                    Ok(text) => state.buffer.push_str(text),
                    Err(e) => {
                        return Some((
                            Err(HamoruError::ProviderUnavailable {
                                provider: state.provider_id.clone(),
                                reason: format!("Invalid UTF-8 in NDJSON stream: {e}"),
                            }),
                            state,
                        ));
                    }
                },
                Some(Err(e)) => {
                    return Some((
                        Err(HamoruError::ProviderUnavailable {
                            provider: state.provider_id.clone(),
                            reason: format!("NDJSON stream interrupted: {e}"),
                        }),
                        state,
                    ));
                }
                None => return None,
            }
        }
    }))
}

// ---------------------------------------------------------------------------
// Provider implementation
// ---------------------------------------------------------------------------

/// Ollama provider for locally-hosted models.
///
/// Communicates with a local Ollama server (default: `http://localhost:11434`).
#[derive(Debug)]
pub struct OllamaProvider {
    /// HTTP client for API requests.
    client: reqwest::Client,
    /// Provider name from config (used as provider ID).
    name: String,
    /// Base URL for the Ollama API.
    base_url: String,
    /// Config model entries for filtering (empty = all).
    model_entries: Vec<ModelEntry>,
}

impl OllamaProvider {
    /// Creates a new Ollama provider.
    ///
    /// # Arguments
    /// * `name` - Provider ID from config (returned by `id()`).
    /// * `base_url` - Base URL for the Ollama API.
    /// * `model_entries` - Config model entries for filtering (empty = all).
    pub fn new(name: String, base_url: String, model_entries: Vec<ModelEntry>) -> Result<Self> {
        let client = build_client(DEFAULT_TIMEOUT).map_err(|e| HamoruError::ConfigError {
            reason: format!("Failed to build HTTP client: {e}"),
        })?;
        Ok(Self {
            client,
            name,
            base_url,
            model_entries,
        })
    }

    /// Maps a connection error to a user-friendly message.
    fn connection_error(&self, e: reqwest::Error) -> HamoruError {
        HamoruError::ProviderUnavailable {
            provider: self.name.clone(),
            reason: format!(
                "Failed to reach Ollama at {}: {e}. Is Ollama running? Start with: ollama serve",
                self.base_url
            ),
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn id(&self) -> &str {
        &self.name
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        // Start with catalog models
        let mut models = catalog::default_models(&ProviderType::Ollama);
        for model in &mut models {
            model.provider = self.name.clone();
        }

        // Try to fetch models from the running Ollama server
        match self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                if let Ok(tags) = response.json::<OllamaTagsResponse>().await {
                    // Add models from server that aren't in the catalog
                    for tag in &tags.models {
                        if !models.iter().any(|m| m.id == tag.name) {
                            models.push(ModelInfo {
                                id: tag.name.clone(),
                                provider: self.name.clone(),
                                context_window: 0, // unknown
                                cost_per_input_token: 0.0,
                                cost_per_output_token: 0.0,
                                cost_per_cached_input_token: None,
                                capabilities: vec![Capability::Chat],
                                max_output_tokens: None,
                            });
                        }
                    }
                }
            }
            _ => {
                // Server unreachable — return catalog models only
            }
        }

        catalog::apply_overrides(&mut models, &self.model_entries);
        Ok(models)
    }

    #[instrument(skip_all, fields(provider = "ollama", model = %request.model))]
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let mut api_request = build_ollama_request(&request);
        api_request.stream = false;

        let start = Instant::now();
        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&api_request)
            .send()
            .await
            .map_err(|e| self.connection_error(e))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(map_http_error(status, &body, &self.name, &request.model));
        }

        let latency_ms = start.elapsed().as_millis() as u64;
        let api_response: OllamaResponse =
            response
                .json()
                .await
                .map_err(|e| HamoruError::ProviderUnavailable {
                    provider: self.name.clone(),
                    reason: format!("Failed to parse Ollama response: {e}"),
                })?;

        Ok(parse_ollama_response(api_response, latency_ms))
    }

    #[instrument(skip_all, fields(provider = "ollama", model = %request.model))]
    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk>> + Send>>> {
        let mut api_request = build_ollama_request(&request);
        api_request.stream = true;

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&api_request)
            .send()
            .await
            .map_err(|e| self.connection_error(e))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(map_http_error(status, &body, &self.name, &request.model));
        }

        Ok(parse_ndjson_stream(
            response.bytes_stream(),
            self.name.clone(),
        ))
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
    fn build_request_simple() {
        let request = ChatRequest {
            model: "llama3.3:70b".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("Hello".to_string()),
            }],
            temperature: None,
            max_tokens: None,
            tools: None,
            tool_choice: None,
            stream: false,
        };

        let api_req = build_ollama_request(&request);
        assert_eq!(api_req.model, "llama3.3:70b");
        assert_eq!(api_req.messages.len(), 1);
        assert_eq!(api_req.messages[0].role, "user");
        assert_eq!(api_req.messages[0].content, "Hello");
        assert!(!api_req.stream);
        assert!(api_req.options.is_none());
    }

    #[test]
    fn build_request_system_message_kept() {
        let request = ChatRequest {
            model: "llama3.3:70b".to_string(),
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

        let api_req = build_ollama_request(&request);
        // System message stays in messages array (Ollama supports it)
        assert_eq!(api_req.messages.len(), 2);
        assert_eq!(api_req.messages[0].role, "system");
        assert_eq!(api_req.messages[0].content, "You are helpful.");
        assert_eq!(api_req.messages[1].role, "user");
    }

    #[test]
    fn build_request_with_options() {
        let request = ChatRequest {
            model: "llama3.3:70b".to_string(),
            messages: vec![],
            temperature: Some(0.5),
            max_tokens: Some(200),
            tools: None,
            tool_choice: None,
            stream: false,
        };

        let api_req = build_ollama_request(&request);
        let options = api_req.options.expect("should have options");
        assert_eq!(options.temperature, Some(0.5));
        assert_eq!(options.num_predict, Some(200));
    }

    #[test]
    fn build_request_multimodal_flattened_to_text() {
        let request = ChatRequest {
            model: "llama3.3:70b".to_string(),
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
            max_tokens: None,
            tools: None,
            tool_choice: None,
            stream: false,
        };

        let api_req = build_ollama_request(&request);
        // Image is stripped, only text remains
        assert_eq!(api_req.messages[0].content, "What is this?");
    }

    // -----------------------------------------------------------------------
    // Response conversion tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_response_token_counts() {
        let response = OllamaResponse {
            model: "llama3.3:70b".to_string(),
            message: OllamaResponseMessage {
                content: "Hello!".to_string(),
            },
            done: true,
            eval_count: Some(10),
            prompt_eval_count: Some(5),
        };

        let chat_resp = parse_ollama_response(response, 50);
        assert_eq!(chat_resp.content, "Hello!");
        assert_eq!(chat_resp.model, "llama3.3:70b");
        assert_eq!(chat_resp.usage.input_tokens, 5);
        assert_eq!(chat_resp.usage.output_tokens, 10);
        assert_eq!(chat_resp.latency_ms, 50);
        assert_eq!(chat_resp.finish_reason, FinishReason::Stop);
    }

    #[test]
    fn parse_response_missing_counts() {
        let response = OllamaResponse {
            model: "llama3.3:70b".to_string(),
            message: OllamaResponseMessage {
                content: "Hi".to_string(),
            },
            done: true,
            eval_count: None,
            prompt_eval_count: None,
        };

        let chat_resp = parse_ollama_response(response, 30);
        assert_eq!(chat_resp.usage.input_tokens, 0);
        assert_eq!(chat_resp.usage.output_tokens, 0);
    }

    // -----------------------------------------------------------------------
    // NDJSON parser tests
    // -----------------------------------------------------------------------

    fn make_ndjson_bytes(
        raw: &str,
    ) -> impl Stream<Item = reqwest::Result<bytes::Bytes>> + Send + 'static {
        let data = bytes::Bytes::from(raw.to_string());
        futures::stream::once(async move { Ok(data) })
    }

    fn make_split_ndjson_bytes(
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
    async fn parse_ndjson_single_line() {
        let raw = r#"{"model":"llama3.3:70b","message":{"content":"Hi"},"done":true,"eval_count":3,"prompt_eval_count":2}
"#;

        let stream = parse_ndjson_stream(make_ndjson_bytes(raw), "test".into());
        let chunks: Vec<_> = stream.collect().await;

        assert_eq!(chunks.len(), 1);
        let c = chunks[0].as_ref().unwrap();
        assert_eq!(c.delta, "Hi");
        assert_eq!(c.finish_reason, Some(FinishReason::Stop));
        let usage = c.usage.as_ref().unwrap();
        assert_eq!(usage.input_tokens, 2);
        assert_eq!(usage.output_tokens, 3);
    }

    #[tokio::test]
    async fn parse_ndjson_multi_lines() {
        let raw = concat!(
            r#"{"model":"llama3.3:70b","message":{"content":"Hel"},"done":false}"#,
            "\n",
            r#"{"model":"llama3.3:70b","message":{"content":"lo"},"done":false}"#,
            "\n",
            r#"{"model":"llama3.3:70b","message":{"content":"!"},"done":true,"eval_count":5,"prompt_eval_count":3}"#,
            "\n",
        );

        let stream = parse_ndjson_stream(make_ndjson_bytes(raw), "test".into());
        let chunks: Vec<_> = stream.collect().await;

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].as_ref().unwrap().delta, "Hel");
        assert!(chunks[0].as_ref().unwrap().finish_reason.is_none());
        assert_eq!(chunks[1].as_ref().unwrap().delta, "lo");
        assert_eq!(chunks[2].as_ref().unwrap().delta, "!");
        assert_eq!(
            chunks[2].as_ref().unwrap().finish_reason,
            Some(FinishReason::Stop)
        );
    }

    #[tokio::test]
    async fn parse_ndjson_split_bytes() {
        let chunks_data = vec![
            r#"{"model":"llama3.3:70b","message":{"content":"sp"#,
            r#"lit"},"done":true,"eval_count":1}"#,
            "\n",
        ];

        let stream = parse_ndjson_stream(make_split_ndjson_bytes(chunks_data), "test".into());
        let chunks: Vec<_> = stream.collect().await;

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].as_ref().unwrap().delta, "split");
    }

    // -----------------------------------------------------------------------
    // Provider method tests (no network)
    // -----------------------------------------------------------------------

    #[test]
    fn id_returns_config_name() {
        let provider = OllamaProvider::new("local".into(), "http://localhost:11434".into(), vec![])
            .expect("provider creation should succeed");
        assert_eq!(provider.id(), "local");
    }

    #[tokio::test]
    async fn model_info_not_found() {
        let provider = OllamaProvider::new("local".into(), "http://localhost:11434".into(), vec![])
            .expect("provider creation should succeed");
        let result = provider.model_info("nonexistent").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            HamoruError::ModelNotFound { provider, model } => {
                assert_eq!(provider, "local");
                assert_eq!(model, "nonexistent");
            }
            e => panic!("expected ModelNotFound, got {e:?}"),
        }
    }
}
