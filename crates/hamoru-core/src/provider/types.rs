//! Shared types for LLM provider abstraction.
//!
//! These types define the vocabulary for cross-layer communication.
//! Provider-specific API types must NOT appear here — only the unified abstractions.

use std::ops::AddAssign;

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
    /// A tool invocation by the assistant (ADR-007).
    ///
    /// Maps from OpenAI `assistant.tool_calls` and Anthropic `tool_use` content blocks.
    /// `input` is parsed JSON rather than a raw string so that provider adapters
    /// can work with structured data without repeated parsing.
    #[serde(rename = "tool_use")]
    ToolUse {
        /// Unique identifier for this tool call.
        id: String,
        /// Name of the tool being called.
        name: String,
        /// Parsed JSON arguments for the tool.
        input: serde_json::Value,
    },
    /// A tool execution result (ADR-007).
    ///
    /// Maps from OpenAI `role: "tool"` messages and Anthropic `tool_result` content blocks.
    #[serde(rename = "tool_result")]
    ToolResult {
        /// ID of the tool call this result corresponds to.
        tool_use_id: String,
        /// The tool's output content.
        content: String,
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

/// Merges two `Option<u64>` cache fields: `None + Some(x) → Some(x)`, `Some(a) + Some(b) → Some(a+b)`.
fn merge_option_u64(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x + y),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

impl AddAssign for TokenUsage {
    fn add_assign(&mut self, rhs: Self) {
        self.input_tokens += rhs.input_tokens;
        self.output_tokens += rhs.output_tokens;
        self.cache_creation_input_tokens = merge_option_u64(
            self.cache_creation_input_tokens,
            rhs.cache_creation_input_tokens,
        );
        self.cache_read_input_tokens =
            merge_option_u64(self.cache_read_input_tokens, rhs.cache_read_input_tokens);
    }
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

/// How the model should choose which tool to call.
///
/// Only meaningful when `tools` is `Some` in the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoice {
    /// Model decides whether to call a tool.
    Auto,
    /// Model must not call any tool. Tools are visible but suppressed.
    None,
    /// Model must call a tool (any tool from the provided list).
    Required,
    /// Model must call a specific tool by name.
    Tool {
        /// Name of the tool the model must call.
        name: String,
    },
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
    /// How the model should choose which tool to call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
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
    /// Complete tool calls (ADR-007: buffered, not incremental).
    ///
    /// Tool calls are accumulated by the provider adapter and emitted as
    /// complete objects on the final chunk. This avoids the fragmented
    /// argument streaming that has caused persistent bugs in other projects.
    pub tool_calls: Option<Vec<ToolCall>>,
}

/// Accumulates incremental tool call fragments from streaming responses
/// into complete `ToolCall` objects (ADR-007).
///
/// Provider adapters use this to buffer partial tool call data (name, argument
/// fragments) and emit complete tool calls only when the stream signals completion.
/// This shared utility prevents duplicating buffering logic across providers.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    /// In-progress tool calls keyed by index (provider stream order).
    pending: std::collections::BTreeMap<u32, PendingToolCall>,
}

