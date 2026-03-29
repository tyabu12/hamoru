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
    /// Telemetry storage configuration.
    #[serde(default)]
    pub telemetry: Option<TelemetryConfig>,
    /// API server configuration for `hamoru serve`.
    #[serde(default)]
    pub server: Option<ServerConfig>,
}

impl HamoruConfig {
    /// Returns the configured local telemetry database path, or the default.
    pub fn telemetry_local_path(&self) -> &str {
        self.telemetry
            .as_ref()
            .and_then(|t| t.local.as_ref())
            .map(|l| l.path.as_str())
            .unwrap_or(".hamoru/state.db")
    }
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

/// API server configuration for `hamoru serve`.
///
/// All fields are optional with sane defaults, ensuring backward compatibility
/// with config files that predate Phase 5b. This struct contains YAML parsing
/// types only — runtime construction of axum layers is the CLI's responsibility.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Environment variable names that hold API keys (informational).
    /// Actual key resolution uses `HAMORU_API_KEYS` env var at startup.
    #[serde(default)]
    pub api_key_env_names: Vec<String>,

    /// Rate limiting configuration.
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,

    /// Non-streaming request timeout in seconds. Default: 300.
    #[serde(default = "ServerConfig::default_request_timeout_secs")]
    pub request_timeout_secs: u64,

    /// Max time between consecutive streaming chunks before timeout. Default: 30.
    #[serde(default = "ServerConfig::default_stream_stall_timeout_secs")]
    pub stream_stall_timeout_secs: u64,

    /// Max total duration for a streaming response. Default: 300.
    #[serde(default = "ServerConfig::default_max_stream_duration_secs")]
    pub max_stream_duration_secs: u64,

    /// Max request body size in bytes. Default: 10 MB.
    #[serde(default = "ServerConfig::default_max_request_body_bytes")]
    pub max_request_body_bytes: usize,
}

impl ServerConfig {
    fn default_request_timeout_secs() -> u64 {
        300
    }
    fn default_stream_stall_timeout_secs() -> u64 {
        30
    }
    fn default_max_stream_duration_secs() -> u64 {
        300
    }
    fn default_max_request_body_bytes() -> usize {
        10 * 1024 * 1024
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            api_key_env_names: Vec::new(),
            rate_limit: None,
            request_timeout_secs: Self::default_request_timeout_secs(),
            stream_stall_timeout_secs: Self::default_stream_stall_timeout_secs(),
            max_stream_duration_secs: Self::default_max_stream_duration_secs(),
            max_request_body_bytes: Self::default_max_request_body_bytes(),
        }
    }
}

/// Rate limiting configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum requests per minute. Default: 60.
    #[serde(default = "RateLimitConfig::default_requests_per_minute")]
    pub requests_per_minute: u32,
}

impl RateLimitConfig {
    fn default_requests_per_minute() -> u32 {
        60
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: Self::default_requests_per_minute(),
        }
    }
}

/// Telemetry storage configuration.
#[derive(Debug, Default, Deserialize)]
pub struct TelemetryConfig {
    /// Local storage settings.
    #[serde(default)]
    pub local: Option<LocalTelemetryConfig>,
    /// Remote storage settings (S3/R2).
    #[serde(default)]
    pub remote: Option<RemoteTelemetryConfig>,
}

/// Local telemetry storage configuration.
#[derive(Debug, Deserialize)]
pub struct LocalTelemetryConfig {
    /// Path to the SQLite database file.
    pub path: String,
}

/// Remote telemetry storage configuration.
#[derive(Debug, Deserialize)]
pub struct RemoteTelemetryConfig {
    /// Remote storage backend.
    pub backend: RemoteBackend,
    /// Bucket name.
    pub bucket: String,
    /// AWS region or "auto".
    #[serde(default)]
    pub region: Option<String>,
    /// Custom endpoint URL (for R2, MinIO, etc.).
    #[serde(default)]
    pub endpoint: Option<String>,
}

