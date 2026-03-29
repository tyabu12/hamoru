//! OpenAI-compatible wire format types.
//!
//! These types define the JSON shapes that external clients send and receive.
//! They are intentionally separate from internal types (`ChatRequest`, `ChatResponse`)
//! to keep the translation explicit and provider-specific formats isolated.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// OpenAI-compatible chat completion request body.
#[derive(Debug, Clone, Deserialize)]
pub struct OaiChatRequest {
    /// Model identifier (e.g., "hamoru:cost-optimized", "claude:claude-sonnet-4-6").
    pub model: String,
    /// Conversation messages.
    pub messages: Vec<OaiMessage>,
    /// Sampling temperature.
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Maximum tokens to generate.
    #[serde(default)]
    pub max_tokens: Option<u64>,
    /// Whether to stream the response as SSE.
    #[serde(default)]
    pub stream: bool,
    /// Tools available for the model.
    #[serde(default)]
    pub tools: Option<Vec<OaiTool>>,
    /// How the model should choose which tool to call.
    #[serde(default)]
    pub tool_choice: Option<OaiToolChoice>,
}

/// A message in the OpenAI wire format.
#[derive(Debug, Clone, Deserialize)]
pub struct OaiMessage {
    /// Message role: "system", "user", "assistant", or "tool".
    pub role: String,
    /// Text content (may be absent for assistant messages with tool_calls).
    #[serde(default)]
    pub content: Option<String>,
    /// Tool calls made by the assistant (OpenAI format).
    #[serde(default)]
    pub tool_calls: Option<Vec<OaiToolCall>>,
    /// For tool result messages: the ID of the tool call this responds to.
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

/// A tool call in the OpenAI wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OaiToolCall {
    /// Unique identifier for this tool call.
    pub id: String,
    /// Always "function" in OpenAI format.
    #[serde(rename = "type", default = "default_function_type")]
    pub call_type: String,
    /// Function details.
    pub function: OaiFunction,
}

fn default_function_type() -> String {
    "function".to_string()
}

/// Function details within a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OaiFunction {
    /// Function name.
    pub name: String,
    /// JSON-encoded arguments.
    pub arguments: String,
}

/// A tool definition in the OpenAI wire format.
#[derive(Debug, Clone, Deserialize)]
pub struct OaiTool {
    /// Always "function".
    #[serde(rename = "type")]
    pub tool_type: String,
    /// Function definition.
    pub function: OaiToolFunction,
}

/// Function definition within a tool.
#[derive(Debug, Clone, Deserialize)]
pub struct OaiToolFunction {
    /// Function name.
    pub name: String,
    /// Description (optional per OpenAI spec).
    #[serde(default)]
    pub description: Option<String>,
    /// JSON Schema for parameters.
    #[serde(default)]
    pub parameters: serde_json::Value,
}

/// Tool choice in the OpenAI wire format.
///
/// Can be a string ("auto", "required", "none") or an object specifying a function.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OaiToolChoice {
    /// String mode: "auto", "required", or "none".
    Mode(String),
    /// Specific function choice.
    Function {
        /// Always "function".
        #[serde(rename = "type")]
        choice_type: String,
        /// Function to call.
        function: OaiToolChoiceFunction,
    },
}

