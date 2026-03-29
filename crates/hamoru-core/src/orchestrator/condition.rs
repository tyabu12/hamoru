//! Condition evaluation for workflow step transitions.
//!
//! Supports two modes: Tool Calling (v2, default) and STATUS line parsing (v1, fallback).
//! Tool Calling is preferred because it uses structured output via the `report_status` tool.
//! STATUS line parsing is kept as a fallback for models without tool support.

use crate::error::HamoruError;
use crate::provider::types::{ChatResponse, Tool};

use super::{ConditionMode, StepOutput, Transition, TransitionTarget};

/// Name of the status reporting tool injected into LLM requests.
pub const REPORT_STATUS_TOOL_NAME: &str = "report_status";

/// Number of lines from the end to scan for a STATUS line (v1).
const STATUS_SCAN_LINES: usize = 10;

/// Builds the `report_status` tool definition for a given set of valid status values.
///
/// The `valid_statuses` are the condition strings from the step's transitions,
/// encoded as a JSON Schema `enum` to constrain the LLM to valid values only.
pub fn build_report_status_tool(valid_statuses: &[&str]) -> Tool {
    let status_enum: Vec<serde_json::Value> = valid_statuses
        .iter()
        .map(|s| serde_json::Value::String(s.to_string()))
        .collect();

    Tool {
        name: REPORT_STATUS_TOOL_NAME.to_string(),
        description:
            "Report the status of your evaluation. You MUST call this tool to indicate your decision."
                .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": status_enum,
                    "description": "The evaluation status."
                },
                "reason": {
                    "type": "string",
                    "description": "A brief explanation for your decision."
                }
            },
            "required": ["status", "reason"]
        }),
    }
}

/// Evaluates the condition from a `ChatResponse` using the specified mode.
///
/// For `ToolCalling` mode: extracts status from a `report_status` tool call.
/// If the LLM did not call the tool, falls back to STATUS line parsing with a warning.
/// For `StatusLine` mode: parses STATUS lines from the response text.
pub fn evaluate_condition(
    response: &ChatResponse,
    mode: &ConditionMode,
    step_name: &str,
) -> Result<StepOutput, HamoruError> {
    match mode {
        ConditionMode::ToolCalling => {
            // Try tool call extraction first
            if let Some(output) = extract_status_from_tool_call(response) {
                return Ok(output);
            }
            // Fallback to STATUS line parsing
            tracing::warn!(
                step = step_name,
                "ToolCalling mode: LLM did not call report_status. \
                 Falling back to STATUS line parsing."
            );
            extract_status_from_text(&response.content).ok_or_else(|| {
                HamoruError::ConditionEvaluationFailed {
                    step: step_name.to_string(),
                    reason: "No status found. The model did not call report_status \
                             or include a STATUS line in its response. \
                             Check the step instruction or try condition_mode: status_line."
                        .to_string(),
                }
            })
        }
        ConditionMode::StatusLine => extract_status_from_text(&response.content).ok_or_else(|| {
            HamoruError::ConditionEvaluationFailed {
                step: step_name.to_string(),
                reason: "No STATUS line found in LLM response. \
                         The model should include a line like 'STATUS: approved' \
                         near the end of its output."
                    .to_string(),
            }
        }),
    }
}

/// Matches an extracted status against a step's transitions.
///
/// Case-insensitive match. Returns the target of the first matching transition.
pub fn match_transition<'a>(
    status: &str,
    transitions: &'a [Transition],
) -> Option<&'a TransitionTarget> {
    let normalized = status.to_lowercase();
    transitions
        .iter()
        .find(|t| t.condition.to_lowercase() == normalized)
        .map(|t| &t.next)
}

/// Extracts status from a `report_status` tool call in the response.
fn extract_status_from_tool_call(response: &ChatResponse) -> Option<StepOutput> {
    let tool_calls = response.tool_calls.as_ref()?;
    let call = tool_calls
        .iter()
        .find(|tc| tc.name == REPORT_STATUS_TOOL_NAME)?;

    let args: serde_json::Value = serde_json::from_str(&call.arguments).ok()?;
    let status = args.get("status")?.as_str()?;
    let reason = args
        .get("reason")
        .and_then(|r| r.as_str())
        .map(String::from);

    Some(StepOutput {
        full_content: response.content.clone(),
        status: normalize_status(status),
        content: response.content.clone(),
        reason,
    })
}

