//! The model picker, shared by the AI panel and the Settings pane.
//!
//! Presets come first — most people want "fast, balanced or powerful" and not a model id — then
//! every model the catalogue knows, each with its context size and price.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;

use crate::ai::catalog::ModelCatalog;
use crate::ai::model_registry::{ModelCache, refresh_models};
use crate::ai::settings::{AiProvider, ModelPreset};
use crate::state::AppState;

struct ModelChoice {
    id: String,
    label: String,
    detail: Option<String>,
}

/// Local models come from Ollama; cloud models from the catalogue, presets first.
fn model_choices(
    catalog: &ModelCatalog,
    provider: AiProvider,
    current: &str,
    cached: &ModelCache,
) -> (Vec<ModelChoice>, Vec<ModelChoice>) {
    if provider == AiProvider::Ollama {
        let mut ids = match cached {
            ModelCache::Loaded(list) => list.clone(),
            _ => Vec::new(),
        };
        if !current.trim().is_empty() && !ids.iter().any(|id| id == current) {
            ids.push(current.to_string());
        }
        ids.sort();
        ids.dedup();
        let models =
            ids.into_iter().map(|id| ModelChoice { label: id.clone(), id, detail: None }).collect();
        return (Vec::new(), models);
    }

    let presets: Vec<ModelChoice> = ModelPreset::ALL
        .into_iter()
        .filter_map(|preset| {
            let id = provider.preset_model(preset)?;
            let model = catalog.model(provider, id);
            Some(ModelChoice {
                id: id.to_string(),
                label: format!(
                    "{} · {}",
                    preset.label(),
                    model.map(|model| model.name.as_str()).unwrap_or(id)
                ),
                detail: Some(
                    model
                        .map(|model| model.summary())
                        .unwrap_or_else(|| preset.description().into()),
                ),
            })
        })
        .collect();

    let mut models: Vec<ModelChoice> = catalog
        .models(provider)
        .iter()
        .filter(|model| model.tool_call && !model.is_deprecated())
        .map(|model| ModelChoice {
            id: model.id.clone(),
            label: if model.name.is_empty() { model.id.clone() } else { model.name.clone() },
            detail: Some(model.summary()),
        })
        .collect();
    // A model typed by hand, or one the catalogue dropped, still has to be selectable.
    if !current.trim().is_empty() && !models.iter().any(|choice| choice.id == current) {
        models.push(ModelChoice {
            id: current.to_string(),
            label: current.to_string(),
            detail: Some("Not in the catalogue".to_string()),
        });
    }
    (presets, models)
}

fn choice_item(choice: &ModelChoice, checked: bool) -> PopupMenuItem {
    match &choice.detail {
        Some(detail) => {
            let (label, detail) = (choice.label.clone(), detail.clone());
            PopupMenuItem::element(move |_window, cx| {
                div()
                    .flex()
                    .flex_col()
                    .child(div().text_sm().text_color(cx.theme().foreground).child(label.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(detail.clone()),
                    )
            })
            .checked(checked)
        }
        None => PopupMenuItem::new(choice.label.clone()).checked(checked),
    }
}

/// Fill a popup menu with the current provider's models. Selecting one saves it immediately.
pub fn build_model_menu(mut menu: PopupMenu, state: Entity<AppState>, cx: &App) -> PopupMenu {
    let app_state = state.read(cx);
    let provider = app_state.settings.ai.provider;
    let current = app_state.settings.ai.model.clone();
    let cached = app_state.ai_chat.cached_models.clone();
    let catalog = app_state.ai_chat.catalog();

    menu = menu.label(provider.label());
    if let Some(hint) = match (&cached, provider) {
        (ModelCache::Loading, _) => Some("Loading models…".to_string()),
        (ModelCache::NotFetched, AiProvider::Ollama) => Some("Fetching models…".to_string()),
        (ModelCache::Error(message), _) => Some(crate::helpers::truncate_chars(message, 60)),
        (ModelCache::NoKey, _) => Some("Add an API key in Settings".to_string()),
        _ => None,
    } {
        menu = menu.item(PopupMenuItem::new(hint).disabled(true));
    }

    let (presets, models) = model_choices(&catalog, provider, &current, &cached);
    let select = |choice: &ModelChoice, state: &Entity<AppState>| {
        let state = state.clone();
        let id = choice.id.clone();
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            state.update(cx, |app_state, cx| {
                app_state.settings.ai.set_model(id.clone());
                app_state.save_settings();
                cx.notify();
            });
        }
    };

    for choice in &presets {
        menu =
            menu.item(choice_item(choice, choice.id == current).on_click(select(choice, &state)));
    }
    if !presets.is_empty() && !models.is_empty() {
        menu = menu.item(PopupMenuItem::separator()).label("All models");
    }
    for choice in &models {
        menu =
            menu.item(choice_item(choice, choice.id == current).on_click(select(choice, &state)));
    }

    menu.item(PopupMenuItem::separator()).item(
        PopupMenuItem::new("Refresh models")
            .icon(gpui_kit::component::Icon::new(gpui_kit::component::IconName::Redo))
            .on_click(move |_, _, cx| refresh_models(&state, cx)),
    )
}

/// The label on the picker button: the preset name when one matches, else the model's own name.
pub fn model_button_label(state: &AppState) -> String {
    let provider = state.settings.ai.provider;
    let model = &state.settings.ai.model;
    if let Some(preset) = provider.preset_for_model(model) {
        return format!("{} · {}", provider.label(), preset.label());
    }
    let name = state
        .ai_chat
        .catalog()
        .model(provider, model)
        .map(|info| info.name.clone())
        .unwrap_or_else(|| model.clone());
    format!("{}: {}", provider.label(), crate::helpers::truncate_chars(&name, 24))
}

#[cfg(test)]
mod tests {
    use super::model_choices;
    use crate::ai::catalog::ModelCatalog;
    use crate::ai::model_registry::ModelCache;
    use crate::ai::settings::{AiProvider, ModelPreset};

    #[test]
    fn cloud_choices_lead_with_presets_and_keep_a_custom_model() {
        let catalog = ModelCatalog::bundled();
        let (presets, models) =
            model_choices(&catalog, AiProvider::Anthropic, "my-own-model", &ModelCache::NotFetched);

        assert_eq!(presets.len(), 3);
        assert!(presets[0].label.starts_with("Fast · "));
        assert_eq!(
            presets[1].id,
            AiProvider::Anthropic.preset_model(ModelPreset::Balanced).unwrap()
        );
        assert!(models.iter().any(|choice| choice.id == "my-own-model"));
        assert!(models.iter().all(|choice| choice.detail.is_some()));
    }

    #[test]
    fn local_choices_come_from_ollama_and_include_the_current_model() {
        let catalog = ModelCatalog::bundled();
        let cached = ModelCache::Loaded(vec!["llama3:8b".to_string()]);
        let (presets, models) = model_choices(&catalog, AiProvider::Ollama, "qwen3:32b", &cached);

        assert!(presets.is_empty(), "local models have no presets");
        let ids: Vec<_> = models.iter().map(|choice| choice.id.as_str()).collect();
        assert_eq!(ids, vec!["llama3:8b", "qwen3:32b"]);
    }
}
