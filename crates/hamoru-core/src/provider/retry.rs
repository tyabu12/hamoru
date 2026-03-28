//! Retry wrapper for LLM providers.
//!
//! `RetryProvider` wraps any `LlmProvider` with exponential backoff retry logic.
//! This is a decorator pattern — consumers see a normal `LlmProvider`.

use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use futures_core::Stream;
use rand::Rng;

use super::LlmProvider;
use super::types::*;
use crate::Result;
use crate::error::HamoruError;

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (excludes the initial attempt).
    pub max_retries: u32,
    /// Initial backoff duration before the first retry.
    pub initial_backoff: Duration,
    /// Maximum backoff duration (caps exponential growth).
    pub max_backoff: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
        }
    }
}

/// Provider wrapper that retries transient failures with exponential backoff.
///
/// Wraps any `LlmProvider` and transparently retries on transient errors.
/// Non-retryable errors (auth, not found) are returned immediately.
/// For `chat_stream`, only the initial connection is retried — not mid-stream errors.
pub struct RetryProvider {
    inner: Box<dyn LlmProvider>,
    config: RetryConfig,
}

impl RetryProvider {
    /// Creates a new retry wrapper around the given provider.
    pub fn new(inner: Box<dyn LlmProvider>, config: RetryConfig) -> Self {
        Self { inner, config }
    }

    /// Executes an async operation with retry logic.
    async fn with_retry<F, Fut, T>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut last_error: HamoruError;

        // First attempt (always runs)
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) if !e.is_retryable() => return Err(e),
            Err(e) => last_error = e,
        }

        // Retry attempts with exponential backoff + full jitter
        for attempt in 1..=self.config.max_retries {
            // Full jitter: sleep = random(0, min(cap, base * 2^attempt))
            // Avoids thundering herd while staying within configured bounds.
            let base = self
                .config
                .initial_backoff
                .saturating_mul(2u32.saturating_pow(attempt));
            let capped_ms = base.min(self.config.max_backoff).as_millis() as u64;
            let sleep_ms = rand::rng().random_range(0..=capped_ms);
            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;

            match operation().await {
                Ok(result) => return Ok(result),
                Err(e) if !e.is_retryable() => return Err(e),
                Err(e) => last_error = e,
            }
        }

        Err(HamoruError::ProviderRequestFailed {
            attempts: self.config.max_retries + 1,
            source: Box::new(last_error),
        })
    }
}

#[async_trait]
impl LlmProvider for RetryProvider {
    fn id(&self) -> &str {
        self.inner.id()
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.with_retry(|| self.inner.list_models()).await
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.with_retry(|| self.inner.chat(request.clone())).await
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk>> + Send>>> {
        // Retry only the connection establishment, not mid-stream errors
        self.with_retry(|| self.inner.chat_stream(request.clone()))
            .await
    }

    async fn model_info(&self, model: &str) -> Result<ModelInfo> {
        let model = model.to_string();
        self.with_retry(|| self.inner.model_info(&model)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::mock::MockProvider;

    fn make_success_response() -> ChatResponse {
        ChatResponse {
            content: "Hello!".to_string(),
            model: "test".to_string(),
            usage: TokenUsage::default(),
            latency_ms: 100,
            finish_reason: FinishReason::Stop,
            tool_calls: None,
        }
    }

    fn make_retryable_error() -> HamoruError {
        HamoruError::ProviderUnavailable {
            provider: "test".to_string(),
            reason: "rate limited".to_string(),
        }
    }

    fn make_non_retryable_error() -> HamoruError {
        HamoruError::ModelNotFound {
            provider: "test".to_string(),
            model: "unknown".to_string(),
        }
    }

    fn fast_retry_config() -> RetryConfig {
        RetryConfig {
            max_retries: 3,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
        }
    }

    #[tokio::test]
    async fn succeeds_without_retry() {
        let mock = MockProvider::new("test");
        mock.queue_chat_response(Ok(make_success_response()));

        let provider = RetryProvider::new(Box::new(mock), fast_retry_config());
        let request = ChatRequest {
            model: "test".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            tools: None,
            stream: false,
        };
        let result = provider.chat(request).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn retries_then_succeeds() {
        tokio::time::pause();

        let mock = MockProvider::new("test");
        mock.queue_chat_response(Err(make_retryable_error()));
        mock.queue_chat_response(Err(make_retryable_error()));
        mock.queue_chat_response(Ok(make_success_response()));

        let provider = RetryProvider::new(Box::new(mock), fast_retry_config());
        let request = ChatRequest {
            model: "test".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            tools: None,
            stream: false,
        };
        let result = provider.chat(request).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().content, "Hello!");
    }

    #[tokio::test]
    async fn non_retryable_error_returns_immediately() {
        let mock = MockProvider::new("test");
        mock.queue_chat_response(Err(make_non_retryable_error()));

        let provider = RetryProvider::new(Box::new(mock), fast_retry_config());
        let request = ChatRequest {
            model: "test".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            tools: None,
            stream: false,
        };
        let result = provider.chat(request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            HamoruError::ModelNotFound { .. } => {} // Expected — not wrapped in ProviderRequestFailed
            e => panic!("expected ModelNotFound, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn retry_exhaustion_returns_provider_request_failed() {
        tokio::time::pause();

        let mock = MockProvider::new("test");
        for _ in 0..4 {
            mock.queue_chat_response(Err(make_retryable_error()));
        }

        let provider = RetryProvider::new(Box::new(mock), fast_retry_config());
        let request = ChatRequest {
            model: "test".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            tools: None,
            stream: false,
        };
        let result = provider.chat(request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            HamoruError::ProviderRequestFailed { attempts, .. } => {
                assert_eq!(attempts, 4); // 1 initial + 3 retries
            }
            e => panic!("expected ProviderRequestFailed, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn id_delegates_to_inner() {
        let mock = MockProvider::new("my-provider");
        let provider = RetryProvider::new(Box::new(mock), RetryConfig::default());
        assert_eq!(provider.id(), "my-provider");
    }
}
