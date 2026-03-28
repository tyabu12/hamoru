//! Layer 2: Provider Abstraction.
//!
//! Defines the unified `LlmProvider` trait and shared types for cross-layer communication.
//! Provider-specific API types (Anthropic request/response structs, etc.) must NOT
//! appear outside this module.

pub mod anthropic;
pub mod catalog;
pub mod http;
#[cfg(any(test, feature = "test-utils"))]
pub mod mock;
pub mod ollama;
pub mod retry;
pub mod types;

use std::pin::Pin;

use async_trait::async_trait;
use futures_core::Stream;

use crate::Result;
pub use types::*;

/// Unified trait for all LLM providers.
///
/// Each provider (Anthropic, Ollama, etc.) implements this trait, converting
/// between the shared types and their provider-specific API formats internally.
/// Consumers interact only with this trait and the shared types from `types` module.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Returns the provider's unique identifier (e.g., "claude", "ollama").
    fn id(&self) -> &str;

    /// Lists all models available from this provider.
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;

    /// Sends a chat completion request and returns the full response.
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse>;

    /// Sends a chat completion request and returns a stream of chunks.
    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk>> + Send>>>;

    /// Returns metadata about a specific model.
    async fn model_info(&self, model: &str) -> Result<ModelInfo>;
}

/// Registry of available LLM providers.
///
/// Holds all configured providers and allows lookup by provider ID.
/// The internal collection is private — use `register()`, `get()`, and `iter()`
/// to interact with providers.
pub struct ProviderRegistry {
    providers: Vec<Box<dyn LlmProvider>>,
}

impl ProviderRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Registers a provider in the registry.
    pub fn register(&mut self, provider: Box<dyn LlmProvider>) {
        self.providers.push(provider);
    }

    /// Looks up a provider by its ID.
    pub fn get(&self, id: &str) -> Option<&dyn LlmProvider> {
        self.providers.iter().find(|p| p.id() == id).map(|p| &**p)
    }

    /// Returns an iterator over all registered providers.
    pub fn iter(&self) -> impl Iterator<Item = &dyn LlmProvider> {
        self.providers.iter().map(|p| &**p)
    }

    /// Returns the number of registered providers.
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Returns `true` if no providers are registered.
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRegistry")
            .field("provider_count", &self.providers.len())
            .finish()
    }
}
