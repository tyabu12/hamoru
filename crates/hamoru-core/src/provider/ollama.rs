//! Ollama (local LLM) provider implementation.
//!
//! Implements `LlmProvider` for locally-hosted models via the Ollama API.
//! No API key required — communicates with a local Ollama server.

use std::pin::Pin;

use async_trait::async_trait;
use futures_core::Stream;

use super::{ChatChunk, ChatRequest, ChatResponse, LlmProvider, ModelInfo};
use crate::Result;

/// Ollama provider for locally-hosted models.
///
/// Communicates with a local Ollama server (default: `http://localhost:11434`).
#[derive(Debug)]
#[allow(dead_code)] // Fields used in Phase 1 implementation.
pub struct OllamaProvider {
    /// HTTP client for API requests.
    client: reqwest::Client,
    /// Base URL for the Ollama API.
    base_url: String,
}

impl OllamaProvider {
    /// Creates a new Ollama provider with the given base URL.
    pub fn new(base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn id(&self) -> &str {
        "ollama"
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
