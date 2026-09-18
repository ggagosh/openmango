use gpui_kit::{App, AppContext as _, AsyncApp, Entity};

use crate::ai::bridge::AiBridge;
use crate::ai::catalog::{fetch_catalog, load_cached_catalog};
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

/// Load the catalogue cached by an earlier refresh, once per run.
fn ensure_catalog_loaded(state: &Entity<AppState>, cx: &mut App) {
    if state.read(cx).ai_chat.refreshed_catalog.is_some() {
        return;
    }
    let (catalog, etag) = load_cached_catalog();
    state.update(cx, |state, _| {
        state.ai_chat.refreshed_catalog = Some(catalog);
        state.ai_chat.catalog_etag = etag;
    });
}

/// Refresh the model list for the current provider.
///
/// - Ollama: asks the server what it is serving.
/// - Cloud providers with a key: refreshes the models.dev catalogue, sending the stored ETag so
///   an unchanged catalogue costs one empty response. A failure keeps the models we already have.
/// - Cloud providers without a key: reports `NoKey`.
pub fn refresh_models(state: &Entity<AppState>, cx: &mut App) {
    ensure_catalog_loaded(state, cx);
    let settings = state.read(cx).settings.ai.clone();
    let provider = settings.provider;

    if provider == AiProvider::Ollama {
        state.update(cx, |state, cx| {
            state.ai_chat.cached_models = ModelCache::Loading;
            cx.notify();
        });
        let base_url = settings.ollama_base_url.clone();
        let task =
            cx.background_spawn(async move { AiBridge::block_on(detect_ollama_models(&base_url)) });
        let state = state.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let result: Result<Vec<String>, AiError> = task.await;
            cx.update(|cx| {
                state.update(cx, |state, cx| {
                    state.ai_chat.cached_models = match result {
                        Ok(models) => ModelCache::Loaded(models),
                        Err(error) => ModelCache::Error(error.user_message()),
                    };
                    cx.notify();
                });
            });
        })
        .detach();
        return;
    }

    if settings.configured_api_key().is_none() {
        state.update(cx, |state, cx| {
            state.ai_chat.cached_models = ModelCache::NoKey;
            cx.notify();
        });
        return;
    }

    let etag = state.read(cx).ai_chat.catalog_etag.clone();
    state.update(cx, |state, cx| {
        state.ai_chat.cached_models = ModelCache::Loading;
        cx.notify();
    });
    let task = cx.background_spawn(async move { AiBridge::block_on(fetch_catalog(etag)) });
    let state = state.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = task.await;
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                match result {
                    Ok(Some((catalog, etag))) => {
                        state.ai_chat.refreshed_catalog = Some(catalog);
                        state.ai_chat.catalog_etag = etag;
                    }
                    // Unchanged upstream: what we already list is current.
                    Ok(None) => {}
                    Err(error) => {
                        log::warn!("Model catalogue refresh failed: {error}");
                        state.ai_chat.cached_models = ModelCache::Error(error.user_message());
                        cx.notify();
                        return;
                    }
                }
                let models = state.ai_chat.catalog().usable_model_ids(provider);
                state.ai_chat.cached_models = ModelCache::Loaded(models);
                cx.notify();
            });
        });
    })
    .detach();
}

/// Kick off a model fetch from a view.
pub fn spawn_model_fetch<V: 'static>(state: &Entity<AppState>, cx: &mut gpui_kit::Context<V>) {
    let state = state.clone();
    refresh_models(&state, cx);
}
