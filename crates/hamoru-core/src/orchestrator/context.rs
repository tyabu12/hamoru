//! Context management for workflow step execution.
//!
//! Handles template substitution (`{task}`, `{previous_output}`) and
//! `ContextPolicy` application for message history management.

use crate::provider::types::{Message, MessageContent, Role};

use super::ContextPolicy;

/// Builds the message array for a workflow step execution.
///
/// - Replaces `{task}` in the instruction with the actual task text.
/// - **Security (Hard Rule 4)**: `{previous_output}` is stripped from the instruction
///   and injected as a separate User role message to prevent injection attacks.
/// - Emits `tracing::warn!` if `previous_output` contains system-instruction-like patterns.
pub fn build_step_messages(
    instruction: &str,
    task: &str,
    previous_output: Option<&str>,
) -> Vec<Message> {
    // Replace {task} placeholder
    let processed = instruction.replace("{task}", task);

    // Strip {previous_output} from instruction — it will be a separate User message
    let processed = processed.replace("{previous_output}", "");
    let processed = processed.trim().to_string();

    let mut messages = vec![Message {
        role: Role::System,
        content: MessageContent::Text(processed),
    }];

    // Inject previous_output as a separate User message (Hard Rule 4)
    if let Some(prev) = previous_output {
        // Injection detection (design-plan.md §5.3)
        if prev.starts_with("System:") || prev.contains("You are a") || prev.contains("</") {
            tracing::warn!(
                "Previous output contains system-instruction-like patterns. \
                 This may indicate a prompt injection attempt."
            );
        }

        messages.push(Message {
            role: Role::User,
            content: MessageContent::Text(prev.to_string()),
        });
    }

    messages
}

/// Applies a `ContextPolicy` to a message history.
///
/// For `KeepAll`, returns messages unchanged.
/// For `KeepLastN { n }`, keeps the system message (first) plus the last `n`
/// User/Assistant message pairs.
pub fn apply_context_policy(messages: &[Message], policy: &ContextPolicy) -> Vec<Message> {
    match policy {
        ContextPolicy::KeepAll => messages.to_vec(),
        ContextPolicy::KeepLastN { n } => {
            if messages.is_empty() {
                return Vec::new();
            }

            let n = *n as usize;

            // Always keep the system message (first message)
            let system_msg = if messages[0].role == Role::System {
                Some(messages[0].clone())
            } else {
                None
            };

            let non_system: Vec<&Message> = messages
                .iter()
                .skip(if system_msg.is_some() { 1 } else { 0 })
                .collect();

            // Keep last n*2 messages (n pairs of User/Assistant)
            let keep_count = n * 2;
            let start = non_system.len().saturating_sub(keep_count);
            let kept: Vec<Message> = non_system[start..].iter().cloned().cloned().collect();

            let mut result = Vec::new();
            if let Some(sys) = system_msg {
                result.push(sys);
            }
            result.extend(kept);
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_substitution() {
        let msgs = build_step_messages("Do this: {task}", "write code", None);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::System);
        match &msgs[0].content {
            MessageContent::Text(t) => assert_eq!(t, "Do this: write code"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn previous_output_not_in_system_message() {
        let msgs = build_step_messages("Review: {previous_output}", "task", Some("SECRET CONTENT"));
        // System message must NOT contain previous output
        match &msgs[0].content {
            MessageContent::Text(t) => {
                assert!(!t.contains("SECRET CONTENT"));
                assert!(!t.contains("{previous_output}"));
            }
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn previous_output_as_user_message() {
        let msgs = build_step_messages(
            "Review: {previous_output}",
            "task",
            Some("The generated code"),
        );
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, Role::System);
        assert_eq!(msgs[1].role, Role::User);
        match &msgs[1].content {
            MessageContent::Text(t) => assert_eq!(t, "The generated code"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn no_previous_output_system_only() {
        let msgs = build_step_messages("Do {task}", "thing", None);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::System);
    }

    #[test]
    fn keep_all_passthrough() {
        let msgs = vec![
            Message {
                role: Role::System,
                content: MessageContent::Text("sys".to_string()),
            },
            Message {
                role: Role::User,
                content: MessageContent::Text("u1".to_string()),
            },
            Message {
                role: Role::Assistant,
                content: MessageContent::Text("a1".to_string()),
            },
        ];
        let result = apply_context_policy(&msgs, &ContextPolicy::KeepAll);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn keep_last_n_trims_correctly() {
        let msgs = vec![
            Message {
                role: Role::System,
                content: MessageContent::Text("sys".to_string()),
            },
            Message {
                role: Role::User,
                content: MessageContent::Text("u1".to_string()),
            },
            Message {
                role: Role::Assistant,
                content: MessageContent::Text("a1".to_string()),
            },
            Message {
                role: Role::User,
                content: MessageContent::Text("u2".to_string()),
            },
            Message {
                role: Role::Assistant,
                content: MessageContent::Text("a2".to_string()),
            },
            Message {
                role: Role::User,
                content: MessageContent::Text("u3".to_string()),
            },
            Message {
                role: Role::Assistant,
                content: MessageContent::Text("a3".to_string()),
            },
        ];
        let result = apply_context_policy(&msgs, &ContextPolicy::KeepLastN { n: 1 });
        assert_eq!(result.len(), 3); // system + last pair
        assert_eq!(result[0].role, Role::System);
        match &result[1].content {
            MessageContent::Text(t) => assert_eq!(t, "u3"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn keep_last_n_zero_system_only() {
        let msgs = vec![
            Message {
                role: Role::System,
                content: MessageContent::Text("sys".to_string()),
            },
            Message {
                role: Role::User,
                content: MessageContent::Text("u1".to_string()),
            },
        ];
        let result = apply_context_policy(&msgs, &ContextPolicy::KeepLastN { n: 0 });
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, Role::System);
    }

    #[test]
    fn both_placeholders() {
        let msgs = build_step_messages(
            "{task}\nReview output:\n{previous_output}",
            "write auth",
            Some("generated code here"),
        );
        assert_eq!(msgs.len(), 2);
        match &msgs[0].content {
            MessageContent::Text(t) => {
                assert!(t.contains("write auth"));
                assert!(!t.contains("generated code here"));
                assert!(!t.contains("{previous_output}"));
            }
            _ => panic!("expected text"),
        }
        match &msgs[1].content {
            MessageContent::Text(t) => assert_eq!(t, "generated code here"),
            _ => panic!("expected text"),
        }
    }
}
