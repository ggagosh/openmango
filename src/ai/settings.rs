use gpui_kit::App;
use serde::{Deserialize, Serialize};

use crate::ai::errors::AiError;
use crate::helpers::keystore::KeyStore;

/// Ollama has no catalogue, so a common local model stands in as its default.
const LOCAL_DEFAULT_MODEL: &str = "qwen3:32b";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    #[default]
    Gemini,
    OpenAi,
    Anthropic,
    OpenRouter,
    Ollama,
}

impl AiProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gemini => "Gemini",
            Self::OpenAi => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::OpenRouter => "OpenRouter",
            Self::Ollama => "Ollama",
        }
    }

    pub fn env_var(self) -> Option<&'static str> {
        match self {
            Self::Gemini => Some("GEMINI_API_KEY"),
            Self::OpenAi => Some("OPENAI_API_KEY"),
            Self::Anthropic => Some("ANTHROPIC_API_KEY"),
            Self::OpenRouter => Some("OPENROUTER_API_KEY"),
            Self::Ollama => None,
        }
    }

    /// The model a provider starts on: its balanced preset.
    pub fn default_model(self) -> &'static str {
        match self {
            // A predictable starting point among hundreds; `openrouter/auto` routes anywhere and
            // can answer with images, which is not what this assistant is for.
            Self::OpenRouter => "anthropic/claude-sonnet-5",
            Self::Ollama => LOCAL_DEFAULT_MODEL,
            provider => provider.preset_model(ModelPreset::Balanced).unwrap_or(LOCAL_DEFAULT_MODEL),
        }
    }

    /// An aggregator serves models it did not train, open weights included; a first-party API
    /// only lists what it runs itself.
    pub fn is_aggregator(self) -> bool {
        matches!(self, Self::OpenRouter)
    }

    /// models.dev keys Gemini under "google"; Ollama serves its own list over HTTP.
    pub fn catalog_key(self) -> Option<&'static str> {
        Some(match self {
            Self::Gemini => "google",
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::OpenRouter => "openrouter",
            Self::Ollama => return None,
        })
    }

    /// The curated model behind each preset. Refresh these with `scripts/update_ai_models.sh`
    /// when models.dev ships newer ones; the catalogue tests fail if an id goes stale.
    pub fn preset_model(self, preset: ModelPreset) -> Option<&'static str> {
        Some(match (self, preset) {
            (Self::Gemini, ModelPreset::Fast) => "gemini-3.5-flash-lite",
            (Self::Gemini, ModelPreset::Balanced) => "gemini-3.8-flash",
            (Self::Gemini, ModelPreset::Powerful) => "gemini-3.1-pro-preview",
            (Self::OpenAi, ModelPreset::Fast) => "gpt-5.6-luna",
            (Self::OpenAi, ModelPreset::Balanced) => "gpt-5.6",
            (Self::OpenAi, ModelPreset::Powerful) => "gpt-6-astra",
            (Self::Anthropic, ModelPreset::Fast) => "claude-haiku-4-5",
            (Self::Anthropic, ModelPreset::Balanced) => "claude-sonnet-5",
            (Self::Anthropic, ModelPreset::Powerful) => "claude-opus-5",
            // OpenRouter is a catalogue of hundreds of models from every lab; picking three for
            // the user would be arbitrary, so it offers the full searchable list instead.
            // Local models are whatever Ollama is serving, so they have no presets either.
            (Self::OpenRouter | Self::Ollama, _) => return None,
        })
    }

    /// The preset this model belongs to, or `None` when it was chosen by hand.
    pub fn preset_for_model(self, model: &str) -> Option<ModelPreset> {
        ModelPreset::ALL.into_iter().find(|preset| self.preset_model(*preset) == Some(model))
    }

    pub fn keystore_id(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::OpenRouter => "openrouter",
            Self::Ollama => "ollama",
        }
    }

    pub const ALL: [Self; 5] =
        [Self::Gemini, Self::OpenAi, Self::Anthropic, Self::OpenRouter, Self::Ollama];
}

/// How much model to spend on a question. Each provider maps these to a curated model id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPreset {
    Fast,
    Balanced,
    Powerful,
}