/// A tool call being assembled from streaming fragments.
#[derive(Debug, Default)]
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    /// Creates a new empty accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a tool call fragment from a streaming chunk.
    ///
    /// `index` is the tool call's position in the array (provider-assigned).
    /// Call this for each incremental piece: first with id+name, then with
    /// argument fragments.
    pub fn feed(&mut self, index: u32, id: Option<&str>, name: Option<&str>, arguments: &str) {
        let entry = self.pending.entry(index).or_default();
        if let Some(id) = id {
            entry.id = id.to_string();
        }
        if let Some(name) = name {
            entry.name = name.to_string();
        }
        entry.arguments.push_str(arguments);
    }

    /// Returns `true` if any tool call fragments have been accumulated.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Consumes the accumulator and returns complete `ToolCall` objects.
    ///
    /// Call this when the stream signals completion (finish_reason received).
    pub fn finish(self) -> Vec<ToolCall> {
        self.pending
            .into_values()
            .map(|p| ToolCall {
                id: p.id,
                name: p.name,
                arguments: p.arguments,
            })
            .collect()
    }
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
    fn token_usage_add_assign_basic() {
        let mut a = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };
        let b = TokenUsage {
            input_tokens: 200,
            output_tokens: 100,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };
        a += b;
        assert_eq!(a.input_tokens, 300);
        assert_eq!(a.output_tokens, 150);
        assert!(a.cache_creation_input_tokens.is_none());
        assert!(a.cache_read_input_tokens.is_none());
    }

    #[test]
    fn token_usage_add_assign_merges_cache_fields() {
        let mut a = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_input_tokens: Some(10),
            cache_read_input_tokens: None,
        };
        let b = TokenUsage {
            input_tokens: 200,
            output_tokens: 100,
            cache_creation_input_tokens: Some(20),
            cache_read_input_tokens: Some(30),
        };
        a += b;
        assert_eq!(a.input_tokens, 300);
        assert_eq!(a.output_tokens, 150);
        assert_eq!(a.cache_creation_input_tokens, Some(30));
        assert_eq!(a.cache_read_input_tokens, Some(30));
    }

    #[test]
    fn token_usage_add_assign_default_plus_values() {
        let mut a = TokenUsage::default();
        let b = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_input_tokens: Some(10),
            cache_read_input_tokens: Some(20),
        };
        a += b;
        assert_eq!(a.input_tokens, 100);
        assert_eq!(a.output_tokens, 50);
        assert_eq!(a.cache_creation_input_tokens, Some(10));
        assert_eq!(a.cache_read_input_tokens, Some(20));
    }

    #[test]
    fn tool_choice_serialization() {
        let auto = serde_json::to_value(&ToolChoice::Auto).unwrap();
        assert_eq!(auto, serde_json::json!("auto"));

        let required = serde_json::to_value(&ToolChoice::Required).unwrap();
        assert_eq!(required, serde_json::json!("required"));

        let tool = serde_json::to_value(&ToolChoice::Tool {
            name: "report_status".to_string(),
        })
        .unwrap();
        assert_eq!(tool, serde_json::json!({"tool": {"name": "report_status"}}));
    }

    #[test]
    fn tool_choice_none_serialization() {
        let none = serde_json::to_value(&ToolChoice::None).unwrap();
        assert_eq!(none, serde_json::json!("none"));
    }

    #[test]
    fn tool_choice_deserialization_roundtrip() {
        let choices = vec![
            ToolChoice::Auto,
            ToolChoice::None,
            ToolChoice::Required,
            ToolChoice::Tool {
                name: "report_status".to_string(),
            },
        ];
        for choice in choices {
            let json = serde_json::to_string(&choice).unwrap();
            let parsed: ToolChoice = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, choice);
        }
    }

    #[test]
    fn chat_request_omits_none_tool_choice() {
        let request = ChatRequest {
            model: "test".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            tools: None,
            tool_choice: None,
            stream: false,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert!(!json.as_object().unwrap().contains_key("tool_choice"));
    }

    #[test]
    fn content_part_tool_use_serde_roundtrip() {
        let tool_use = ContentPart::ToolUse {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            input: serde_json::json!({"location": "Tokyo"}),
        };
        let json = serde_json::to_string(&tool_use).unwrap();
        let parsed: ContentPart = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, tool_use);
    }

    #[test]
    fn content_part_tool_result_serde_roundtrip() {
        let tool_result = ContentPart::ToolResult {
            tool_use_id: "call_123".to_string(),
            content: "Sunny, 25C".to_string(),
        };
        let json = serde_json::to_string(&tool_result).unwrap();
        let parsed: ContentPart = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, tool_result);
    }

    #[test]
    fn content_part_tool_use_has_correct_type_tag() {
        let tool_use = ContentPart::ToolUse {
            id: "call_1".to_string(),
            name: "search".to_string(),
            input: serde_json::json!({}),
        };
        let json = serde_json::to_value(&tool_use).unwrap();
        assert_eq!(json["type"], "tool_use");
        assert_eq!(json["id"], "call_1");
        assert_eq!(json["name"], "search");
    }

    #[test]
    fn content_part_tool_result_has_correct_type_tag() {
        let tool_result = ContentPart::ToolResult {
            tool_use_id: "call_1".to_string(),
            content: "result".to_string(),
        };
        let json = serde_json::to_value(&tool_result).unwrap();
        assert_eq!(json["type"], "tool_result");
        assert_eq!(json["tool_use_id"], "call_1");
    }

    #[test]
    fn chat_chunk_with_tool_calls() {
        let chunk = ChatChunk {
            delta: String::new(),
            finish_reason: Some(FinishReason::ToolUse),
            usage: None,
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: r#"{"q":"rust"}"#.to_string(),
            }]),
        };
        let json = serde_json::to_value(&chunk).unwrap();
        assert!(json["tool_calls"].is_array());
        assert_eq!(json["tool_calls"][0]["name"], "search");
    }

    #[test]
    fn chat_chunk_without_tool_calls() {
        let chunk = ChatChunk {
            delta: "hello".to_string(),
            finish_reason: None,
            usage: None,
            tool_calls: None,
        };
        let json = serde_json::to_value(&chunk).unwrap();
        assert!(json["tool_calls"].is_null());
    }

    #[test]
    fn tool_call_accumulator_single_tool() {
        let mut acc = ToolCallAccumulator::new();
        acc.feed(0, Some("call_1"), Some("search"), "");
        acc.feed(0, None, None, r#"{"q":"#);
        acc.feed(0, None, None, r#""rust"}"#);
        assert!(acc.has_pending());

        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[0].arguments, r#"{"q":"rust"}"#);
    }

    #[test]
    fn tool_call_accumulator_multiple_tools() {
        let mut acc = ToolCallAccumulator::new();
        acc.feed(0, Some("call_1"), Some("search"), r#"{"q":"a"}"#);
        acc.feed(1, Some("call_2"), Some("fetch"), r#"{"url":"b"}"#);

        let calls = acc.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[1].name, "fetch");
    }

    #[test]
    fn tool_call_accumulator_empty() {
        let acc = ToolCallAccumulator::new();
        assert!(!acc.has_pending());
        let calls = acc.finish();
        assert!(calls.is_empty());
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
