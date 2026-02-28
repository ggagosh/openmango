use futures::StreamExt;
use rig::agent::MultiTurnStreamItem;
use rig::client::CompletionClient as _;
use rig::client::Nothing;
use rig::completion::{Chat as _, Message as RigMessage, Prompt as _, PromptError};
use rig::providers::{anthropic, gemini, ollama, openai};
use rig::streaming::{StreamedAssistantContent, StreamingChat as _, StreamingPrompt as _};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::time::Duration;

use crate::ai::blocks::{ChatMessage, ChatRole};
use crate::ai::errors::AiError;
use crate::ai::settings::{AiProvider, AiSettings};

const HISTORY_LIMIT: usize = 18;
const MAX_OUTPUT_TOKENS: u32 = 2048;

#[derive(Debug, Clone)]
pub struct AiGenerationRequest {
    pub system_prompt: String,
    pub history: Vec<ChatMessage>,
    pub user_prompt: String,
}

pub async fn generate_text(
    settings: &AiSettings,
    request: AiGenerationRequest,
) -> Result<String, AiError> {
    settings.validate_for_request()?;
    match settings.provider {
        AiProvider::Gemini => call_gemini(settings, request).await,
        AiProvider::OpenAi => call_openai(settings, request).await,
        AiProvider::Anthropic => call_anthropic(settings, request).await,
        AiProvider::Ollama => call_ollama(settings, request).await,
    }
}

pub async fn generate_text_streaming(
    settings: &AiSettings,
    request: AiGenerationRequest,
    mut on_delta: impl FnMut(&str) + Send,
) -> Result<String, AiError> {
    settings.validate_for_request()?;
    match settings.provider {
        AiProvider::Gemini => call_gemini_streaming(settings, request, &mut on_delta).await,
        AiProvider::OpenAi => call_openai_streaming(settings, request, &mut on_delta).await,
        AiProvider::Anthropic => call_anthropic_streaming(settings, request, &mut on_delta).await,
        AiProvider::Ollama => call_ollama_streaming(settings, request, &mut on_delta).await,
    }
}

async fn call_gemini(
    settings: &AiSettings,
    request: AiGenerationRequest,
) -> Result<String, AiError> {
    let api_key = settings.configured_api_key().ok_or_else(|| AiError::MissingApiKey {
        provider: settings.provider.label().to_string(),
    })?;
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("Gemini model is empty".to_string()));
    }

    let client = gemini::Client::new(api_key).map_err(|error| {
        AiError::Runtime(format!("failed to initialize Gemini client: {error}"))
    })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .temperature(0.2)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let history = to_rig_history(&request.history);
    let response = if history.is_empty() {
        agent.prompt(request.user_prompt).await
    } else {
        agent.chat(request.user_prompt, history).await
    };
    response.map_err(|error| map_rig_error(AiProvider::Gemini, error))
}

async fn call_openai(
    settings: &AiSettings,
    request: AiGenerationRequest,
) -> Result<String, AiError> {
    let api_key = settings.configured_api_key().ok_or_else(|| AiError::MissingApiKey {
        provider: settings.provider.label().to_string(),
    })?;
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("OpenAI model is empty".to_string()));
    }

    let client = openai::Client::<reqwest::Client>::new(api_key).map_err(|error| {
        AiError::Runtime(format!("failed to initialize OpenAI client: {error}"))
    })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .temperature(0.2)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let history = to_rig_history(&request.history);
    let response = if history.is_empty() {
        agent.prompt(request.user_prompt).await
    } else {
        agent.chat(request.user_prompt, history).await
    };
    response.map_err(|error| map_rig_error(AiProvider::OpenAi, error))
}

async fn call_anthropic(
    settings: &AiSettings,
    request: AiGenerationRequest,
) -> Result<String, AiError> {
    let api_key = settings.configured_api_key().ok_or_else(|| AiError::MissingApiKey {
        provider: settings.provider.label().to_string(),
    })?;
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("Anthropic model is empty".to_string()));
    }

    let client = anthropic::Client::<reqwest::Client>::new(api_key).map_err(|error| {
        AiError::Runtime(format!("failed to initialize Anthropic client: {error}"))
    })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .temperature(0.2)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let history = to_rig_history(&request.history);
    let response = if history.is_empty() {
        agent.prompt(request.user_prompt).await
    } else {
        agent.chat(request.user_prompt, history).await
    };
    response.map_err(|error| map_rig_error(AiProvider::Anthropic, error))
}