impl ModelPreset {
    pub const ALL: [Self; 3] = [Self::Fast, Self::Balanced, Self::Powerful];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Balanced => "Balanced",
            Self::Powerful => "Powerful",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Fast => "Cheapest and quickest, for short questions",
            Self::Balanced => "The default: good answers at a sane price",
            Self::Powerful => "Most capable, for hard multi-step work",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub provider: AiProvider,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_ollama_base_url")]
    pub ollama_base_url: String,
    #[serde(default)]
    pub share_selected_documents: bool,
    #[serde(default)]
    pub share_sample_documents: bool,
    /// Whether conversations are kept between runs. On by default, visible and reversible in
    /// Settings; turning it off keeps the assistant's memory to the current run.
    #[serde(default = "default_remember_conversations")]
    pub remember_conversations: bool,
    /// Days a stored conversation is kept. `0` keeps conversations until they are deleted.
    #[serde(default = "default_memory_retention_days")]
    pub memory_retention_days: u32,
}

impl Default for AiSettings {
    fn default() -> Self {
        let provider = AiProvider::Gemini;
        Self {
            enabled: false,
            provider,
            model: provider.default_model().to_string(),
            api_key: String::new(),
            ollama_base_url: default_ollama_base_url(),
            share_selected_documents: false,
            share_sample_documents: false,
            remember_conversations: default_remember_conversations(),
            memory_retention_days: default_memory_retention_days(),
        }
    }
}

impl AiSettings {
    pub fn set_provider(&mut self, provider: AiProvider) {
        if self.provider == provider {
            return;
        }
        self.provider = provider;
        self.model = provider.default_model().to_string();
        if provider == AiProvider::Ollama && self.ollama_base_url.trim().is_empty() {
            self.ollama_base_url = default_ollama_base_url();
        }
    }

    pub fn set_model(&mut self, value: String) {
        self.model = value;
    }

    /// The model to actually use. Settings written by an older version — or by hand — can leave
    /// this empty, and "no model" is not a state worth showing anyone.
    pub fn resolved_model(&self) -> String {
        if self.model.trim().is_empty() {
            self.provider.default_model().to_string()
        } else {
            self.model.clone()
        }
    }

    pub fn set_api_key(&mut self, value: String, cx: &App) {
        let provider = self.provider.keystore_id();
        if value.trim().is_empty() {
            KeyStore::delete(cx, provider).detach();
        } else {
            KeyStore::write(cx, provider, value.trim()).detach();
        }
        self.api_key = value;
    }

    pub fn set_ollama_base_url(&mut self, value: String) {
        self.ollama_base_url = value;
    }

    pub fn validate_panel_enabled(&self) -> Result<(), AiError> {
        if !self.enabled {
            return Err(AiError::Disabled);
        }
        Ok(())
    }

    pub fn configured_api_key(&self) -> Option<String> {
        if !self.api_key.trim().is_empty() {
            return Some(self.api_key.trim().to_string());
        }
        let env = self.provider.env_var()?;
        std::env::var(env).ok().filter(|value| !value.trim().is_empty())
    }

