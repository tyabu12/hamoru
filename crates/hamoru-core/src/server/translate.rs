//! Translation between OpenAI wire format and internal types.
//!
//! Pure functions with no framework dependency. The translation follows
//! ADR-007 Phase 5 mapping tables.

use crate::error::HamoruError;
use crate::provider::types::{
    ChatResponse, ContentPart, FinishReason, Message, MessageContent, Role, Tool, ToolCall,
    ToolChoice,
};

use super::types::{OaiFunction, OaiMessage, OaiToolCall, OaiToolChoice};

// ---------------------------------------------------------------------------
// Inbound: OpenAI wire format → internal types
// ---------------------------------------------------------------------------

/// Translate an OpenAI message into an internal `Message`.
///
/// Handles the three message shapes:
/// - Regular: `{role, content}` → `Message { role, Text(content) }`
/// - Assistant with tool_calls: → `Message { role, Parts([Text?, ToolUse...]) }`
/// - Tool result: `{role: "tool", tool_call_id, content}` → `Message { role: Tool, Parts([ToolResult]) }`
pub fn oai_message_to_internal(msg: &OaiMessage) -> Result<Message, HamoruError> {
    let role = parse_role(&msg.role)?;

    // Tool result message
    if role == Role::Tool {
        let tool_call_id = msg.tool_call_id.as_deref().unwrap_or_default();
        let content = msg.content.clone().unwrap_or_default();
        return Ok(Message {
            role,
            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                tool_use_id: tool_call_id.to_string(),
                content,
            }]),
        });
    }

    // Assistant message with tool_calls
    if let Some(ref tool_calls) = msg.tool_calls {
        let mut parts = Vec::new();
        // Include text content if present
        if let Some(ref text) = msg.content
            && !text.is_empty()
        {
            parts.push(ContentPart::Text { text: text.clone() });
        }
        // Convert tool_calls to ToolUse content parts
        for tc in tool_calls {
            let input: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).map_err(|e| {
                    HamoruError::ConfigError {
                        reason: format!(
                            "Invalid JSON in tool_call arguments for '{}': {}",
                            tc.function.name, e
                        ),
                    }
                })?;
            parts.push(ContentPart::ToolUse {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                input,
            });
        }
        return Ok(Message {
            role,
            content: MessageContent::Parts(parts),
        });
    }

    // Regular message
    let text = msg.content.clone().unwrap_or_default();
    Ok(Message {
        role,
        content: MessageContent::Text(text),
    })
}

/// Translate OpenAI tool definitions to internal `Tool` types.
pub fn oai_tools_to_internal(tools: &[super::types::OaiTool]) -> Vec<Tool> {
    tools
        .iter()
        .map(|t| Tool {
            name: t.function.name.clone(),
            description: t.function.description.clone(),
            parameters: t.function.parameters.clone(),
        })
        .collect()
}

/// Translate OpenAI tool_choice to internal `ToolChoice`.
pub fn oai_tool_choice_to_internal(choice: &OaiToolChoice) -> Result<ToolChoice, HamoruError> {
    match choice {
        OaiToolChoice::Mode(mode) => match mode.as_str() {
            "auto" => Ok(ToolChoice::Auto),
            "required" => Ok(ToolChoice::Required),
            "none" => Ok(ToolChoice::Auto), // "none" maps to auto (no forced choice)
            other => Err(HamoruError::ConfigError {
                reason: format!("Unknown tool_choice mode: '{other}'"),
            }),
        },
        OaiToolChoice::Function { function, .. } => Ok(ToolChoice::Tool {
            name: function.name.clone(),
        }),
    }
}

fn parse_role(role: &str) -> Result<Role, HamoruError> {
    match role {
        "system" => Ok(Role::System),
        "user" => Ok(Role::User),
        "assistant" => Ok(Role::Assistant),
        "tool" => Ok(Role::Tool),
        other => Err(HamoruError::ConfigError {
            reason: format!("Unknown message role: '{other}'"),
        }),
    }
}

