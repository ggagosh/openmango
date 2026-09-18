use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use futures::StreamExt;
use rig::agent::{
    AgentHook, HookContext, ModelTurnAction, ModelTurnFinished, MultiTurnStreamItem,
    StreamingResult, ToolCall as ToolCallEvent, ToolCallAction,
};
use rig::client::Nothing;
use rig::completion::{Chat as _, Message as RigMessage, Prompt as _, PromptError};
use rig::prelude::*;
use rig::providers::{anthropic, gemini, ollama, openai, openrouter};
use rig::streaming::{StreamedAssistantContent, StreamedUserContent};
use serde::Deserialize;
use tokio::sync::mpsc::UnboundedSender;

use crate::ai::blocks::{ChatMessage, ChatRole};
use crate::ai::errors::AiError;
use crate::ai::settings::{AiProvider, AiSettings};
use crate::ai::tools::{MongoContext, StreamEvent, build_agent, truncate_str};

const MAX_OUTPUT_TOKENS: u32 = 4096;
/// Used when the catalogue does not say how much the model can hold.
const DEFAULT_CONTEXT_TOKENS: usize = 100_000;
/// However tight the budget looks, the conversation keeps at least this much room.
const MIN_MEMORY_TOKENS: usize = 4_000;

fn estimate_tokens(chars: usize) -> usize {
    (chars as f64 * 0.3) as usize
}

/// Model calls in one run. A step is cheap; being cut off mid-investigation is not.
const MAX_TURNS: usize = 30;
/// Tool calls in one run — the backstop against a model looping on the database.
const MAX_TOOL_CALLS: usize = 20;

// ---------------------------------------------------------------------------
// RunPolicy — what the agent loop is allowed to do
// ---------------------------------------------------------------------------

/// Stops the run when the user hits Stop, and keeps the model from grinding through tool calls
/// forever. rig calls this at every tool call and at the end of every model turn, so a cancelled
/// run ends inside the loop instead of being abandoned mid-request.
#[derive(Clone)]
struct RunPolicy {
    max_calls: usize,
    calls: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
}

