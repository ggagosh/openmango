//! The model picker, shared by the AI panel and the Settings pane.
//!
//! Presets come first — most people want "fast, balanced or powerful", not a model id — and the
//! rest of the catalogue sits behind a search box, because a provider can serve dozens of models.

use gpui_kit::component::select::{
    SearchableVec, Select, SelectEvent, SelectGroup, SelectItem, SelectState,
};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, Size};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ai::model_registry::ModelCache;
use crate::ai::settings::{AiProvider, ModelPreset};
use crate::state::AppState;

pub type ModelSelectState = SelectState<SearchableVec<SelectGroup<ModelItem>>>;

/// One row: a model id, what to call it, and what it costs.
#[derive(Clone)]
pub struct ModelItem {
    id: SharedString,
    title: SharedString,
    detail: SharedString,
    haystack: String,
}

impl ModelItem {
    fn new(id: impl Into<SharedString>, title: impl Into<SharedString>, detail: String) -> Self {
        let (id, title) = (id.into(), title.into());
        // Searching by model id matters as much as by name: people paste ids from docs.
        let haystack = format!("{id} {title} {detail}").to_lowercase();
        Self { id, title, detail: detail.into(), haystack }
    }
}

impl SelectItem for ModelItem {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty() || query.split_whitespace().all(|word| self.haystack.contains(word))
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let detail = self.detail.clone();
        div()
            .flex()
            .flex_col()
            .child(div().text_sm().text_color(cx.theme().foreground).child(self.title.clone()))
            .when(!detail.is_empty(), |this| {
                this.child(div().text_xs().text_color(cx.theme().muted_foreground).child(detail))
            })
    }
}

/// The picker's two lists: the presets, and every other model for this provider.
fn model_items(state: &AppState) -> (Vec<ModelItem>, Vec<ModelItem>) {
    let provider = state.settings.ai.provider;
    let current = state.settings.ai.model.clone();

    if provider == AiProvider::Ollama {
        let mut ids = match &state.ai_chat.cached_models {
            ModelCache::Loaded(list) => list.clone(),
            _ => Vec::new(),
        };
        if !current.trim().is_empty() {
            ids.push(current);
        }
        ids.sort();
        ids.dedup();
        let items: Vec<ModelItem> =
            ids.into_iter().map(|id| ModelItem::new(id.clone(), id, String::new())).collect();
        return (Vec::new(), items);
    }

    let catalog = state.ai_chat.catalog();
    let presets: Vec<ModelItem> = ModelPreset::ALL
        .into_iter()
        .filter_map(|preset| {
            let id = provider.preset_model(preset)?;
            let model = catalog.model(provider, id);
            let name = model.map(|model| model.name.as_str()).unwrap_or(id);
            Some(ModelItem::new(
                id,
                format!("{} · {name}", preset.label()),
                model.map(|model| model.summary()).unwrap_or_else(|| preset.description().into()),
            ))
        })
        .collect();

    let mut models: Vec<ModelItem> = catalog
        .models(provider)
        .iter()
        .filter(|model| model.is_chat_model())
        // A preset already has its own row; listing it twice only makes the search noisier.
        .filter(|model| provider.preset_for_model(&model.id).is_none())
        .map(|model| {
            let title = if model.name.is_empty() { model.id.clone() } else { model.name.clone() };
            ModelItem::new(model.id.clone(), title, model.summary())
        })
        .collect();
    // A model typed by hand, or one the catalogue dropped, still has to be selectable.
    if !current.trim().is_empty()
        && catalog.model(provider, &current).is_none()
        && provider.preset_for_model(&current).is_none()
    {
        models.push(ModelItem::new(current.clone(), current, "Not in the catalogue".to_string()));
    }

    (presets, models)
}

/// Presets first, then the rest, as the select's grouped sections.
fn model_groups(state: &AppState) -> Vec<SelectGroup<ModelItem>> {
    let local = state.settings.ai.provider == AiProvider::Ollama;
    let (presets, models) = model_items(state);
    let mut groups = Vec::new();
    if !presets.is_empty() {
        groups.push(SelectGroup::new("Presets").items(presets));
    }
    if !models.is_empty() {
        groups.push(
            SelectGroup::new(if local { "Local models" } else { "All models" }).items(models),
        );
    }
    groups
}

