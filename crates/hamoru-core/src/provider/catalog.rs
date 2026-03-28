//! Hardcoded model catalog with config override support.
//!
//! Provides default `ModelInfo` for known models. Config overrides can patch
//! pricing and filter which models are exposed.

use crate::config::{ModelEntry, ProviderType};
use crate::provider::types::{Capability, ModelInfo};

/// Returns the default hardcoded models for a given provider type.
pub fn default_models(provider_type: &ProviderType) -> Vec<ModelInfo> {
    match provider_type {
        ProviderType::Anthropic => anthropic_models(),
        ProviderType::Ollama => ollama_models(),
    }
}

/// Applies config overrides to a model list.
///
/// If `entries` is non-empty:
/// - Filters models to only those whose ID appears in `entries`
/// - Patches `cost_per_input_token` / `cost_per_output_token` from `WithOverride` entries
///
/// If `entries` is empty, returns models unchanged.
pub fn apply_overrides(models: &mut Vec<ModelInfo>, entries: &[ModelEntry]) {
    if entries.is_empty() {
        return;
    }

    // Filter to only configured models
    models.retain(|m| entries.iter().any(|e| e.id() == m.id));

    // Apply cost overrides
    for entry in entries {
        let ModelEntry::WithOverride {
            id,
            cost_per_input_token,
            cost_per_output_token,
        } = entry
        else {
            continue;
        };
        if let Some(model) = models.iter_mut().find(|m| m.id == *id) {
            if let Some(cost) = cost_per_input_token {
                model.cost_per_input_token = *cost;
            }
            if let Some(cost) = cost_per_output_token {
                model.cost_per_output_token = *cost;
            }
        }
    }
}

fn anthropic_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "claude-sonnet-4-6".to_string(),
            provider: String::new(), // Set by factory
            context_window: 200_000,
            cost_per_input_token: 3.0 / 1_000_000.0,
            cost_per_output_token: 15.0 / 1_000_000.0,
            cost_per_cached_input_token: Some(0.30 / 1_000_000.0),
            capabilities: vec![
                Capability::Chat,
                Capability::Vision,
                Capability::FunctionCalling,
                Capability::PromptCaching,
            ],
            max_output_tokens: Some(16_384),
        },
        ModelInfo {
            id: "claude-haiku-4-5".to_string(),
            provider: String::new(),
            context_window: 200_000,
            cost_per_input_token: 0.80 / 1_000_000.0,
            cost_per_output_token: 4.0 / 1_000_000.0,
            cost_per_cached_input_token: Some(0.08 / 1_000_000.0),
            capabilities: vec![
                Capability::Chat,
                Capability::Vision,
                Capability::FunctionCalling,
                Capability::PromptCaching,
            ],
            max_output_tokens: Some(8_192),
        },
    ]
}

fn ollama_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "llama3.3:70b".to_string(),
            provider: String::new(),
            context_window: 128_000,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            cost_per_cached_input_token: None,
            capabilities: vec![Capability::Chat],
            max_output_tokens: None,
        },
        ModelInfo {
            id: "qwen2.5-coder:14b".to_string(),
            provider: String::new(),
            context_window: 32_768,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            cost_per_cached_input_token: None,
            capabilities: vec![Capability::Chat],
            max_output_tokens: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_default_models_have_correct_pricing() {
        let models = default_models(&ProviderType::Anthropic);
        assert_eq!(models.len(), 2);

        let sonnet = &models[0];
        assert_eq!(sonnet.id, "claude-sonnet-4-6");
        assert_eq!(sonnet.context_window, 200_000);
        assert!((sonnet.cost_per_input_token - 3.0 / 1_000_000.0).abs() < f64::EPSILON);
        assert!((sonnet.cost_per_output_token - 15.0 / 1_000_000.0).abs() < f64::EPSILON);
        assert!(sonnet.capabilities.contains(&Capability::PromptCaching));

        let haiku = &models[1];
        assert_eq!(haiku.id, "claude-haiku-4-5");
        assert!((haiku.cost_per_input_token - 0.80 / 1_000_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn ollama_default_models_are_free() {
        let models = default_models(&ProviderType::Ollama);
        assert!(!models.is_empty());
        for model in &models {
            assert!((model.cost_per_input_token).abs() < f64::EPSILON);
            assert!((model.cost_per_output_token).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn apply_overrides_filters_to_configured_models() {
        let mut models = default_models(&ProviderType::Anthropic);
        let entries = vec![ModelEntry::Simple("claude-sonnet-4-6".to_string())];
        apply_overrides(&mut models, &entries);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "claude-sonnet-4-6");
    }

    #[test]
    fn apply_overrides_patches_pricing() {
        let mut models = default_models(&ProviderType::Anthropic);
        let entries = vec![ModelEntry::WithOverride {
            id: "claude-sonnet-4-6".to_string(),
            cost_per_input_token: Some(0.002),
            cost_per_output_token: Some(0.010),
        }];
        apply_overrides(&mut models, &entries);

        let sonnet = models.iter().find(|m| m.id == "claude-sonnet-4-6").unwrap();
        assert!((sonnet.cost_per_input_token - 0.002).abs() < f64::EPSILON);
        assert!((sonnet.cost_per_output_token - 0.010).abs() < f64::EPSILON);
    }

    #[test]
    fn apply_overrides_simple_keeps_default_pricing() {
        let mut models = default_models(&ProviderType::Anthropic);
        let original_cost = models[0].cost_per_input_token;
        let entries = vec![ModelEntry::Simple("claude-sonnet-4-6".to_string())];
        apply_overrides(&mut models, &entries);
        assert!((models[0].cost_per_input_token - original_cost).abs() < f64::EPSILON);
    }

    #[test]
    fn apply_overrides_empty_entries_keeps_all_models() {
        let mut models = default_models(&ProviderType::Anthropic);
        let original_len = models.len();
        apply_overrides(&mut models, &[]);
        assert_eq!(models.len(), original_len);
    }
}