impl RunPolicy {
    fn new(max_calls: usize, cancel: Arc<AtomicBool>) -> Self {
        Self { max_calls, calls: Arc::new(AtomicUsize::new(0)), cancel }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

const CANCELLED: &str = "The user stopped this request.";

impl AgentHook for RunPolicy {
    fn on_tool_call(
        &self,
        _ctx: &HookContext,
        event: ToolCallEvent<'_>,
    ) -> impl Future<Output = ToolCallAction> + Send {
        let cancelled = self.cancelled();
        let count = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let (max, name) = (self.max_calls, event.tool_name.to_string());
        async move {
            if cancelled {
                return ToolCallAction::stop(CANCELLED);
            }
            if count > max {
                log::debug!("[ai-hook] skipping tool call #{count} ({name}), limit is {max}");
                ToolCallAction::skip(
                    "TOOL CALL LIMIT REACHED. Do NOT call any more tools. \
                     Respond to the user NOW with what you have found so far.",
                )
            } else {
                log::debug!("[ai-hook] allowing tool call #{count}/{max}: {name}");
                ToolCallAction::run()
            }
        }
    }

    fn on_model_turn_finished(
        &self,
        _ctx: &HookContext,
        _event: ModelTurnFinished<'_>,
    ) -> impl Future<Output = ModelTurnAction> + Send {
        let cancelled = self.cancelled();
        async move {
            if cancelled {
                ModelTurnAction::stop(CANCELLED)
            } else {
                ModelTurnAction::continue_run()
            }
        }
    }
}

/// An HTTP client that retries the failures worth retrying: 429s and transient 5xx, backing off
/// between attempts. rig has no retry of its own, and a rate limit should not end a turn.
fn retrying_http_client() -> reqwest_middleware::ClientWithMiddleware {
    let policy = reqwest_retry::policies::ExponentialBackoff::builder().build_with_max_retries(3);
    reqwest_middleware::ClientBuilder::new(reqwest::Client::default())
        .with(reqwest_retry::RetryTransientMiddleware::new_with_policy(policy))
        .build()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AiGenerationRequest {
    pub system_prompt: String,
    pub history: Vec<ChatMessage>,
    pub user_prompt: String,
    /// Names the conversation in the memory store.
    pub conversation_id: String,
    /// Where the conversation is kept. Without one the turn still runs, it is just not remembered.
    pub memory: Option<crate::ai::memory::ChatMemory>,
    /// The chosen model's context window, from the catalogue. `None` falls back to a safe default.
    pub context_tokens: Option<usize>,
}

/// One completed turn.
#[derive(Debug, Clone, Default)]
pub struct TurnOutcome {
    pub text: String,
    /// What the provider charged for this turn.
    pub usage: crate::ai::blocks::TurnUsage,
    /// Feed this back as the next turn's transcript so the model keeps what its tools found.
    pub transcript: Vec<RigMessage>,
}

/// What the model is shown of the conversation when there is no memory to load it from — the
/// visible chat, replayed. With memory, rig loads the real transcript instead.
fn conversation_history(request: &AiGenerationRequest) -> Vec<RigMessage> {
    let mut history = to_rig_history(&request.history);
    crate::ai::budget::trim_transcript(
        &mut history,
        request.system_prompt.chars().count(),
        request.context_tokens,
    );
    history
}

/// How much of the stored conversation rig may load: what the model can hold, minus the room the
/// system prompt and the answer need. rig drops whole turns to fit, keeping tool calls with their
/// results.
fn memory_policy(request: &AiGenerationRequest) -> rig_memory::TokenWindowMemory {
    rig_memory::TokenWindowMemory::new(
        memory_budget(request),
        rig_memory::HeuristicTokenCounter::anthropic(),
    )
}

/// Tokens the stored conversation may use: the model's window, less the room the system prompt
/// and the answer need, and never so little that the last exchange cannot be loaded.
fn memory_budget(request: &AiGenerationRequest) -> usize {
    let context = request.context_tokens.unwrap_or(DEFAULT_CONTEXT_TOKENS);
    let reserved =
        estimate_tokens(request.system_prompt.chars().count()) + MAX_OUTPUT_TOKENS as usize;
    context.saturating_sub(reserved).max(MIN_MEMORY_TOKENS)
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
        AiProvider::OpenRouter => call_openrouter(settings, request).await,
        AiProvider::Ollama => call_ollama(settings, request).await,
    }
}

pub async fn generate_text_streaming(
    settings: &AiSettings,
    request: AiGenerationRequest,
    tool_ctx: Option<MongoContext>,
    cancel: Arc<AtomicBool>,
    event_tx: UnboundedSender<StreamEvent>,
) -> Result<TurnOutcome, AiError> {
    settings.validate_for_request()?;
    let tool_ctx = tool_ctx.map(|mut ctx| {
        ctx.event_tx = Some(event_tx.clone());
        ctx
    });
    let policy = RunPolicy::new(MAX_TOOL_CALLS, cancel);
    match settings.provider {
        AiProvider::Gemini => {
            call_gemini_streaming(settings, request, tool_ctx, policy, &event_tx).await
        }
        AiProvider::OpenAi => {
            call_openai_streaming(settings, request, tool_ctx, policy, &event_tx).await
        }
        AiProvider::Anthropic => {
            call_anthropic_streaming(settings, request, tool_ctx, policy, &event_tx).await
        }
        AiProvider::OpenRouter => {
            call_openrouter_streaming(settings, request, tool_ctx, policy, &event_tx).await
        }
        AiProvider::Ollama => {
            call_ollama_streaming(settings, request, tool_ctx, policy, &event_tx).await
        }
    }
}

// ---------------------------------------------------------------------------
// Non-streaming providers (unchanged)
// ---------------------------------------------------------------------------

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

    let client = gemini::Client::builder()
        .http_client(retrying_http_client())
        .api_key(api_key)
        .build()
        .map_err(|error| {
            AiError::Runtime(format!("failed to initialize Gemini client: {error}"))
        })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let mut history = conversation_history(&request);
    let response = if history.is_empty() {
        agent.prompt(request.user_prompt).await
    } else {
        agent.chat(request.user_prompt, &mut history).await
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

    let client = openai::Client::builder()
        .http_client(retrying_http_client())
        .api_key(api_key)
        .build()
        .map_err(|error| {
            AiError::Runtime(format!("failed to initialize OpenAI client: {error}"))
        })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let mut history = conversation_history(&request);
    let response = if history.is_empty() {
        agent.prompt(request.user_prompt).await
    } else {
        agent.chat(request.user_prompt, &mut history).await
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

    let client = anthropic::Client::builder()
        .http_client(retrying_http_client())
        .api_key(api_key)
        .build()
        .map_err(|error| {
            AiError::Runtime(format!("failed to initialize Anthropic client: {error}"))
        })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let mut history = conversation_history(&request);
    let response = if history.is_empty() {
        agent.prompt(request.user_prompt).await
    } else {
        agent.chat(request.user_prompt, &mut history).await
    };
    response.map_err(|error| map_rig_error(AiProvider::Anthropic, error))
}

async fn call_openrouter(
    settings: &AiSettings,
    request: AiGenerationRequest,
) -> Result<String, AiError> {
    let api_key = settings.configured_api_key().ok_or_else(|| AiError::MissingApiKey {
        provider: settings.provider.label().to_string(),
    })?;
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("OpenRouter model is empty".to_string()));
    }

    let client = openrouter::Client::builder()
        .http_client(retrying_http_client())
        .api_key(api_key)
        .build()
        .map_err(|error| {
            AiError::Runtime(format!("failed to initialize OpenRouter client: {error}"))
        })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let mut history = conversation_history(&request);
    let response = if history.is_empty() {
        agent.prompt(request.user_prompt).await
    } else {
        agent.chat(request.user_prompt, &mut history).await
    };
    response.map_err(|error| map_rig_error(AiProvider::OpenRouter, error))
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

    let client = ollama::Client::builder()
        .http_client(retrying_http_client())
        .api_key(Nothing)
        .base_url(base_url)
        .build()
        .map_err(|error| {
            AiError::Runtime(format!("failed to initialize Ollama client: {error}"))
        })?;
    let agent = client
        .agent(model)
        .preamble(&request.system_prompt)
        .max_tokens(MAX_OUTPUT_TOKENS as u64)
        .build();

    let mut history = conversation_history(&request);
    let response = if history.is_empty() {
        agent.prompt(request.user_prompt).await
    } else {
        agent.chat(request.user_prompt, &mut history).await
    };
    response.map_err(|error| map_rig_error(AiProvider::Ollama, error))
}

/// Give the agent the conversation store, wrapped in the window policy that decides how much of
/// it fits. Without a store the agent simply has no memory of earlier runs.
fn with_memory(
    builder: rig::agent::AgentBuilder<rig::agent::NoToolConfig>,
    request: &AiGenerationRequest,
) -> rig::agent::AgentBuilder<rig::agent::NoToolConfig> {
    let Some(memory) = request.memory.clone() else {
        return builder;
    };
    builder.memory(rig_memory::PolicyMemory::new(memory, memory_policy(request)))
}

// ---------------------------------------------------------------------------
// Streaming providers
// ---------------------------------------------------------------------------

async fn call_gemini_streaming(
    settings: &AiSettings,
    request: AiGenerationRequest,
    tool_ctx: Option<MongoContext>,
    policy: RunPolicy,
    event_tx: &UnboundedSender<StreamEvent>,
) -> Result<TurnOutcome, AiError> {
    let api_key = settings.configured_api_key().ok_or_else(|| AiError::MissingApiKey {
        provider: settings.provider.label().to_string(),
    })?;
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("Gemini model is empty".to_string()));
    }

    let client = gemini::Client::builder()
        .http_client(retrying_http_client())
        .api_key(api_key)
        .build()
        .map_err(|error| {
            AiError::Runtime(format!("failed to initialize Gemini client: {error}"))
        })?;
    let agent = build_agent(
        with_memory(
            client
                .agent(model)
                .preamble(&request.system_prompt)
                .max_tokens(MAX_OUTPUT_TOKENS as u64)
                .add_hook(policy),
            &request,
        ),
        tool_ctx,
    );

    // With memory, rig loads the conversation itself; `history` only stands in without it.
    let history =
        if request.memory.is_some() { Vec::new() } else { conversation_history(&request) };
    let mut stream = agent
        .stream_chat(request.user_prompt, history)
        .conversation(request.conversation_id.clone())
        .max_turns(MAX_TURNS)
        .await;

    consume_stream(&mut stream, AiProvider::Gemini, event_tx).await
}