/// Extracts status from a STATUS line in the response text (v1 fallback).
///
/// Scans the last N lines in reverse order, looking for a `STATUS:` prefix.
fn extract_status_from_text(content: &str) -> Option<StepOutput> {
    let lines: Vec<&str> = content.lines().collect();
    let scan_start = lines.len().saturating_sub(STATUS_SCAN_LINES);

    for i in (scan_start..lines.len()).rev() {
        if let Some(status) = try_parse_status_line(lines[i]) {
            // Build content excluding the STATUS line
            let mut content_lines: Vec<&str> = Vec::with_capacity(lines.len());
            for (j, line) in lines.iter().enumerate() {
                if j != i {
                    content_lines.push(line);
                }
            }
            let body = content_lines.join("\n");

            return Some(StepOutput {
                full_content: content.to_string(),
                status,
                content: body.trim().to_string(),
                reason: None,
            });
        }
    }

    None
}

/// Attempts to parse a STATUS line from a single line of text.
///
/// Matches `STATUS:` prefix (case-insensitive) and extracts the value.
fn try_parse_status_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let upper = trimmed.to_uppercase();
    if !upper.starts_with("STATUS") {
        return None;
    }
    let after_status = &trimmed["STATUS".len()..].trim_start();
    if !after_status.starts_with(':') {
        return None;
    }
    let value = after_status[1..].trim();
    if value.is_empty() {
        return None;
    }
    Some(normalize_status(value))
}

