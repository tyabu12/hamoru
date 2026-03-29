//! Model namespace resolution for the API server.
//!
//! Parses the `model` field from API requests into a `ModelTarget` that
//! determines which execution path to follow:
//! - `hamoru:<policy>` → policy-based routing
//! - `hamoru:workflow:<name>` → workflow execution
//! - `hamoru:agents:<name>` → agent collaboration (Phase 6)
//! - `<provider>:<model>` → direct provider pass-through

use crate::error::HamoruError;

/// The execution target determined by parsing the model field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelTarget {
    /// Route through a named policy (e.g., "cost-optimized").
    Policy {
        /// The policy name to use for model selection.
        policy_name: String,
    },
    /// Execute a named workflow (e.g., "generate-and-review").
    Workflow {
        /// The workflow name to load and execute.
        workflow_name: String,
    },
    /// Execute an agent collaboration (Phase 6, not yet implemented).
    Agents {
        /// The collaboration pattern name.
        collaboration_name: String,
    },
    /// Direct pass-through to a specific provider and model.
    Direct {
        /// Provider identifier (e.g., "claude", "ollama").
        provider: String,
        /// Model identifier within the provider.
        model: String,
    },
}

/// Parse a model string from the API request into a `ModelTarget`.
///
/// Format:
/// - `hamoru:<policy>` → `Policy`
/// - `hamoru:workflow:<name>` → `Workflow`
/// - `hamoru:agents:<name>` → `Agents`
/// - `<provider>:<model>` → `Direct`
///
/// Returns an error if the model string is empty or has no colon separator.
pub fn parse_model_target(model: &str) -> Result<ModelTarget, HamoruError> {
    if model.is_empty() {
        return Err(HamoruError::ModelNotFound {
            provider: String::new(),
            model: model.to_string(),
        });
    }

    let Some((prefix, rest)) = model.split_once(':') else {
        return Err(HamoruError::ModelNotFound {
            provider: String::new(),
            model: model.to_string(),
        });
    };

    if rest.is_empty() {
        return Err(HamoruError::ModelNotFound {
            provider: prefix.to_string(),
            model: model.to_string(),
        });
    }

    if prefix == "hamoru" {
        // Check for sub-namespaces: workflow:, agents:
        if let Some(workflow_name) = rest.strip_prefix("workflow:") {
            if workflow_name.is_empty() {
                return Err(HamoruError::ModelNotFound {
                    provider: "hamoru".to_string(),
                    model: model.to_string(),
                });
            }
            return Ok(ModelTarget::Workflow {
                workflow_name: workflow_name.to_string(),
            });
        }
        if let Some(collab_name) = rest.strip_prefix("agents:") {
            if collab_name.is_empty() {
                return Err(HamoruError::ModelNotFound {
                    provider: "hamoru".to_string(),
                    model: model.to_string(),
                });
            }
            return Ok(ModelTarget::Agents {
                collaboration_name: collab_name.to_string(),
            });
        }
        // Otherwise, it's a policy name
        return Ok(ModelTarget::Policy {
            policy_name: rest.to_string(),
        });
    }

    // Direct provider:model
    Ok(ModelTarget::Direct {
        provider: prefix.to_string(),
        model: rest.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_policy() {
        let target = parse_model_target("hamoru:cost-optimized").unwrap();
        assert_eq!(
            target,
            ModelTarget::Policy {
                policy_name: "cost-optimized".to_string()
            }
        );
    }

    #[test]
    fn parse_workflow() {
        let target = parse_model_target("hamoru:workflow:generate-and-review").unwrap();
        assert_eq!(
            target,
            ModelTarget::Workflow {
                workflow_name: "generate-and-review".to_string()
            }
        );
    }

    #[test]
    fn parse_agents() {
        let target = parse_model_target("hamoru:agents:code-gen-review").unwrap();
        assert_eq!(
            target,
            ModelTarget::Agents {
                collaboration_name: "code-gen-review".to_string()
            }
        );
    }

    #[test]
    fn parse_direct_provider() {
        let target = parse_model_target("claude:claude-sonnet-4-6").unwrap();
        assert_eq!(
            target,
            ModelTarget::Direct {
                provider: "claude".to_string(),
                model: "claude-sonnet-4-6".to_string()
            }
        );
    }

    #[test]
    fn parse_direct_provider_with_colons_in_model() {
        // e.g., "ollama:llama3.3:70b" — only first colon splits
        let target = parse_model_target("ollama:llama3.3:70b").unwrap();
        assert_eq!(
            target,
            ModelTarget::Direct {
                provider: "ollama".to_string(),
                model: "llama3.3:70b".to_string()
            }
        );
    }

    #[test]
    fn parse_empty_string_errors() {
        assert!(parse_model_target("").is_err());
    }

    #[test]
    fn parse_no_colon_errors() {
        assert!(parse_model_target("just-a-name").is_err());
    }

    #[test]
    fn parse_trailing_colon_errors() {
        assert!(parse_model_target("hamoru:").is_err());
    }

    #[test]
    fn parse_workflow_empty_name_errors() {
        assert!(parse_model_target("hamoru:workflow:").is_err());
    }

    #[test]
    fn parse_agents_empty_name_errors() {
        assert!(parse_model_target("hamoru:agents:").is_err());
    }

    #[test]
    fn parse_hamoru_quality_first() {
        let target = parse_model_target("hamoru:quality-first").unwrap();
        assert_eq!(
            target,
            ModelTarget::Policy {
                policy_name: "quality-first".to_string()
            }
        );
    }
}