async fn call_openai_streaming(
    settings: &AiSettings,
    request: AiGenerationRequest,
    tool_ctx: Option<MongoContext>,
    policy: RunPolicy,
    event_tx: &UnboundedSender<StreamEvent>,
) -> Result<TurnOutcome, AiError> {
    let api_key = settings.configured_api_key().ok_or_else(|| AiError::MissingApiKey {
        provider: settings.provider.label().to_string(),
    })?;
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("OpenAI model is empty".to_string()));
    }

    let client = openai::Client::builder()
        .http_client(retrying_http_client())
        .api_key(api_key)
        .build()
        .map_err(|error| {
            AiError::Runtime(format!("failed to initialize OpenAI client: {error}"))
        })?;
    let agent = build_agent(
        with_memory(
            client
                .agent(model)
                .preamble(&request.system_prompt)
                .max_tokens(MAX_OUTPUT_TOKENS as u64)
                .add_hook(policy),
            &request,
        ),
        tool_ctx,
    );

    // With memory, rig loads the conversation itself; `history` only stands in without it.
    let history =
        if request.memory.is_some() { Vec::new() } else { conversation_history(&request) };
    let mut stream = agent
        .stream_chat(request.user_prompt, history)
        .conversation(request.conversation_id.clone())
        .max_turns(MAX_TURNS)
        .await;

    consume_stream(&mut stream, AiProvider::OpenAi, event_tx).await
}

