//! Shared types for LLM provider abstraction.
//!
//! These types define the vocabulary for cross-layer communication.
//! Provider-specific API types must NOT appear here — only the unified abstractions.

use serde::{Deserialize, Serialize};

/// Role of a message participant in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System instructions.
    System,
    /// User input.
    User,
    /// Assistant (LLM) output.
    Assistant,
    /// Tool result.
    Tool,
}

/// A single content part within a message.
///
/// Follows OpenAI's content_parts model for multimodal support.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    /// Plain text content.
    #[serde(rename = "text")]
    Text {
        /// The text content.
        text: String,
    },
    /// Image referenced by URL.
    #[serde(rename = "image_url")]
    ImageUrl {
        /// URL of the image.
        url: String,
    },
    /// Base64-encoded image data.
    #[serde(rename = "image_base64")]
    ImageBase64 {
        /// MIME type (e.g., "image/png").
        media_type: String,
        /// Base64-encoded image data.
        data: String,
    },
}

/// Content of a message — optimized for the common text-only case.
///
/// Design doc specifies `Vec<ContentPart>`, but we use an enum to avoid
/// heap-allocating a `Vec` for the 95%+ case of plain text messages.
/// `Message` flows through every trait in the system, so this optimization
/// prevents a breaking change later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain text (common case, no Vec allocation).
    Text(String),
    /// Multimodal content parts.
    Parts(Vec<ContentPart>),
}

/// A single message in a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Role of the message sender.
    pub role: Role,
    /// Content of the message.
    pub content: MessageContent,
}

/// Model capabilities for policy-based routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    /// Standard chat completion.
    Chat,
    /// Image/visual input support.
    Vision,
    /// Function/tool calling support.
    FunctionCalling,
    /// Reasoning models (o1, o3-mini, DeepSeek-R1, etc.).
    /// These have different constraints: no system prompt, no temperature, etc.
    Reasoning,
    /// Prompt caching support (e.g., Anthropic's prompt caching).
    PromptCaching,
}

/// Metadata about a model available from a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier (e.g., "claude-sonnet-4-6").
    pub id: String,
    /// Provider name (e.g., "claude").
    pub provider: String,
    /// Maximum context window in tokens.
    pub context_window: u64,
    /// Cost per input token in USD.
    pub cost_per_input_token: f64,
    /// Cost per output token in USD.
    pub cost_per_output_token: f64,
    /// Cost per cached input token in USD (prompt caching discount).
    pub cost_per_cached_input_token: Option<f64>,
    /// Capabilities supported by this model.
    pub capabilities: Vec<Capability>,
    /// Maximum output tokens, if limited.
    pub max_output_tokens: Option<u64>,
}

/// Token usage statistics for a single LLM call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Number of input tokens consumed.
    pub input_tokens: u64,
    /// Number of output tokens generated.
    pub output_tokens: u64,
    /// Tokens used to create a new prompt cache entry.
    pub cache_creation_input_tokens: Option<u64>,
    /// Tokens read from an existing prompt cache (discounted rate).
    pub cache_read_input_tokens: Option<u64>,
}

impl TokenUsage {
    /// Calculates the cost in USD based on model pricing.
    pub fn calculate_cost(&self, model_info: &ModelInfo) -> f64 {
        let input_cost = self.input_tokens as f64 * model_info.cost_per_input_token;
        let output_cost = self.output_tokens as f64 * model_info.cost_per_output_token;
        let cached_cost = self.cache_read_input_tokens.unwrap_or(0) as f64
            * model_info
                .cost_per_cached_input_token
                .unwrap_or(model_info.cost_per_input_token);
        input_cost + output_cost + cached_cost
    }
}

/// A tool definition passed to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Tool name (e.g., "report_status").
    pub name: String,
    /// Human-readable description of the tool.
    pub description: String,
    /// JSON Schema defining the tool's parameters.
    pub parameters: serde_json::Value,
}

/// A tool call made by the LLM in its response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique identifier for this tool call.
    pub id: String,
    /// Name of the tool being called.
    pub name: String,
    /// JSON-encoded arguments for the tool.
    pub arguments: String,
}

/// A request to an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Target model identifier.
    pub model: String,
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Sampling temperature (0.0 - 2.0).
    pub temperature: Option<f64>,
    /// Maximum tokens to generate.
    pub max_tokens: Option<u64>,
    /// Tools available for the LLM to call.
    pub tools: Option<Vec<Tool>>,
    /// Whether to stream the response.
    pub stream: bool,
}

/// A complete response from an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    /// Generated text content.
    pub content: String,
    /// Model that generated the response.
    pub model: String,
    /// Token usage statistics.
    pub usage: TokenUsage,
    /// Response latency in milliseconds.
    pub latency_ms: u64,
    /// Reason the model stopped generating.
    pub finish_reason: FinishReason,
    /// Tool calls made by the model, if any.
    pub tool_calls: Option<Vec<ToolCall>>,
}

/// Reason the model stopped generating tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinishReason {
    /// Natural stop (end of response).
    Stop,
    /// Hit max_tokens limit.
    Length,
    /// Model invoked a tool.
    ToolUse,
    /// Content was filtered by safety systems.
    ContentFilter,
}

/// A single chunk in a streaming response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChunk {
    /// Incremental text content.
    pub delta: String,
    /// Present only on the final chunk.
    pub finish_reason: Option<FinishReason>,
    /// Present only on the final chunk (total usage).
    pub usage: Option<TokenUsage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_model() -> ModelInfo {
        ModelInfo {
            id: "test-model".to_string(),
            provider: "test".to_string(),
            context_window: 100_000,
            cost_per_input_token: 3.0 / 1_000_000.0,
            cost_per_output_token: 15.0 / 1_000_000.0,
            cost_per_cached_input_token: Some(0.30 / 1_000_000.0),
            capabilities: vec![Capability::Chat],
            max_output_tokens: Some(4096),
        }
    }

    #[test]
    fn calculate_cost_basic() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };
        let model = sample_model();
        let cost = usage.calculate_cost(&model);
        let expected = 1000.0 * 3.0 / 1_000_000.0 + 500.0 * 15.0 / 1_000_000.0;
        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn calculate_cost_with_cached_tokens() {
        let usage = TokenUsage {
            input_tokens: 500,
            output_tokens: 200,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: Some(300),
        };
        let model = sample_model();
        let cost = usage.calculate_cost(&model);
        let expected =
            500.0 * 3.0 / 1_000_000.0 + 200.0 * 15.0 / 1_000_000.0 + 300.0 * 0.30 / 1_000_000.0;
        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn calculate_cost_zero_tokens() {
        let usage = TokenUsage::default();
        let model = sample_model();
        let cost = usage.calculate_cost(&model);
        assert!((cost).abs() < f64::EPSILON);
    }

    #[test]
    fn calculate_cost_no_cached_price_falls_back_to_input() {
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: Some(200),
        };
        let mut model = sample_model();
        model.cost_per_cached_input_token = None;
        let cost = usage.calculate_cost(&model);
        // cached tokens use input price as fallback
        let expected =
            100.0 * 3.0 / 1_000_000.0 + 50.0 * 15.0 / 1_000_000.0 + 200.0 * 3.0 / 1_000_000.0;
        assert!((cost - expected).abs() < 1e-12);
    }
}
