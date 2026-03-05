use gpui::App;
use serde::{Deserialize, Serialize};

use crate::ai::errors::AiError;
use crate::helpers::keystore::KeyStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    #[default]
    Gemini,
    OpenAi,
    Anthropic,
    Ollama,
}

impl AiProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gemini => "Gemini",
            Self::OpenAi => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Ollama => "Ollama",
        }
    }

    pub fn env_var(self) -> Option<&'static str> {
        match self {
            Self::Gemini => Some("GEMINI_API_KEY"),
            Self::OpenAi => Some("OPENAI_API_KEY"),
            Self::Anthropic => Some("ANTHROPIC_API_KEY"),
            Self::Ollama => None,
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Self::Gemini => "gemini-3-flash-preview",
            Self::OpenAi => "gpt-5.2",
            Self::Anthropic => "claude-sonnet-4-6",
            Self::Ollama => "qwen3:32b",
        }
    }

    pub fn keystore_id(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
        }
    }

    pub const ALL: [Self; 4] = [Self::Gemini, Self::OpenAi, Self::Anthropic, Self::Ollama];

    pub fn model_options(self, current_model: &str) -> Vec<String> {
        let mut options: Vec<String> = match self {
            Self::Gemini => {
                vec![
                    "gemini-3-flash-preview",
                    "gemini-3.1-pro-preview",
                    "gemini-3.1-flash-lite-preview",
                ]
            }
            Self::OpenAi => vec!["gpt-5.2", "gpt-5.3-instant", "gpt-5-mini", "gpt-5-nano"],
            Self::Anthropic => {
                vec!["claude-opus-4-6", "claude-sonnet-4-6", "claude-haiku-4-5"]
            }
            Self::Ollama => vec![], // dynamic only
        }
        .into_iter()
        .map(String::from)
        .collect();
        if !current_model.trim().is_empty() {
            options.push(current_model.to_string());
        }
        options.sort();
        options.dedup();
        options
    }

    /// Short description for a known model, shown in dropdown menus.
    pub fn model_note(model: &str) -> Option<&'static str> {
        Some(match model {
            // Gemini
            "gemini-3-flash-preview" => "Fast flagship, pro-grade reasoning",
            "gemini-3.1-pro-preview" => "Most capable, complex tasks",
            "gemini-3.1-flash-lite-preview" => "Fastest, budget-friendly",
            // OpenAI
            "gpt-5.2" => "Flagship, strongest reasoning",
            "gpt-5.3-instant" => "Fast, cost-efficient coding",
            "gpt-5-mini" => "Balanced speed and quality",
            "gpt-5-nano" => "Fastest, lightweight tasks",
            // Anthropic
            "claude-opus-4-6" => "Most capable, deep reasoning",
            "claude-sonnet-4-6" => "Balanced, fast and smart",
            "claude-haiku-4-5" => "Fastest, lightweight tasks",
            _ => return None,
        })
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
        };
        assert!(ollama.assistant_available());
    }
}