    pub fn validate_for_request(&self) -> Result<(), AiError> {
        self.validate_panel_enabled()?;
        if self.model.trim().is_empty() {
            return Err(AiError::InvalidConfig {
                field: "model".to_string(),
                message: "value cannot be empty".to_string(),
            });
        }

        match self.provider {
            AiProvider::Ollama => {
                if self.ollama_base_url.trim().is_empty() {
                    return Err(AiError::InvalidConfig {
                        field: "ollama_base_url".to_string(),
                        message: "value cannot be empty".to_string(),
                    });
                }
            }
            _ => {
                if self.configured_api_key().is_none() {
                    return Err(AiError::MissingApiKey {
                        provider: self.provider.label().to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    /// True when assistant requests can run with current settings.
    pub fn assistant_available(&self) -> bool {
        self.validate_for_request().is_ok()
    }
}

fn default_model() -> String {
    AiProvider::Gemini.default_model().to_string()
}

fn default_remember_conversations() -> bool {
    true
}

fn default_memory_retention_days() -> u32 {
    crate::ai::memory::DEFAULT_RETENTION_DAYS as u32
}

fn default_ollama_base_url() -> String {
    "http://localhost:11434".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_disabled_settings_fails() {
        let settings = AiSettings::default();
        let result = settings.validate_for_request();
        assert!(matches!(result, Err(AiError::Disabled)));
    }

    #[test]
    fn validate_requires_key_for_remote_providers() {
        let settings = AiSettings {
            enabled: true,
            provider: AiProvider::OpenAi,
            model: "gpt-5.2".to_string(),
            api_key: String::new(),
            ..AiSettings::default()
        };
        let result = settings.validate_for_request();
        assert!(matches!(result, Err(AiError::MissingApiKey { .. })));
    }

    #[test]
    fn validate_ollama_does_not_require_api_key() {
        let settings = AiSettings {
            enabled: true,
            provider: AiProvider::Ollama,
            model: "qwen3:32b".to_string(),
            api_key: String::new(),
            ollama_base_url: "http://localhost:11434".to_string(),
            ..AiSettings::default()
        };
        let result = settings.validate_for_request();
        assert!(result.is_ok());
    }

    #[test]
    fn switching_provider_sets_default_model() {
        let mut settings = AiSettings { enabled: true, ..AiSettings::default() };
        settings.set_model("custom".to_string());

        settings.set_provider(AiProvider::OpenAi);
        assert_eq!(settings.model, AiProvider::OpenAi.default_model());

        settings.set_provider(AiProvider::Ollama);
        assert_eq!(settings.model, AiProvider::Ollama.default_model());
        assert!(!settings.ollama_base_url.is_empty());
    }

    #[test]
    fn document_content_sharing_requires_explicit_consent() {
        let defaults = AiSettings::default();
        assert!(!defaults.share_selected_documents);
        assert!(!defaults.share_sample_documents);

        let legacy: AiSettings = serde_json::from_str(
            r#"{"enabled":true,"provider":"ollama","model":"qwen3:32b","api_key":"","ollama_base_url":"http://localhost:11434"}"#,
        )
        .unwrap();
        assert!(!legacy.share_selected_documents);
        assert!(!legacy.share_sample_documents);
    }

    #[test]
    fn assistant_available_requires_valid_request_config() {
        let disabled = AiSettings::default();
        assert!(!disabled.assistant_available());

        let missing_key = AiSettings {
            enabled: true,
            provider: AiProvider::OpenAi,
            model: "gpt-5.2".to_string(),
            api_key: String::new(),
            ..AiSettings::default()
        };
        assert!(!missing_key.assistant_available());

        let ollama = AiSettings {
            enabled: true,
            provider: AiProvider::Ollama,
            model: "qwen3:32b".to_string(),
            api_key: String::new(),
            ollama_base_url: "http://localhost:11434".to_string(),
            ..AiSettings::default()
        };
        assert!(ollama.assistant_available());
    }

    #[test]
    fn presets_round_trip_through_model_ids() {
        for provider in AiProvider::ALL {
            for preset in ModelPreset::ALL {
                let Some(model) = provider.preset_model(preset) else { continue };
                assert_eq!(provider.preset_for_model(model), Some(preset));
            }
        }
        assert_eq!(AiProvider::Anthropic.preset_for_model("some-custom-model"), None);
        assert!(AiProvider::Ollama.preset_model(ModelPreset::Fast).is_none());
    }

    #[test]
    fn every_provider_starts_on_a_model_it_can_actually_use() {
        let catalog = crate::ai::catalog::ModelCatalog::bundled();
        for provider in AiProvider::ALL {
            let model = provider.default_model();
            assert!(!model.trim().is_empty(), "{} has no default model", provider.label());

            if let Some(preset) = provider.preset_model(ModelPreset::Balanced) {
                assert_eq!(model, preset, "a provider with presets starts on the balanced one");
            }
            // A provider with a catalogue must start on a model that is in it.
            if provider.catalog_key().is_some() {
                let info = catalog.model(provider, model).unwrap_or_else(|| {
                    panic!("{} default {model} is not listed", provider.label())
                });
                assert!(
                    info.is_chat_model(provider.is_aggregator()),
                    "{} default {model} cannot hold a tool-calling conversation",
                    provider.label()
                );
            }
        }
    }
}