/// Normalizes a status string: lowercase, trim, strip trailing punctuation.
fn normalize_status(raw: &str) -> String {
    let trimmed = raw.trim().to_lowercase();
    trimmed
        .trim_end_matches(['.', '!', ',', ';', '?'])
        .to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::provider::types::{ChatResponse, FinishReason, TokenUsage, ToolCall};

    // -----------------------------------------------------------------------
    // Test fixtures (pub(crate) for reuse in engine tests)
    // -----------------------------------------------------------------------

    /// Creates a ChatResponse with a report_status tool call.
    pub(crate) fn response_with_tool_status(status: &str, reason: &str) -> ChatResponse {
        ChatResponse {
            content: String::new(),
            model: "test-model".to_string(),
            usage: TokenUsage::default(),
            latency_ms: 100,
            finish_reason: FinishReason::ToolUse,
            tool_calls: Some(vec![ToolCall {
                id: "call_001".to_string(),
                name: REPORT_STATUS_TOOL_NAME.to_string(),
                arguments: serde_json::json!({
                    "status": status,
                    "reason": reason,
                })
                .to_string(),
            }]),
        }
    }

    /// Creates a ChatResponse with a STATUS line in the content.
    pub(crate) fn response_with_status_line(body: &str, status: &str) -> ChatResponse {
        let content = format!("{body}\nSTATUS: {status}");
        ChatResponse {
            content,
            model: "test-model".to_string(),
            usage: TokenUsage::default(),
            latency_ms: 100,
            finish_reason: FinishReason::Stop,
            tool_calls: None,
        }
    }

    /// Creates a simple ChatResponse with text content only.
    pub(crate) fn simple_response(content: &str) -> ChatResponse {
        ChatResponse {
            content: content.to_string(),
            model: "test-model".to_string(),
            usage: TokenUsage::default(),
            latency_ms: 100,
            finish_reason: FinishReason::Stop,
            tool_calls: None,
        }
    }

    // -----------------------------------------------------------------------
    // build_report_status_tool tests
    // -----------------------------------------------------------------------

    #[test]
    fn tool_has_correct_name_and_schema() {
        let tool = build_report_status_tool(&["approved", "improve"]);
        assert_eq!(tool.name, REPORT_STATUS_TOOL_NAME);
        let props = &tool.parameters["properties"];
        let status_enum = props["status"]["enum"].as_array().unwrap();
        assert_eq!(status_enum.len(), 2);
        assert_eq!(status_enum[0], "approved");
        assert_eq!(status_enum[1], "improve");
        let required = tool.parameters["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("status")));
        assert!(required.contains(&serde_json::json!("reason")));
    }

    // -----------------------------------------------------------------------
    // v2: Tool call extraction tests
    // -----------------------------------------------------------------------

    #[test]
    fn tool_call_happy_path() {
        let response = response_with_tool_status("approved", "Looks good");
        let output = extract_status_from_tool_call(&response).unwrap();
        assert_eq!(output.status, "approved");
        assert_eq!(output.reason.as_deref(), Some("Looks good"));
    }

    #[test]
    fn tool_call_normalizes_status() {
        let response = response_with_tool_status("APPROVED", "ok");
        let output = extract_status_from_tool_call(&response).unwrap();
        assert_eq!(output.status, "approved");
    }

    #[test]
    fn tool_call_no_tool_calls() {
        let response = simple_response("No tools here");
        assert!(extract_status_from_tool_call(&response).is_none());
    }

    #[test]
    fn tool_call_wrong_tool_name() {
        let response = ChatResponse {
            content: String::new(),
            model: "test".to_string(),
            usage: TokenUsage::default(),
            latency_ms: 100,
            finish_reason: FinishReason::ToolUse,
            tool_calls: Some(vec![ToolCall {
                id: "call_001".to_string(),
                name: "other_tool".to_string(),
                arguments: r#"{"status":"approved","reason":"ok"}"#.to_string(),
            }]),
        };
        assert!(extract_status_from_tool_call(&response).is_none());
    }

    #[test]
    fn tool_call_malformed_json() {
        let response = ChatResponse {
            content: String::new(),
            model: "test".to_string(),
            usage: TokenUsage::default(),
            latency_ms: 100,
            finish_reason: FinishReason::ToolUse,
            tool_calls: Some(vec![ToolCall {
                id: "call_001".to_string(),
                name: REPORT_STATUS_TOOL_NAME.to_string(),
                arguments: "not json".to_string(),
            }]),
        };
        assert!(extract_status_from_tool_call(&response).is_none());
    }

    #[test]
    fn tool_call_missing_status_field() {
        let response = ChatResponse {
            content: String::new(),
            model: "test".to_string(),
            usage: TokenUsage::default(),
            latency_ms: 100,
            finish_reason: FinishReason::ToolUse,
            tool_calls: Some(vec![ToolCall {
                id: "call_001".to_string(),
                name: REPORT_STATUS_TOOL_NAME.to_string(),
                arguments: r#"{"result":"approved","reason":"ok"}"#.to_string(),
            }]),
        };
        assert!(extract_status_from_tool_call(&response).is_none());
    }

    #[test]
    fn tool_call_multiple_tools_extracts_correct_one() {
        let response = ChatResponse {
            content: String::new(),
            model: "test".to_string(),
            usage: TokenUsage::default(),
            latency_ms: 100,
            finish_reason: FinishReason::ToolUse,
            tool_calls: Some(vec![
                ToolCall {
                    id: "call_001".to_string(),
                    name: "other_tool".to_string(),
                    arguments: r#"{"data":"something"}"#.to_string(),
                },
                ToolCall {
                    id: "call_002".to_string(),
                    name: REPORT_STATUS_TOOL_NAME.to_string(),
                    arguments: r#"{"status":"improve","reason":"needs work"}"#.to_string(),
                },
            ]),
        };
        let output = extract_status_from_tool_call(&response).unwrap();
        assert_eq!(output.status, "improve");
    }

    #[test]
    fn tool_call_wrong_schema() {
        let response = ChatResponse {
            content: String::new(),
            model: "test".to_string(),
            usage: TokenUsage::default(),
            latency_ms: 100,
            finish_reason: FinishReason::ToolUse,
            tool_calls: Some(vec![ToolCall {
                id: "call_001".to_string(),
                name: REPORT_STATUS_TOOL_NAME.to_string(),
                arguments: r#"{"result":"approved"}"#.to_string(),
            }]),
        };
        assert!(extract_status_from_tool_call(&response).is_none());
    }

    // -----------------------------------------------------------------------
    // v1: STATUS line parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn status_line_standard() {
        let output = extract_status_from_text("Some output\nSTATUS: approved").unwrap();
        assert_eq!(output.status, "approved");
        assert_eq!(output.content, "Some output");
    }

    #[test]
    fn status_line_no_space() {
        let output = extract_status_from_text("body\nSTATUS:approved").unwrap();
        assert_eq!(output.status, "approved");
    }

    #[test]
    fn status_line_extra_space() {
        let output = extract_status_from_text("body\nSTATUS :  approved  ").unwrap();
        assert_eq!(output.status, "approved");
    }

    #[test]
    fn status_line_case_variations() {
        let output = extract_status_from_text("body\nStatus: APPROVED").unwrap();
        assert_eq!(output.status, "approved");

        let output = extract_status_from_text("body\nstatus: Improve").unwrap();
        assert_eq!(output.status, "improve");
    }

    #[test]
    fn status_line_trailing_punctuation() {
        let output = extract_status_from_text("body\nSTATUS: approved.").unwrap();
        assert_eq!(output.status, "approved");

        let output = extract_status_from_text("body\nSTATUS: done!").unwrap();
        assert_eq!(output.status, "done");
    }

    #[test]
    fn status_line_multiple_last_wins() {
        let output =
            extract_status_from_text("body\nSTATUS: improve\nmore text\nSTATUS: approved").unwrap();
        assert_eq!(output.status, "approved");
    }

    #[test]
    fn status_line_not_found() {
        assert!(extract_status_from_text("just some text\nno status here").is_none());
    }

    #[test]
    fn status_line_content_excludes_status() {
        let output = extract_status_from_text("line 1\nline 2\nSTATUS: done").unwrap();
        assert_eq!(output.content, "line 1\nline 2");
    }

    #[test]
    fn status_line_empty_content() {
        assert!(extract_status_from_text("").is_none());
    }

    #[test]
    fn status_line_beyond_scan_window() {
        // Put STATUS line at the very beginning, followed by >10 lines
        let mut text = "STATUS: approved\n".to_string();
        for i in 0..15 {
            text.push_str(&format!("line {i}\n"));
        }
        // STATUS line is beyond the last 10 lines scan window
        assert!(extract_status_from_text(&text).is_none());
    }

    // -----------------------------------------------------------------------
    // Unified evaluate_condition tests
    // -----------------------------------------------------------------------

    #[test]
    fn evaluate_tool_calling_dispatch() {
        let response = response_with_tool_status("approved", "ok");
        let output =
            evaluate_condition(&response, &ConditionMode::ToolCalling, "test_step").unwrap();
        assert_eq!(output.status, "approved");
    }

    #[test]
    fn evaluate_status_line_dispatch() {
        let response = response_with_status_line("body", "done");
        let output =
            evaluate_condition(&response, &ConditionMode::StatusLine, "test_step").unwrap();
        assert_eq!(output.status, "done");
    }

    #[test]
    fn evaluate_tool_calling_fallback_to_status_line() {
        // Response has no tool calls but has a STATUS line
        let response = response_with_status_line("review complete", "approved");
        let output =
            evaluate_condition(&response, &ConditionMode::ToolCalling, "test_step").unwrap();
        assert_eq!(output.status, "approved");
    }

    #[test]
    fn evaluate_both_fail_returns_error() {
        let response = simple_response("no status anywhere");
        let err =
            evaluate_condition(&response, &ConditionMode::ToolCalling, "test_step").unwrap_err();
        assert!(err.to_string().contains("No status found"));
    }

    // -----------------------------------------------------------------------
    // Transition matching tests
    // -----------------------------------------------------------------------

    #[test]
    fn match_transition_exact() {
        let transitions = vec![
            Transition {
                condition: "approved".to_string(),
                next: TransitionTarget::Complete,
            },
            Transition {
                condition: "improve".to_string(),
                next: TransitionTarget::Step("generate".to_string()),
            },
        ];
        assert_eq!(
            match_transition("approved", &transitions),
            Some(&TransitionTarget::Complete)
        );
        assert_eq!(
            match_transition("improve", &transitions),
            Some(&TransitionTarget::Step("generate".to_string()))
        );
    }

    #[test]
    fn match_transition_case_insensitive() {
        let transitions = vec![Transition {
            condition: "Approved".to_string(),
            next: TransitionTarget::Complete,
        }];
        assert_eq!(
            match_transition("approved", &transitions),
            Some(&TransitionTarget::Complete)
        );
    }

    #[test]
    fn match_transition_no_match() {
        let transitions = vec![Transition {
            condition: "approved".to_string(),
            next: TransitionTarget::Complete,
        }];
        assert!(match_transition("unknown", &transitions).is_none());
    }

    #[test]
    fn match_transition_complete_target() {
        let transitions = vec![Transition {
            condition: "done".to_string(),
            next: TransitionTarget::Complete,
        }];
        let target = match_transition("done", &transitions).unwrap();
        assert_eq!(target, &TransitionTarget::Complete);
    }
}