/// Function specification in tool_choice.
#[derive(Debug, Clone, Deserialize)]
pub struct OaiToolChoiceFunction {
    /// Name of the function to call.
    pub name: String,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// OpenAI-compatible chat completion response.
#[derive(Debug, Clone, Serialize)]
pub struct OaiChatResponse {
    /// Response ID (e.g., "chatcmpl-xxx").
    pub id: String,
    /// Always "chat.completion".
    pub object: String,
    /// Unix timestamp of creation.
    pub created: i64,
    /// Model used.
    pub model: String,
    /// Response choices (always exactly one for hamoru).
    pub choices: Vec<OaiChoice>,
    /// Token usage statistics.
    pub usage: OaiUsage,
}

/// A choice in the response.
#[derive(Debug, Clone, Serialize)]
pub struct OaiChoice {
    /// Choice index (always 0 for hamoru).
    pub index: u32,
    /// The assistant's message.
    pub message: OaiResponseMessage,
    /// Reason for stopping.
    pub finish_reason: String,
}

/// Assistant message in a response.
#[derive(Debug, Clone, Serialize)]
pub struct OaiResponseMessage {
    /// Always "assistant".
    pub role: String,
    /// Text content (may be null if only tool_calls).
    pub content: Option<String>,
    /// Tool calls, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OaiToolCall>>,
}

/// Token usage in response.
#[derive(Debug, Clone, Serialize)]
pub struct OaiUsage {
    /// Tokens in the prompt.
    pub prompt_tokens: u64,
    /// Tokens in the completion.
    pub completion_tokens: u64,
    /// Total tokens.
    pub total_tokens: u64,
}

// ---------------------------------------------------------------------------
// Streaming types
// ---------------------------------------------------------------------------

/// A streaming chunk in the OpenAI SSE format.
#[derive(Debug, Clone, Serialize)]
pub struct OaiChatChunk {
    /// Response ID (same across all chunks in a stream).
    pub id: String,
    /// Always "chat.completion.chunk".
    pub object: String,
    /// Unix timestamp.
    pub created: i64,
    /// Model used.
    pub model: String,
    /// Chunk choices.
    pub choices: Vec<OaiChunkChoice>,
    /// Usage (present only on final chunk).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<OaiUsage>,
    /// hamoru extension field for progress events (ADR-007 L1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hamoru: Option<OaiHamoruExtension>,
}

/// A choice within a streaming chunk.
#[derive(Debug, Clone, Serialize)]
pub struct OaiChunkChoice {
    /// Choice index (always 0).
    pub index: u32,
    /// Delta content.
    pub delta: OaiChunkDelta,
    /// Finish reason (present only on final chunk).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// Delta content in a streaming chunk.
#[derive(Debug, Clone, Serialize)]
pub struct OaiChunkDelta {
    /// Role (present only on first chunk).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Incremental text content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Complete tool calls (ADR-007: buffered, not incremental).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OaiToolCall>>,
}

/// hamoru extension field for SSE progress events (ADR-007 L1).
#[derive(Debug, Clone, Serialize)]
pub struct OaiHamoruExtension {
    /// Event type: "step_start", "step_complete".
    #[serde(rename = "type")]
    pub event_type: String,
    /// Step name (e.g., "reviewer").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    /// Current iteration number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    /// Model being used for this step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Cumulative cost so far.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_so_far: Option<f64>,
    /// Cumulative tokens so far.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_so_far: Option<u64>,
}

// ---------------------------------------------------------------------------
// Error response type
// ---------------------------------------------------------------------------

/// OpenAI-compatible error response body.
#[derive(Debug, Clone, Serialize)]
pub struct OaiErrorResponse {
    /// Error details.
    pub error: OaiErrorBody,
}

/// Error details within an error response.
#[derive(Debug, Clone, Serialize)]
pub struct OaiErrorBody {
    /// Human-readable error message.
    pub message: String,
    /// Error type (e.g., "invalid_request_error", "not_found_error").
    #[serde(rename = "type")]
    pub error_type: String,
    /// Error code (optional, e.g., "model_not_found").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oai_chat_response_serializes_correctly() {
        let resp = OaiChatResponse {
            id: "chatcmpl-123".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "hamoru:cost-optimized".to_string(),
            choices: vec![OaiChoice {
                index: 0,
                message: OaiResponseMessage {
                    role: "assistant".to_string(),
                    content: Some("Hello!".to_string()),
                    tool_calls: None,
                },
                finish_reason: "stop".to_string(),
            }],
            usage: OaiUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["object"], "chat.completion");
        assert_eq!(json["choices"][0]["finish_reason"], "stop");
        assert!(json["choices"][0]["message"].get("tool_calls").is_none());
    }

