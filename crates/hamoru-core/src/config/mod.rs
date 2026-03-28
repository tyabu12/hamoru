//! Configuration loading for hamoru.
//!
//! Parses `hamoru.yaml` and resolves provider credentials from environment variables.

use std::path::Path;

use serde::Deserialize;

use crate::Result;
use crate::error::HamoruError;

/// Top-level configuration from `hamoru.yaml`.
#[derive(Debug, Deserialize)]
pub struct HamoruConfig {
    /// Schema version (e.g., "1").
    pub version: String,
    /// Configured LLM providers.
    pub providers: Vec<ProviderConfig>,
    /// Default settings.
    #[serde(default)]
    pub defaults: Option<DefaultsConfig>,
}

/// Configuration for a single LLM provider.
#[derive(Debug, Deserialize)]
pub struct ProviderConfig {
    /// User-chosen name for this provider instance (used as provider ID).
    pub name: String,
    /// Provider type (anthropic, ollama).
    #[serde(rename = "type")]
    pub provider_type: ProviderType,
    /// Custom API endpoint URL. Uses provider default if omitted.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Models to expose from this provider. Empty means all known models.
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}

/// Supported provider types.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    /// Anthropic Claude API.
    Anthropic,
    /// Ollama local inference server.
    Ollama,
}

/// A model entry in config — either a simple name or an object with cost overrides.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ModelEntry {
    /// Simple model name (e.g., "claude-sonnet-4-6").
    Simple(String),
    /// Model with cost overrides.
    WithOverride {
        /// Model identifier.
        id: String,
        /// Override cost per input token in USD.
        #[serde(default)]
        cost_per_input_token: Option<f64>,
        /// Override cost per output token in USD.
        #[serde(default)]
        cost_per_output_token: Option<f64>,
    },
}

impl ModelEntry {
    /// Returns the model ID regardless of variant.
    pub fn id(&self) -> &str {
        match self {
            ModelEntry::Simple(id) => id,
            ModelEntry::WithOverride { id, .. } => id,
        }
    }
}

/// Default settings for the project.
#[derive(Debug, Default, Deserialize)]
pub struct DefaultsConfig {
    /// Default policy name.
    #[serde(default)]
    pub policy: Option<String>,
}

/// Loads and parses a `hamoru.yaml` configuration file.
///
/// Returns a `ConfigError` if the file cannot be read or parsed.
pub fn load_config(path: &Path) -> Result<HamoruConfig> {
    let content = std::fs::read_to_string(path).map_err(|e| HamoruError::ConfigError {
        reason: format!(
            "Failed to read config file '{}': {}. Run 'hamoru init' to create one.",
            path.display(),
            e
        ),
    })?;
    parse_config(&content)
}

/// Parses a YAML string into a `HamoruConfig`.
pub fn parse_config(yaml: &str) -> Result<HamoruConfig> {
    serde_yaml::from_str(yaml).map_err(|e| HamoruError::ConfigError {
        reason: format!("Invalid YAML configuration: {e}"),
    })
}

/// Resolves the API key for a given provider type from environment variables.
///
/// - `Anthropic` → reads `HAMORU_ANTHROPIC_API_KEY`
/// - `Ollama` → returns empty string (no key needed)
pub fn resolve_api_key(provider_type: &ProviderType) -> Result<String> {
    match provider_type {
        ProviderType::Anthropic => {
            std::env::var("HAMORU_ANTHROPIC_API_KEY").map_err(|_| HamoruError::CredentialNotFound {
                provider: "anthropic".to_string(),
            })
        }
        ProviderType::Ollama => Ok(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_config() {
        let yaml = r#"
version: "1"

providers:
  - name: claude
    type: anthropic
    models:
      - claude-sonnet-4-6
      - claude-haiku-4-5

  - name: local
    type: ollama
    endpoint: http://localhost:11434
    models:
      - llama3.3:70b
"#;
        let config = parse_config(yaml).unwrap();
        assert_eq!(config.version, "1");
        assert_eq!(config.providers.len(), 2);

        let claude = &config.providers[0];
        assert_eq!(claude.name, "claude");
        assert_eq!(claude.provider_type, ProviderType::Anthropic);
        assert!(claude.endpoint.is_none());
        assert_eq!(claude.models.len(), 2);
        assert_eq!(claude.models[0].id(), "claude-sonnet-4-6");

        let local = &config.providers[1];
        assert_eq!(local.name, "local");
        assert_eq!(local.provider_type, ProviderType::Ollama);
        assert_eq!(local.endpoint.as_deref(), Some("http://localhost:11434"));
        assert_eq!(local.models[0].id(), "llama3.3:70b");
    }

    #[test]
    fn parse_config_with_cost_overrides() {
        let yaml = r#"
version: "1"
providers:
  - name: claude
    type: anthropic
    models:
      - id: claude-sonnet-4-6
        cost_per_input_token: 0.002
        cost_per_output_token: 0.010
"#;
        let config = parse_config(yaml).unwrap();
        let model = &config.providers[0].models[0];
        match model {
            ModelEntry::WithOverride {
                id,
                cost_per_input_token,
                cost_per_output_token,
            } => {
                assert_eq!(id, "claude-sonnet-4-6");
                assert_eq!(*cost_per_input_token, Some(0.002));
                assert_eq!(*cost_per_output_token, Some(0.010));
            }
            ModelEntry::Simple(_) => panic!("expected WithOverride variant"),
        }
    }

    #[test]
    fn parse_config_missing_optional_fields() {
        let yaml = r#"
version: "1"
providers:
  - name: local
    type: ollama
"#;
        let config = parse_config(yaml).unwrap();
        assert_eq!(config.providers.len(), 1);
        assert!(config.providers[0].endpoint.is_none());
        assert!(config.providers[0].models.is_empty());
        assert!(config.defaults.is_none());
    }

    #[test]
    fn parse_invalid_yaml_returns_config_error() {
        let yaml = "not: [valid: yaml: {{";
        let result = parse_config(yaml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            HamoruError::ConfigError { reason } => {
                assert!(reason.contains("Invalid YAML"));
            }
            _ => panic!("expected ConfigError, got {err:?}"),
        }
    }

    #[test]
    fn parse_unknown_provider_type_returns_error() {
        let yaml = r#"
version: "1"
providers:
  - name: foo
    type: unknown_provider
"#;
        let result = parse_config(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_api_key_ollama_returns_empty() {
        let result = resolve_api_key(&ProviderType::Ollama).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn resolve_api_key_anthropic_missing_returns_credential_not_found() {
        // SAFETY: test-only, single-threaded access to env var.
        let original = std::env::var("HAMORU_ANTHROPIC_API_KEY").ok();
        unsafe {
            std::env::remove_var("HAMORU_ANTHROPIC_API_KEY");
        }

        let result = resolve_api_key(&ProviderType::Anthropic);
        assert!(result.is_err());
        match result.unwrap_err() {
            HamoruError::CredentialNotFound { provider } => {
                assert_eq!(provider, "anthropic");
            }
            e => panic!("expected CredentialNotFound, got {e:?}"),
        }

        // Restore
        if let Some(val) = original {
            unsafe {
                std::env::set_var("HAMORU_ANTHROPIC_API_KEY", val);
            }
        }
    }
}