async fn call_anthropic_streaming(
    settings: &AiSettings,
    request: AiGenerationRequest,
    tool_ctx: Option<MongoContext>,
    policy: RunPolicy,
    event_tx: &UnboundedSender<StreamEvent>,
) -> Result<TurnOutcome, AiError> {
    let api_key = settings.configured_api_key().ok_or_else(|| AiError::MissingApiKey {
        provider: settings.provider.label().to_string(),
    })?;
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("Anthropic model is empty".to_string()));
    }

    let client = anthropic::Client::builder()
        .http_client(retrying_http_client())
        .api_key(api_key)
        .build()
        .map_err(|error| {
            AiError::Runtime(format!("failed to initialize Anthropic client: {error}"))
        })?;
    let agent = build_agent(
        with_memory(
            client
                .agent(model)
                .preamble(&request.system_prompt)
                .max_tokens(MAX_OUTPUT_TOKENS as u64)
                .add_hook(policy),
            &request,
        ),
        tool_ctx,
    );

    // With memory, rig loads the conversation itself; `history` only stands in without it.
    let history =
        if request.memory.is_some() { Vec::new() } else { conversation_history(&request) };
    let mut stream = agent
        .stream_chat(request.user_prompt, history)
        .conversation(request.conversation_id.clone())
        .max_turns(MAX_TURNS)
        .await;

    consume_stream(&mut stream, AiProvider::Anthropic, event_tx).await
}

async fn call_openrouter_streaming(
    settings: &AiSettings,
    request: AiGenerationRequest,
    tool_ctx: Option<MongoContext>,
    policy: RunPolicy,
    event_tx: &UnboundedSender<StreamEvent>,
) -> Result<TurnOutcome, AiError> {
    let api_key = settings.configured_api_key().ok_or_else(|| AiError::MissingApiKey {
        provider: settings.provider.label().to_string(),
    })?;
    let model = settings.model.trim();
    if model.is_empty() {
        return Err(AiError::Parse("OpenRouter model is empty".to_string()));
    }

    let client = openrouter::Client::builder()
        .http_client(retrying_http_client())
        .api_key(api_key)
        .build()
        .map_err(|error| {
            AiError::Runtime(format!("failed to initialize OpenRouter client: {error}"))
        })?;
    let agent = build_agent(
        with_memory(
            client
                .agent(model)
                .preamble(&request.system_prompt)
                .max_tokens(MAX_OUTPUT_TOKENS as u64)
                .add_hook(policy),
            &request,
        ),
        tool_ctx,
    );

    // With memory, rig loads the conversation itself; `history` only stands in without it.
    let history =
        if request.memory.is_some() { Vec::new() } else { conversation_history(&request) };
    let mut stream = agent
        .stream_chat(request.user_prompt, history)
        .conversation(request.conversation_id.clone())
        .max_turns(MAX_TURNS)
        .await;

    consume_stream(&mut stream, AiProvider::OpenRouter, event_tx).await
}

