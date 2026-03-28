//! Anthropic (Claude) provider implementation.
//!
//! Implements `LlmProvider` using the Anthropic Messages API directly
//! via reqwest + serde. No third-party abstraction libraries.

use std::pin::Pin;

use async_trait::async_trait;
use futures_core::Stream;

use super::{ChatChunk, ChatRequest, ChatResponse, LlmProvider, ModelInfo};
use crate::Result;

/// Anthropic provider for Claude models.
///
/// Communicates with the Anthropic Messages API (`/v1/messages`).
#[allow(dead_code)] // Fields used in Phase 1 implementation.
pub struct AnthropicProvider {
    /// HTTP client for API requests.
    client: reqwest::Client,
    /// API key for authentication (from environment variable).
    api_key: String,
    /// Base URL for the Anthropic API.
    base_url: String,
}

// Manual Debug impl to redact the API key — SECURITY: never log credentials.
impl std::fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl AnthropicProvider {
    /// Creates a new Anthropic provider with the given API key and base URL.
    pub fn new(api_key: String, base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url,
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn id(&self) -> &str {
        "claude"
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        todo!()
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        todo!()
    }

    async fn chat_stream(
        &self,
        _request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk>> + Send>>> {
        todo!()
    }

    async fn model_info(&self, _model: &str) -> Result<ModelInfo> {
        todo!()
    }
}
