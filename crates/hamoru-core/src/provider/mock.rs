//! Mock provider for testing.
//!
//! Configurable to return fixed responses, simulate errors, or record calls.
//! Available when the `test-utils` feature is enabled or in test builds.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Mutex;

use async_trait::async_trait;
use futures::stream;
use futures_core::Stream;

use super::LlmProvider;
use super::types::*;
use crate::Result;

/// Mock LLM provider for testing.
///
/// Responses are queued via `VecDeque` — each `chat()` call pops the next
/// response from the front. This allows sequencing: fail, fail, succeed.
pub struct MockProvider {
    id: String,
    chat_responses: Mutex<VecDeque<Result<ChatResponse>>>,
    stream_chunks: Mutex<VecDeque<Result<Vec<ChatChunk>>>>,
    models: Vec<ModelInfo>,
    call_log: Mutex<Vec<ChatRequest>>,
}

impl MockProvider {
    /// Creates a new mock provider with the given ID.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            chat_responses: Mutex::new(VecDeque::new()),
            stream_chunks: Mutex::new(VecDeque::new()),
            models: Vec::new(),
            call_log: Mutex::new(Vec::new()),
        }
    }

    /// Queues a chat response to be returned by the next `chat()` call.
    pub fn queue_chat_response(&self, response: Result<ChatResponse>) {
        self.chat_responses
            .lock()
            .expect("MockProvider mutex poisoned")
            .push_back(response);
    }

    /// Queues stream chunks for the next `chat_stream()` call.
    pub fn queue_stream_chunks(&self, chunks: Result<Vec<ChatChunk>>) {
        self.stream_chunks
            .lock()
            .expect("MockProvider mutex poisoned")
            .push_back(chunks);
    }

    /// Sets the models returned by `list_models()` and `model_info()`.
    pub fn set_models(&mut self, models: Vec<ModelInfo>) {
        self.models = models;
    }

    /// Returns all `ChatRequest`s that were passed to `chat()`.
    pub fn call_log(&self) -> Vec<ChatRequest> {
        self.call_log
            .lock()
            .expect("MockProvider mutex poisoned")
            .clone()
    }

    /// Returns the number of `chat()` calls made.
    pub fn call_count(&self) -> usize {
        self.call_log
            .lock()
            .expect("MockProvider mutex poisoned")
            .len()
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(self.models.clone())
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.call_log
            .lock()
            .expect("MockProvider mutex poisoned")
            .push(request);
        self.chat_responses
            .lock()
            .expect("MockProvider mutex poisoned")
            .pop_front()
            .expect("MockProvider: no more queued chat responses. Queue responses with queue_chat_response().")
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk>> + Send>>> {
        self.call_log
            .lock()
            .expect("MockProvider mutex poisoned")
            .push(request);
        let chunks = self
            .stream_chunks
            .lock()
            .expect("MockProvider mutex poisoned")
            .pop_front()
            .expect(
                "MockProvider: no more queued stream chunks. Queue with queue_stream_chunks().",
            )?;
        Ok(Box::pin(stream::iter(chunks.into_iter().map(Ok))))
    }

    async fn model_info(&self, model: &str) -> Result<ModelInfo> {
        self.models
            .iter()
            .find(|m| m.id == model)
            .cloned()
            .ok_or_else(|| crate::error::HamoruError::ModelNotFound {
                provider: self.id.clone(),
                model: model.to_string(),
            })
    }
}