async fn call_ollama_streaming(
    settings: &AiSettings,
    request: AiGenerationRequest,
    tool_ctx: Option<MongoContext>,
    policy: RunPolicy,
    event_tx: &UnboundedSender<StreamEvent>,
) -> Result<TurnOutcome, AiError> {
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

    let client = ollama::Client::builder()
        .http_client(retrying_http_client())
        .api_key(Nothing)
        .base_url(base_url)
        .build()
        .map_err(|error| {
            AiError::Runtime(format!("failed to initialize Ollama client: {error}"))
        })?;
    let agent = build_agent(
        with_memory(
            client
                .agent(model)
                .preamble(&request.system_prompt)
                .max_tokens(MAX_OUTPUT_TOKENS as u64)
                .add_hook(policy),
            &request,
        ),
        tool_ctx,
    );

    // With memory, rig loads the conversation itself; `history` only stands in without it.
    let history =
        if request.memory.is_some() { Vec::new() } else { conversation_history(&request) };
    let mut stream = agent
        .stream_chat(request.user_prompt, history)
        .conversation(request.conversation_id.clone())
        .max_turns(MAX_TURNS)
        .await;

    consume_stream(&mut stream, AiProvider::Ollama, event_tx).await
}

// ---------------------------------------------------------------------------
// Shared streaming loop
// ---------------------------------------------------------------------------

async fn consume_stream(
    stream: &mut StreamingResult,
    provider: AiProvider,
    event_tx: &UnboundedSender<StreamEvent>,
) -> Result<TurnOutcome, AiError> {
    let mut full_text = String::new();
    let mut final_text = String::new();
    let mut transcript = Vec::new();
    let mut usage = crate::ai::blocks::TurnUsage::default();
    let mut turn_count: usize = 0;
    let mut tool_call_count: usize = 0;

    log::debug!("[ai-stream] starting consume_stream for provider={}", provider.label());

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                full_text.push_str(&text.text);
                let _ = event_tx.send(StreamEvent::TextDelta(text.text));
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::ToolCall {
                tool_call,
                internal_call_id,
            })) => {
                tool_call_count += 1;
                let name = tool_call.function.name.clone();
                let args_full = tool_call.function.arguments.to_string();
                let args_preview = truncate_str(&args_full, 200).to_string();
                log::debug!("[ai-stream] tool_call #{tool_call_count}: {name} args={args_preview}");
                let _ = event_tx.send(StreamEvent::ToolCallStart {
                    call_id: internal_call_id,
                    name,
                    args_preview,
                    args_full,
                });
            }
            Ok(MultiTurnStreamItem::StreamUserItem(StreamedUserContent::ToolResult {
                tool_result,
                internal_call_id,
            })) => {
                turn_count += 1;
                // 0.42 reports the executed tool's own name, so results land on the right row
                // even when the same tool runs twice in a turn.
                let name = tool_result.name.clone();
                let (result_preview, result_json) = extract_tool_result(&tool_result);
                log::debug!(
                    "[ai-stream] tool_result #{turn_count}: {name} preview={}",
                    truncate_str(&result_preview, 100)
                );
                let event = match tool_failure_reason(result_json.as_deref().unwrap_or_default()) {
                    Some(reason) => {
                        StreamEvent::ToolCallFailed { call_id: internal_call_id, name, reason }
                    }
                    None => StreamEvent::ToolCallEnd {
                        call_id: internal_call_id,
                        name,
                        result_preview,
                        result_json,
                    },
                };
                let _ = event_tx.send(event);
            }
            Ok(MultiTurnStreamItem::FinalResponse(final_response)) => {
                final_text = final_response.output().to_string();
                transcript = final_response.messages.clone().unwrap_or_default();
                usage = crate::ai::blocks::TurnUsage {
                    input_tokens: final_response.usage.input_tokens,
                    output_tokens: final_response.usage.output_tokens,
                };
                log::debug!(
                    "[ai-stream] final_response after {tool_call_count} tool calls, \
                     {turn_count} results"
                );
            }
            Ok(_) => {
                log::debug!("[ai-stream] other event");
            }
            Err(error) => {
                log::debug!("[ai-stream] error after {tool_call_count} tool calls: {error}");
                // Running out of turns, or the user pressing Stop, ends the run without
                // being a failure: whatever the model already said still stands.
                // Running out of turns, or the user pressing Stop, ends the run without being a
                // failure: whatever the model already said still stands.
                let ended_early = match &error {
                    rig::agent::StreamingError::Prompt(prompt) => match prompt.as_ref() {
                        PromptError::MaxTurnsError { .. } => {
                            Some("*(Tool call limit reached — see the results above.)*")
                        }
                        PromptError::PromptCancelled { .. } => Some("*(Stopped.)*"),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(note) = ended_early {
                    if full_text.trim().is_empty() {
                        let _ = event_tx.send(StreamEvent::TextDelta(note.to_string()));
                        full_text = note.to_string();
                    }
                    break;
                }
                return Err(map_provider_error(provider, error.to_string()));
            }
        }
    }

    log::debug!(
        "[ai-stream] stream ended: {tool_call_count} tool calls, \
         {turn_count} results, text_len={}",
        full_text.len()
    );

    if full_text.trim().is_empty() {
        full_text = final_text;
    }
    Ok(TurnOutcome { text: full_text, transcript, usage })
}