/// Supported remote storage backends.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteBackend {
    /// S3-compatible storage (AWS S3, Cloudflare R2, MinIO).
    S3,
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

    #[test]
    fn parse_config_with_telemetry() {
        let yaml = r#"
version: "1"
providers:
  - name: local
    type: ollama
telemetry:
  local:
    path: /tmp/custom.db
  remote:
    backend: s3
    bucket: my-telemetry
    region: us-east-1
    endpoint: https://r2.example.com
"#;
        let config = parse_config(yaml).unwrap();
        let telem = config.telemetry.as_ref().unwrap();
        assert_eq!(telem.local.as_ref().unwrap().path, "/tmp/custom.db");
        let remote = telem.remote.as_ref().unwrap();
        assert_eq!(remote.backend, RemoteBackend::S3);
        assert_eq!(remote.bucket, "my-telemetry");
        assert_eq!(remote.region.as_deref(), Some("us-east-1"));
        assert_eq!(remote.endpoint.as_deref(), Some("https://r2.example.com"));
    }

    #[test]
    fn parse_config_without_telemetry() {
        let yaml = r#"
version: "1"
providers:
  - name: local
    type: ollama
"#;
        let config = parse_config(yaml).unwrap();
        assert!(config.telemetry.is_none());
        assert_eq!(config.telemetry_local_path(), ".hamoru/state.db");
    }

    #[test]
    fn parse_config_with_server_section() {
        let yaml = r#"
version: "1"
providers:
  - name: local
    type: ollama
server:
  api_key_env_names:
    - HAMORU_API_KEY_PROD
    - HAMORU_API_KEY_DEV
  rate_limit:
    requests_per_minute: 120
  request_timeout_secs: 600
  stream_stall_timeout_secs: 60
  max_stream_duration_secs: 600
  max_request_body_bytes: 5242880
"#;
        let config = parse_config(yaml).unwrap();
        let server = config.server.as_ref().unwrap();
        assert_eq!(server.api_key_env_names.len(), 2);
        assert_eq!(server.api_key_env_names[0], "HAMORU_API_KEY_PROD");
        assert_eq!(server.request_timeout_secs, 600);
        assert_eq!(server.stream_stall_timeout_secs, 60);
        assert_eq!(server.max_stream_duration_secs, 600);
        assert_eq!(server.max_request_body_bytes, 5_242_880);
        let rate = server.rate_limit.as_ref().unwrap();
        assert_eq!(rate.requests_per_minute, 120);
    }

    #[test]
    fn parse_config_without_server_section() {
        let yaml = r#"
version: "1"
providers:
  - name: local
    type: ollama
"#;
        let config = parse_config(yaml).unwrap();
        assert!(config.server.is_none());
    }

    #[test]
    fn server_config_defaults() {
        let config = ServerConfig::default();
        assert!(config.api_key_env_names.is_empty());
        assert!(config.rate_limit.is_none());
        assert_eq!(config.request_timeout_secs, 300);
        assert_eq!(config.stream_stall_timeout_secs, 30);
        assert_eq!(config.max_stream_duration_secs, 300);
        assert_eq!(config.max_request_body_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn server_config_partial_yaml_uses_defaults() {
        let yaml = r#"
version: "1"
providers:
  - name: local
    type: ollama
server:
  request_timeout_secs: 60
"#;
        let config = parse_config(yaml).unwrap();
        let server = config.server.as_ref().unwrap();
        assert_eq!(server.request_timeout_secs, 60);
        // Other fields should be defaults
        assert_eq!(server.stream_stall_timeout_secs, 30);
        assert_eq!(server.max_request_body_bytes, 10 * 1024 * 1024);
        assert!(server.rate_limit.is_none());
    }

    #[test]
    fn parse_config_partial_telemetry() {
        let yaml = r#"
version: "1"
providers:
  - name: local
    type: ollama
telemetry:
  local:
    path: custom.db
"#;
        let config = parse_config(yaml).unwrap();
        let telem = config.telemetry.as_ref().unwrap();
        assert!(telem.remote.is_none());
        assert_eq!(config.telemetry_local_path(), "custom.db");
    }
}