// ---------------------------------------------------------------------------
// Outbound: internal types → OpenAI wire format
// ---------------------------------------------------------------------------

/// Map internal `FinishReason` to the OpenAI string representation.
pub fn finish_reason_to_oai(reason: &FinishReason) -> &'static str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
        FinishReason::ToolUse => "tool_calls",
        FinishReason::ContentFilter => "content_filter",
    }
}

/// Convert internal `ToolCall` objects to OpenAI wire format.
pub fn tool_calls_to_oai(calls: &[ToolCall]) -> Vec<OaiToolCall> {
    calls
        .iter()
        .map(|tc| OaiToolCall {
            id: tc.id.clone(),
            call_type: "function".to_string(),
            function: OaiFunction {
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            },
        })
        .collect()
}

/// Build the response content and tool_calls from a `ChatResponse`.
///
/// Per ADR-007: `ChatResponse` uses flat `content + tool_calls` fields
/// that map 1:1 to OpenAI response fields with no conversion needed.
pub fn chat_response_to_oai_parts(
    resp: &ChatResponse,
) -> (Option<String>, Option<Vec<OaiToolCall>>) {
    let content = if resp.content.is_empty() {
        None
    } else {
        Some(resp.content.clone())
    };
    let tool_calls = resp.tool_calls.as_ref().map(|tcs| tool_calls_to_oai(tcs));
    (content, tool_calls)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_simple_user_message() {
        let oai = OaiMessage {
            role: "user".to_string(),
            content: Some("Hello".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };
        let msg = oai_message_to_internal(&oai).unwrap();
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, MessageContent::Text("Hello".to_string()));
    }

    #[test]
    fn translate_system_message() {
        let oai = OaiMessage {
            role: "system".to_string(),
            content: Some("You are helpful.".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };
        let msg = oai_message_to_internal(&oai).unwrap();
        assert_eq!(msg.role, Role::System);
    }

    #[test]
    fn translate_assistant_with_tool_calls() {
        let oai = OaiMessage {
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
            tool_call_id: None,
        };
        let msg = oai_message_to_internal(&oai).unwrap();
        assert_eq!(msg.role, Role::Assistant);
        match &msg.content {
            MessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 1);
                match &parts[0] {
                    ContentPart::ToolUse { id, name, input } => {
                        assert_eq!(id, "call_1");
                        assert_eq!(name, "get_weather");
                        assert_eq!(input["location"], "Tokyo");
                    }
                    other => panic!("Expected ToolUse, got {other:?}"),
                }
            }
            other => panic!("Expected Parts, got {other:?}"),
        }
    }

    #[test]
    fn translate_assistant_with_text_and_tool_calls() {
        let oai = OaiMessage {
            role: "assistant".to_string(),
            content: Some("Let me check.".to_string()),
            tool_calls: Some(vec![OaiToolCall {
                id: "call_1".to_string(),
                call_type: "function".to_string(),
                function: OaiFunction {
                    name: "search".to_string(),
                    arguments: r#"{"q":"rust"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
        };
        let msg = oai_message_to_internal(&oai).unwrap();
        match &msg.content {
            MessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(&parts[0], ContentPart::Text { text } if text == "Let me check."));
                assert!(matches!(&parts[1], ContentPart::ToolUse { name, .. } if name == "search"));
            }
            other => panic!("Expected Parts, got {other:?}"),
        }
    }

    #[test]
    fn translate_tool_result_message() {
        let oai = OaiMessage {
            role: "tool".to_string(),
            content: Some("Sunny, 25C".to_string()),
            tool_calls: None,
            tool_call_id: Some("call_1".to_string()),
        };
        let msg = oai_message_to_internal(&oai).unwrap();
        assert_eq!(msg.role, Role::Tool);
        match &msg.content {
            MessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 1);
                match &parts[0] {
                    ContentPart::ToolResult {
                        tool_use_id,
                        content,
                    } => {
                        assert_eq!(tool_use_id, "call_1");
                        assert_eq!(content, "Sunny, 25C");
                    }
                    other => panic!("Expected ToolResult, got {other:?}"),
                }
            }
            other => panic!("Expected Parts, got {other:?}"),
        }
    }

    #[test]
    fn translate_invalid_role_errors() {
        let oai = OaiMessage {
            role: "unknown".to_string(),
            content: Some("test".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };
        assert!(oai_message_to_internal(&oai).is_err());
    }

    #[test]
    fn translate_invalid_tool_call_arguments_errors() {
        let oai = OaiMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![OaiToolCall {
                id: "call_1".to_string(),
                call_type: "function".to_string(),
                function: OaiFunction {
                    name: "test".to_string(),
                    arguments: "not valid json".to_string(),
                },
            }]),
            tool_call_id: None,
        };
        assert!(oai_message_to_internal(&oai).is_err());
    }

    #[test]
    fn finish_reason_mapping() {
        assert_eq!(finish_reason_to_oai(&FinishReason::Stop), "stop");
        assert_eq!(finish_reason_to_oai(&FinishReason::Length), "length");
        assert_eq!(finish_reason_to_oai(&FinishReason::ToolUse), "tool_calls");
        assert_eq!(
            finish_reason_to_oai(&FinishReason::ContentFilter),
            "content_filter"
        );
    }

    #[test]
    fn tool_calls_to_oai_format() {
        let calls = vec![ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: r#"{"q":"rust"}"#.to_string(),
        }];
        let oai = tool_calls_to_oai(&calls);
        assert_eq!(oai.len(), 1);
        assert_eq!(oai[0].call_type, "function");
        assert_eq!(oai[0].function.name, "search");
    }

    #[test]
    fn chat_response_to_oai_text_only() {
        let resp = ChatResponse {
            content: "Hello!".to_string(),
            model: "test".to_string(),
            usage: Default::default(),
            latency_ms: 100,
            finish_reason: FinishReason::Stop,
            tool_calls: None,
        };
        let (content, tool_calls) = chat_response_to_oai_parts(&resp);
        assert_eq!(content.unwrap(), "Hello!");
        assert!(tool_calls.is_none());
    }

    #[test]
    fn chat_response_to_oai_tool_calls_only() {
        let resp = ChatResponse {
            content: String::new(),
            model: "test".to_string(),
            usage: Default::default(),
            latency_ms: 100,
            finish_reason: FinishReason::ToolUse,
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: "{}".to_string(),
            }]),
        };
        let (content, tool_calls) = chat_response_to_oai_parts(&resp);
        assert!(content.is_none());
        assert_eq!(tool_calls.unwrap().len(), 1);
    }

    #[test]
    fn tool_choice_auto() {
        let choice = OaiToolChoice::Mode("auto".to_string());
        assert_eq!(
            oai_tool_choice_to_internal(&choice).unwrap(),
            ToolChoice::Auto
        );
    }

    #[test]
    fn tool_choice_required() {
        let choice = OaiToolChoice::Mode("required".to_string());
        assert_eq!(
            oai_tool_choice_to_internal(&choice).unwrap(),
            ToolChoice::Required
        );
    }

    #[test]
    fn tool_choice_specific_function() {
        let choice = OaiToolChoice::Function {
            choice_type: "function".to_string(),
            function: super::super::types::OaiToolChoiceFunction {
                name: "search".to_string(),
            },
        };
        assert_eq!(
            oai_tool_choice_to_internal(&choice).unwrap(),
            ToolChoice::Tool {
                name: "search".to_string()
            }
        );
    }

    #[test]
    fn tool_choice_unknown_mode_errors() {
        let choice = OaiToolChoice::Mode("invalid".to_string());
        assert!(oai_tool_choice_to_internal(&choice).is_err());
    }

    #[test]
    fn oai_tools_to_internal_conversion() {
        let oai_tools = vec![super::super::types::OaiTool {
            tool_type: "function".to_string(),
            function: super::super::types::OaiToolFunction {
                name: "search".to_string(),
                description: "Search the web".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            },
        }];
        let tools = oai_tools_to_internal(&oai_tools);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "search");
        assert_eq!(tools[0].description, "Search the web");
    }
}