/// rig reports a failed tool call as its result text, tagged with the error variant.
fn tool_failure_reason(result: &str) -> Option<String> {
    const TAGS: [&str; 3] = ["ToolCallError: ", "ToolNotFoundError: ", "JsonError: "];
    let mut text = result.trim_start();
    let mut tagged = false;
    // Agents used as tools can nest the tag.
    while let Some(rest) = TAGS.iter().find_map(|tag| text.strip_prefix(tag)) {
        text = rest;
        tagged = true;
    }
    tagged.then(|| text.trim().to_string())
}

/// Extract both a truncated preview and the full text from a tool result.
fn extract_tool_result(result: &rig::message::ToolResult) -> (String, Option<String>) {
    let parts: Vec<String> = result
        .content
        .iter()
        .filter_map(|content| match content {
            rig::message::ToolResultContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect();
    let combined = parts.join("\n");
    let preview = truncate_str(&combined, 200).to_string();
    let full = Some(combined);
    (preview, full)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn to_rig_history(history: &[ChatMessage]) -> Vec<RigMessage> {
    let mut out = Vec::new();
    for message in history {
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

    fn request(history: Vec<ChatMessage>) -> AiGenerationRequest {
        AiGenerationRequest {
            system_prompt: String::new(),
            history,
            user_prompt: "and how big is orders?".to_string(),
            conversation_id: "chat-1".to_string(),
            memory: None,
            context_tokens: None,
        }
    }

    /// Without a store, the visible chat is what the model gets — and it still has to fit.
    #[test]
    fn the_replayed_chat_is_trimmed_to_the_window() {
        let short = conversation_history(&request(vec![ChatMessage::new(
            ChatRole::User,
            "what collections are there?",
        )]));
        assert_eq!(short.len(), 1);

        let long: Vec<ChatMessage> = (0..40)
            .map(|index| ChatMessage::new(ChatRole::User, format!("{index} {}", "x".repeat(4_000))))
            .collect();
        let mut request = request(long);
        request.context_tokens = Some(20_000);
        assert!(conversation_history(&request).len() < 40, "an over-long chat is cut down");
    }

    /// The window rig is given has to leave room for the prompt and the answer.
    #[test]
    fn the_memory_window_reserves_room_for_the_prompt_and_the_answer() {
        let mut request = request(Vec::new());
        request.context_tokens = Some(200_000);
        request.system_prompt = "x".repeat(20_000);
        let budget = memory_budget(&request);
        assert!(budget < 200_000 - MAX_OUTPUT_TOKENS as usize, "the prompt is paid for");
        assert!(budget > 100_000, "a large window is still mostly conversation");

        // A small window must not be reserved down to nothing.
        request.context_tokens = Some(8_000);
        assert_eq!(memory_budget(&request), MIN_MEMORY_TOKENS);
    }

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

    #[test]
    fn tool_failures_are_recognized_by_rigs_tag() {
        assert_eq!(
            super::tool_failure_reason("ToolCallError: ToolCallError: Collection name is required"),
            Some("Collection name is required".to_string())
        );
        assert_eq!(super::tool_failure_reason("{\"documents\": []}"), None);
    }
}