async fn call_ollama(
    settings: &AiSettings,
    request: AiGenerationRequest,
) -> Result<String, AiError> {
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("Ollama model is empty".to_string()));
    }

    let base_url = settings.ollama_base_url.trim();
    if base_url.is_empty() {
        return Err(AiError::InvalidConfig {
            field: "ollama_base_url".to_string(),
            message: "value cannot be empty".to_string(),
        });
    }
    let available_models = detect_ollama_models(base_url).await?;
    if !available_models.is_empty() && !available_models.iter().any(|available| available == model)
    {
        let sample = available_models.into_iter().take(8).collect::<Vec<_>>().join(", ");
        return Err(AiError::InvalidConfig {
            field: "model".to_string(),
            message: format!(
                "Model '{model}' was not found at {base_url}. Available models: {sample}"
            ),
        });
    }

    let client = ollama::Client::<reqwest::Client>::builder()
        .api_key(Nothing)
        .base_url(base_url)
        .build()
        .map_err(|error| {
            AiError::Runtime(format!("failed to initialize Ollama client: {error}"))
        })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .temperature(0.2)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let history = to_rig_history(&request.history);
    let response = if history.is_empty() {
        agent.prompt(request.user_prompt).await
    } else {
        agent.chat(request.user_prompt, history).await
    };
    response.map_err(|error| map_rig_error(AiProvider::Ollama, error))
}

async fn call_gemini_streaming(
    settings: &AiSettings,
    request: AiGenerationRequest,
    on_delta: &mut (impl FnMut(&str) + Send),
) -> Result<String, AiError> {
    let api_key = settings.configured_api_key().ok_or_else(|| AiError::MissingApiKey {
        provider: settings.provider.label().to_string(),
    })?;
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("Gemini model is empty".to_string()));
    }

    let client = gemini::Client::new(api_key).map_err(|error| {
        AiError::Runtime(format!("failed to initialize Gemini client: {error}"))
    })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .temperature(0.2)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let history = to_rig_history(&request.history);
    let mut stream = if history.is_empty() {
        agent.stream_prompt(request.user_prompt).await
    } else {
        agent.stream_chat(request.user_prompt, history).await
    };

    let mut full_text = String::new();
    let mut final_text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                full_text.push_str(&text.text);
                on_delta(&text.text);
            }
            Ok(MultiTurnStreamItem::FinalResponse(final_response)) => {
                final_text = final_response.response().to_string();
            }
            Ok(_) => {}
            Err(error) => {
                return Err(map_provider_error(AiProvider::Gemini, error.to_string()));
            }
        }
    }

    if full_text.trim().is_empty() {
        full_text = final_text;
    }
    Ok(full_text)
}

async fn call_openai_streaming(
    settings: &AiSettings,
    request: AiGenerationRequest,
    on_delta: &mut (impl FnMut(&str) + Send),
) -> Result<String, AiError> {
    let api_key = settings.configured_api_key().ok_or_else(|| AiError::MissingApiKey {
        provider: settings.provider.label().to_string(),
    })?;
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("OpenAI model is empty".to_string()));
    }

    let client = openai::Client::<reqwest::Client>::new(api_key).map_err(|error| {
        AiError::Runtime(format!("failed to initialize OpenAI client: {error}"))
    })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .temperature(0.2)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let history = to_rig_history(&request.history);
    let mut stream = if history.is_empty() {
        agent.stream_prompt(request.user_prompt).await
    } else {
        agent.stream_chat(request.user_prompt, history).await
    };

    let mut full_text = String::new();
    let mut final_text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                full_text.push_str(&text.text);
                on_delta(&text.text);
            }
            Ok(MultiTurnStreamItem::FinalResponse(final_response)) => {
                final_text = final_response.response().to_string();
            }
            Ok(_) => {}
            Err(error) => {
                return Err(map_provider_error(AiProvider::OpenAi, error.to_string()));
            }
        }
    }

    if full_text.trim().is_empty() {
        full_text = final_text;
    }
    Ok(full_text)
}

async fn call_anthropic_streaming(
    settings: &AiSettings,
    request: AiGenerationRequest,
    on_delta: &mut (impl FnMut(&str) + Send),
) -> Result<String, AiError> {
    let api_key = settings.configured_api_key().ok_or_else(|| AiError::MissingApiKey {
        provider: settings.provider.label().to_string(),
    })?;
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("Anthropic model is empty".to_string()));
    }

    let client = anthropic::Client::<reqwest::Client>::new(api_key).map_err(|error| {
        AiError::Runtime(format!("failed to initialize Anthropic client: {error}"))
    })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .temperature(0.2)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let history = to_rig_history(&request.history);
    let mut stream = if history.is_empty() {
        agent.stream_prompt(request.user_prompt).await
    } else {
        agent.stream_chat(request.user_prompt, history).await
    };

    let mut full_text = String::new();
    let mut final_text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                full_text.push_str(&text.text);
                on_delta(&text.text);
            }
            Ok(MultiTurnStreamItem::FinalResponse(final_response)) => {
                final_text = final_response.response().to_string();
            }
            Ok(_) => {}
            Err(error) => {
                return Err(map_provider_error(AiProvider::Anthropic, error.to_string()));
            }
        }
    }

    if full_text.trim().is_empty() {
        full_text = final_text;
    }
    Ok(full_text)
}

