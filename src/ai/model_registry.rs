use gpui::{AppContext as _, AsyncApp, Entity, WeakEntity};

use crate::ai::bridge::AiBridge;
use crate::ai::errors::AiError;
use crate::ai::provider::detect_ollama_models;
use crate::ai::settings::AiProvider;
use crate::state::AppState;

#[derive(Debug, Clone, Default)]
pub enum ModelCache {
    #[default]
    NotFetched,
    Loading,
    Loaded(Vec<String>),
    NoKey,
    Error(String),
}

/// Kick off a model fetch for the current provider.
///
/// - Cloud providers with a configured API key: synchronously sets `Loaded(curated_list)`.
/// - Cloud providers without an API key: sets `NoKey`.
/// - Ollama: sets `Loading`, spawns a background task, writes `Loaded` / `Error` on completion.
pub fn spawn_model_fetch<V: 'static>(state: &Entity<AppState>, cx: &mut gpui::Context<V>) {
    let settings = state.read(cx).settings.ai.clone();
    let provider = settings.provider;

    match provider {
        AiProvider::Ollama => {
            state.update(cx, |s, _| {
                s.ai_chat.cached_models = ModelCache::Loading;
            });
            let base_url = settings.ollama_base_url.clone();
            let state = state.clone();
            let task =
                cx.background_spawn(
                    async move { AiBridge::block_on(detect_ollama_models(&base_url)) },
                );
            cx.spawn(async move |_view: WeakEntity<V>, cx: &mut AsyncApp| {
                let result: Result<Vec<String>, AiError> = task.await;
                let _ = cx.update(|cx| {
                    state.update(cx, |s, cx| {
                        s.ai_chat.cached_models = match result {
                            Ok(models) => ModelCache::Loaded(models),
                            Err(e) => ModelCache::Error(e.user_message()),
                        };
                        cx.notify();
                    });
                });
            })
            .detach();
        }
        _ => {
            let cache = if settings.configured_api_key().is_some() {
                let current_model = settings.model.clone();
                ModelCache::Loaded(provider.model_options(&current_model))
            } else {
                ModelCache::NoKey
            };
            state.update(cx, |s, _| {
                s.ai_chat.cached_models = cache;
            });
        }
    }
}