/// Identifies the list currently loaded, so it is rebuilt only when it actually changed —
/// rebuilding on every frame would wipe whatever the user is typing into the search box.
fn groups_key(state: &AppState) -> String {
    let provider = state.settings.ai.provider;
    let count = match provider {
        AiProvider::Ollama => match &state.ai_chat.cached_models {
            ModelCache::Loaded(list) => list.len(),
            _ => 0,
        },
        _ => state.ai_chat.catalog().models(provider).len(),
    };
    format!("{}:{count}:{}", provider.label(), state.settings.ai.model)
}

/// The model picker's state, held by the view that shows it.
pub struct ModelSelector {
    select: Entity<ModelSelectState>,
    key: String,
}

impl ModelSelector {
    pub fn new<V: 'static>(
        state: &Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Self {
        let app_state = state.read(cx);
        let groups = SearchableVec::new(model_groups(app_state));
        let key = groups_key(app_state);
        let current = SharedString::from(app_state.settings.ai.model.clone());

        let select = cx.new(|cx| {
            let mut select = ModelSelectState::new(groups, None, window, cx).searchable(true);
            select.set_selected_value(&current, window, cx);
            select
        });

        let app_state = state.clone();
        cx.subscribe(&select, move |_view, _select, event: &SelectEvent<_>, cx| {
            let SelectEvent::Confirm(Some(id)) = event else { return };
            let id = id.to_string();
            app_state.update(cx, |app_state, cx| {
                if app_state.settings.ai.model == id {
                    return;
                }
                app_state.settings.ai.set_model(id);
                app_state.save_settings();
                cx.notify();
            });
        })
        .detach();

        Self { select, key }
    }

    /// Reload the list when the provider, the catalogue or the chosen model changed.
    pub fn sync(&mut self, state: &Entity<AppState>, window: &mut Window, cx: &mut App) {
        let app_state = state.read(cx);
        let key = groups_key(app_state);
        if key == self.key {
            return;
        }
        self.key = key;
        let groups = SearchableVec::new(model_groups(app_state));
        let current = SharedString::from(app_state.settings.ai.model.clone());
        self.select.update(cx, |select, cx| {
            select.set_items(groups, window, cx);
            select.set_selected_value(&current, window, cx);
        });
    }

    pub fn entity(&self) -> Entity<ModelSelectState> {
        self.select.clone()
    }
}

/// The picker element. Sized by its caller, because the panel and Settings differ.
pub fn model_select(
    select: &Entity<ModelSelectState>,
    size: Size,
    width: Pixels,
) -> Select<SearchableVec<SelectGroup<ModelItem>>> {
    Select::new(select)
        .with_size(size)
        .w(width)
        .menu_width(px(340.0))
        .search_placeholder("Search models…")
}

#[cfg(test)]
mod tests {
    use super::{ModelItem, model_items};
    use crate::ai::settings::{AiProvider, ModelPreset};
    use crate::state::AppState;
    use gpui_kit::component::select::SelectItem as _;

    fn state_for(provider: AiProvider, model: &str) -> AppState {
        let mut state = AppState::new();
        state.settings.ai.provider = provider;
        state.settings.ai.model = model.to_string();
        state
    }

    #[test]
    fn presets_lead_and_are_not_repeated_in_the_full_list() {
        let state = state_for(AiProvider::Anthropic, "claude-sonnet-5");
        let (presets, models) = model_items(&state);

        assert_eq!(presets.len(), 3);
        assert!(presets[0].title.starts_with("Fast · "));
        let balanced = AiProvider::Anthropic.preset_model(ModelPreset::Balanced).unwrap();
        let all: Vec<_> = models.iter().map(|item| item.id.to_string()).collect();
        assert!(!all.contains(&balanced.to_string()), "preset listed twice");
        assert!(!all.is_empty());
    }

    #[test]
    fn a_hand_typed_model_stays_selectable() {
        let state = state_for(AiProvider::OpenAi, "my-own-model");
        let (_, models) = model_items(&state);
        let all: Vec<_> = models.iter().map(|item| item.id.to_string()).collect();
        assert!(all.contains(&"my-own-model".to_string()));
    }

    #[test]
    fn search_matches_id_name_and_price_word_by_word() {
        let item = ModelItem::new("claude-sonnet-5", "Claude Sonnet 5", "1M context".to_string());
        assert!(item.matches("sonnet"));
        assert!(item.matches("claude-sonnet"));
        assert!(item.matches("sonnet 1m"), "every word has to match somewhere");
        assert!(item.matches(""));
        assert!(!item.matches("opus"));
    }
}