async fn call_ollama_streaming(
    settings: &AiSettings,
    request: AiGenerationRequest,
    on_delta: &mut (impl FnMut(&str) + Send),
) -> Result<String, AiError> {
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("Ollama model is empty".to_string()));
    }

    let base_url = settings.ollama_base_url.trim();
    if base_url.is_empty() {
        return Err(AiError::InvalidConfig {
            field: "ollama_base_url".to_string(),
            message: "value cannot be empty".to_string(),
        });
    }
    let available_models = detect_ollama_models(base_url).await?;
    if !available_models.is_empty() && !available_models.iter().any(|available| available == model)
    {
        let sample = available_models.into_iter().take(8).collect::<Vec<_>>().join(", ");
        return Err(AiError::InvalidConfig {
            field: "model".to_string(),
            message: format!(
                "Model '{model}' was not found at {base_url}. Available models: {sample}"
            ),
        });
    }

    let client = ollama::Client::<reqwest::Client>::builder()
        .api_key(Nothing)
        .base_url(base_url)
        .build()
        .map_err(|error| {
            AiError::Runtime(format!("failed to initialize Ollama client: {error}"))
        })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .temperature(0.2)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let history = to_rig_history(&request.history);
    let mut stream = if history.is_empty() {
        agent.stream_prompt(request.user_prompt).await
    } else {
        agent.stream_chat(request.user_prompt, history).await
    };

    let mut full_text = String::new();
    let mut final_text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                full_text.push_str(&text.text);
                on_delta(&text.text);
            }
            Ok(MultiTurnStreamItem::FinalResponse(final_response)) => {
                final_text = final_response.response().to_string();
            }
            Ok(_) => {}
            Err(error) => {
                return Err(map_provider_error(AiProvider::Ollama, error.to_string()));
            }
        }
    }

    if full_text.trim().is_empty() {
        full_text = final_text;
    }
    Ok(full_text)
}

fn to_rig_history(history: &[ChatMessage]) -> Vec<RigMessage> {
    let mut out = Vec::new();
    for message in history.iter().rev().take(HISTORY_LIMIT).rev() {
        if message.content.trim().is_empty() {
            continue;
        }
        match message.role {
            ChatRole::User => out.push(RigMessage::user(message.content.clone())),
            ChatRole::Assistant => out.push(RigMessage::assistant(message.content.clone())),
            ChatRole::System => {}
        }
    }
    out
}

fn map_provider_error(provider: AiProvider, message: String) -> AiError {
    let provider_name = provider.label().to_string();
    let lower = message.to_lowercase();
    if lower.contains("cancel") || lower.contains("abort") {
        return AiError::Cancelled;
    }
    if lower.contains("401") || lower.contains("403") || lower.contains("unauthorized") {
        return AiError::Unauthorized { provider: provider_name };
    }
    if lower.contains("429") || lower.contains("rate") {
        return AiError::RateLimited { provider: provider_name };
    }
    if lower.contains("timeout") {
        return AiError::Timeout(message);
    }
    AiError::Provider(message)
}

fn map_rig_error(provider: AiProvider, error: PromptError) -> AiError {
    map_provider_error(provider, error.to_string())
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModelEntry>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelEntry {
    #[serde(default)]
    name: String,
}

pub async fn detect_ollama_models(base_url: &str) -> Result<Vec<String>, AiError> {
    let request_url =
        format!("{}/api/tags", base_url.trim().trim_end_matches('/').trim_end_matches("/api"));
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|error| AiError::Runtime(format!("failed to build HTTP client: {error}")))?;

    let response = http.get(&request_url).send().await.map_err(|error| {
        if error.is_timeout() {
            AiError::Timeout(format!("Unable to reach Ollama at {request_url}"))
        } else {
            AiError::Network(format!("Unable to reach Ollama at {request_url}: {error}"))
        }
    })?;

    if !response.status().is_success() {
        return Err(AiError::Runtime(format!(
            "Ollama health check failed at {request_url} with HTTP {}",
            response.status()
        )));
    }

    let parsed: OllamaTagsResponse = response.json().await.map_err(|error| {
        AiError::Parse(format!("Failed to parse Ollama model list from {request_url}: {error}"))
    })?;

    let mut models = BTreeSet::new();
    for model in parsed.models {
        let name = model.name.trim();
        if !name.is_empty() {
            models.insert(name.to_string());
        }
    }
    Ok(models.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_conversion_skips_system_messages() {
        let history = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "hi"),
            ChatMessage::new(ChatRole::Assistant, "hello"),
        ];
        let converted = to_rig_history(&history);
        assert_eq!(converted.len(), 2);
    }
}
