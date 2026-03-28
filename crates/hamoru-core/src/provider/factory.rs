//! Provider factory — builds a `ProviderRegistry` from configuration.
//!
//! `build_registry` uses standard API key resolution. `build_registry_with`
//! accepts a custom resolver for test isolation.

use super::anthropic::AnthropicProvider;
use super::ollama::OllamaProvider;
use super::retry::{RetryConfig, RetryProvider};
use super::{LlmProvider, ProviderRegistry};
use crate::Result;
use crate::config::{HamoruConfig, ProviderType, resolve_api_key};

/// Default Anthropic API base URL.
const ANTHROPIC_DEFAULT_URL: &str = "https://api.anthropic.com";

/// Default Ollama API base URL.
const OLLAMA_DEFAULT_URL: &str = "http://localhost:11434";

/// Builds a `ProviderRegistry` from config using standard API key resolution.
///
/// Each provider is wrapped in a `RetryProvider` with default retry settings.
pub fn build_registry(config: &HamoruConfig) -> Result<ProviderRegistry> {
    build_registry_with(config, resolve_api_key)
}

/// Builds a `ProviderRegistry` with a custom API key resolver (for testing).
///
/// Each provider is wrapped in a `RetryProvider` with default retry settings.
pub fn build_registry_with<F>(config: &HamoruConfig, resolver: F) -> Result<ProviderRegistry>
where
    F: Fn(&ProviderType) -> Result<String>,
{
    let mut registry = ProviderRegistry::new();

    for pc in &config.providers {
        let provider: Box<dyn LlmProvider> = match pc.provider_type {
            ProviderType::Anthropic => {
                let key = resolver(&pc.provider_type)?;
                let url = pc
                    .endpoint
                    .clone()
                    .unwrap_or_else(|| ANTHROPIC_DEFAULT_URL.to_string());
                Box::new(AnthropicProvider::new(
                    pc.name.clone(),
                    key,
                    url,
                    pc.models.clone(),
                )?)
            }
            ProviderType::Ollama => {
                let url = pc
                    .endpoint
                    .clone()
                    .unwrap_or_else(|| OLLAMA_DEFAULT_URL.to_string());
                Box::new(OllamaProvider::new(
                    pc.name.clone(),
                    url,
                    pc.models.clone(),
                )?)
            }
        };
        registry.register(Box::new(RetryProvider::new(
            provider,
            RetryConfig::default(),
        )));
    }

    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ProviderType, parse_config};
    use crate::error::HamoruError;

    fn mock_resolver(provider_type: &ProviderType) -> Result<String> {
        match provider_type {
            ProviderType::Anthropic => Ok("test-api-key".to_string()),
            ProviderType::Ollama => Ok(String::new()),
        }
    }

    fn failing_resolver(provider_type: &ProviderType) -> Result<String> {
        match provider_type {
            ProviderType::Anthropic => Err(HamoruError::CredentialNotFound {
                provider: "anthropic".to_string(),
            }),
            ProviderType::Ollama => Ok(String::new()),
        }
    }

    #[test]
    fn build_registry_anthropic_with_injected_key() {
        let yaml = r#"
version: "1"
providers:
  - name: claude
    type: anthropic
    models:
      - claude-sonnet-4-6
"#;
        let config = parse_config(yaml).unwrap();
        let registry = build_registry_with(&config, mock_resolver).unwrap();
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get("claude").unwrap().id(), "claude");
    }

    #[test]
    fn build_registry_ollama_no_key() {
        let yaml = r#"
version: "1"
providers:
  - name: local
    type: ollama
"#;
        let config = parse_config(yaml).unwrap();
        let registry = build_registry_with(&config, mock_resolver).unwrap();
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get("local").unwrap().id(), "local");
    }

    #[test]
    fn build_registry_missing_key_error() {
        let yaml = r#"
version: "1"
providers:
  - name: claude
    type: anthropic
"#;
        let config = parse_config(yaml).unwrap();
        let result = build_registry_with(&config, failing_resolver);
        assert!(result.is_err());
        match result.unwrap_err() {
            HamoruError::CredentialNotFound { provider } => {
                assert_eq!(provider, "anthropic");
            }
            e => panic!("expected CredentialNotFound, got {e:?}"),
        }
    }

    #[test]
    fn build_registry_multiple_providers() {
        let yaml = r#"
version: "1"
providers:
  - name: claude
    type: anthropic
  - name: local
    type: ollama
"#;
        let config = parse_config(yaml).unwrap();
        let registry = build_registry_with(&config, mock_resolver).unwrap();
        assert_eq!(registry.len(), 2);
        assert!(registry.get("claude").is_some());
        assert!(registry.get("local").is_some());
    }

    #[test]
    fn build_registry_custom_endpoint() {
        let yaml = r#"
version: "1"
providers:
  - name: claude
    type: anthropic
    endpoint: https://custom-api.example.com
  - name: local
    type: ollama
    endpoint: http://gpu-server:11434
"#;
        let config = parse_config(yaml).unwrap();
        // Should not fail — endpoint is just stored in the provider
        let registry = build_registry_with(&config, mock_resolver).unwrap();
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn build_registry_empty_providers() {
        let yaml = r#"
version: "1"
providers: []
"#;
        let config = parse_config(yaml).unwrap();
        let registry = build_registry_with(&config, mock_resolver).unwrap();
        assert!(registry.is_empty());
    }
}