    #[test]
    fn oai_response_with_tool_calls() {
        let resp = OaiChatResponse {
            id: "chatcmpl-456".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "hamoru:cost-optimized".to_string(),
            choices: vec![OaiChoice {
                index: 0,
                message: OaiResponseMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_calls: Some(vec![OaiToolCall {
                        id: "call_1".to_string(),
                        call_type: "function".to_string(),
                        function: OaiFunction {
                            name: "get_weather".to_string(),
                            arguments: r#"{"location":"Tokyo"}"#.to_string(),
                        },
                    }]),
                },
                finish_reason: "tool_calls".to_string(),
            }],
            usage: OaiUsage {
                prompt_tokens: 20,
                completion_tokens: 10,
                total_tokens: 30,
            },
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(
            json["choices"][0]["message"]["tool_calls"][0]["type"],
            "function"
        );
        assert_eq!(
            json["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "get_weather"
        );
    }

    #[test]
    fn oai_chunk_skips_none_fields() {
        let chunk = OaiChatChunk {
            id: "chatcmpl-123".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 1234567890,
            model: "test".to_string(),
            choices: vec![OaiChunkChoice {
                index: 0,
                delta: OaiChunkDelta {
                    role: None,
                    content: Some("hi".to_string()),
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
            hamoru: None,
        };
        let json = serde_json::to_value(&chunk).unwrap();
        assert!(json.get("usage").is_none());
        assert!(json.get("hamoru").is_none());
        assert!(json["choices"][0].get("finish_reason").is_none());
        assert!(json["choices"][0]["delta"].get("role").is_none());
        assert!(json["choices"][0]["delta"].get("tool_calls").is_none());
    }

    #[test]
    fn oai_error_response_shape() {
        let err = OaiErrorResponse {
            error: OaiErrorBody {
                message: "Model not found".to_string(),
                error_type: "not_found_error".to_string(),
                code: Some("model_not_found".to_string()),
            },
        };
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["error"]["type"], "not_found_error");
        assert_eq!(json["error"]["code"], "model_not_found");
    }

    #[test]
    fn oai_l1_progress_event_shape() {
        let chunk = OaiChatChunk {
            id: "chatcmpl-123".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 1234567890,
            model: "hamoru:workflow:gen-review".to_string(),
            choices: vec![],
            usage: None,
            hamoru: Some(OaiHamoruExtension {
                event_type: "step_start".to_string(),
                step: Some("reviewer".to_string()),
                iteration: Some(2),
                model: Some("claude:claude-sonnet-4-6".to_string()),
                cost_so_far: Some(0.031),
                tokens_so_far: Some(2847),
            }),
        };
        let json = serde_json::to_value(&chunk).unwrap();
        assert!(json["choices"].as_array().unwrap().is_empty());
        assert_eq!(json["hamoru"]["type"], "step_start");
        assert_eq!(json["hamoru"]["step"], "reviewer");
    }

    #[test]
    fn oai_request_deserializes_minimal() {
        let json = r#"{
            "model": "hamoru:cost-optimized",
            "messages": [{"role": "user", "content": "Hello"}]
        }"#;
        let req: OaiChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model, "hamoru:cost-optimized");
        assert_eq!(req.messages.len(), 1);
        assert!(!req.stream);
        assert!(req.tools.is_none());
        assert!(req.tool_choice.is_none());
    }

    #[test]
    fn oai_request_deserializes_with_tools() {
        let json = r#"{
            "model": "test",
            "messages": [{"role": "user", "content": "Search for rust"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "search",
                    "description": "Search the web",
                    "parameters": {"type": "object", "properties": {"q": {"type": "string"}}}
                }
            }],
            "tool_choice": "auto"
        }"#;
        let req: OaiChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.tools.as_ref().unwrap().len(), 1);
        assert!(matches!(req.tool_choice, Some(OaiToolChoice::Mode(ref m)) if m == "auto"));
    }

    #[test]
    fn oai_request_deserializes_tool_result_message() {
        let json = r#"{
            "model": "test",
            "messages": [
                {"role": "user", "content": "What's the weather?"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_1", "content": "Sunny, 25C"}
            ]
        }"#;
        let req: OaiChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.messages.len(), 3);
        assert_eq!(req.messages[1].tool_calls.as_ref().unwrap().len(), 1);
        assert_eq!(req.messages[2].tool_call_id.as_ref().unwrap(), "call_1");
    }
}
