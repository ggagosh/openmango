//! Index key row management for the index create dialog.

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::input::{InputEvent, InputState};

use crate::components::Button;
use crate::theme::spacing;

use super::IndexCreateDialog;
use super::support::{FieldSuggestion, IndexKeyKind, MAX_SUGGESTIONS};

/// A single row in the index key list.
#[derive(Clone)]
pub(super) struct IndexKeyRow {
    pub id: u64,
    pub field_state: Entity<InputState>,
    pub kind: IndexKeyKind,
}

impl IndexCreateDialog {
    /// Add a new key row to the dialog.
    pub(super) fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row_id = self.next_row_id;
        self.next_row_id += 1;
        let field_state = cx.new(|cx| InputState::new(window, cx).placeholder("Field"));
        let subscription = cx.subscribe_in(
            &field_state,
            window,
            move |view, _state, event, window, cx| match event {
                InputEvent::Focus => {
                    view.active_row_id = Some(row_id);
                    cx.notify();
                }
                InputEvent::Change => {
                    view.enforce_guardrails(window, cx);
                    cx.notify();
                }
                _ => {}
            },
        );
        self._subscriptions.push(subscription);
        self.rows.push(IndexKeyRow { id: row_id, field_state, kind: IndexKeyKind::Asc });
        self.enforce_guardrails(window, cx);
    }

    /// Remove a key row from the dialog.
    pub(super) fn remove_row(&mut self, row_id: u64) {
        self.rows.retain(|row| row.id != row_id);
        if self.active_row_id == Some(row_id) {
            self.active_row_id = None;
        }
    }

    /// Set the kind of a key row.
    pub(super) fn set_row_kind(
        &mut self,
        row_id: u64,
        kind: IndexKeyKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == row_id) {
            row.kind = kind;
            if kind == IndexKeyKind::Wildcard {
                row.field_state.update(cx, |state, cx| {
                    state.set_value("$**".to_string(), window, cx);
                });
            }
        }
        self.enforce_guardrails(window, cx);
        cx.notify();
    }

    /// Get filtered suggestions for a row.
    pub(super) fn suggestions_for_row(&self, row_id: u64, cx: &App) -> Vec<FieldSuggestion> {
        if self.active_row_id != Some(row_id) {
            return Vec::new();
        }
        let row = self.rows.iter().find(|row| row.id == row_id);
        let Some(row) = row else {
            return Vec::new();
        };
        let query = row.field_state.read(cx).value().to_string().to_lowercase();
        let mut suggestions = self
            .suggestions
            .iter()
            .filter(|entry| {
                if query.is_empty() { true } else { entry.path.to_lowercase().contains(&query) }
            })
            .cloned()
            .collect::<Vec<_>>();
        suggestions.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.path.cmp(&b.path)));
        suggestions.truncate(MAX_SUGGESTIONS);
        suggestions
    }

    /// Render suggestions for a row.
    pub(super) fn render_suggestions(
        &self,
        view: Entity<Self>,
        row_id: u64,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let suggestions = self.suggestions_for_row(row_id, cx);
        if suggestions.is_empty() {
            return None;
        }

        let mut row_children = Vec::new();
        for (index, suggestion) in suggestions.into_iter().enumerate() {
            let label = format!("{} ({})", suggestion.path, suggestion.count);
            let target = suggestion.path.clone();
            row_children.push(
                Button::new((SharedString::from(format!("index-suggestion-{row_id}")), index))
                    .ghost()
                    .compact()
                    .label(label)
                    .on_click({
                        let target = target.clone();
                        let view = view.clone();
                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                            view.update(cx, |this, cx| {
                                if let Some(row) = this.rows.iter().find(|row| row.id == row_id) {
                                    row.field_state.update(cx, |state, cx| {
                                        state.set_value(target.clone(), window, cx);
                                    });
                                    this.active_row_id = Some(row_id);
                                    cx.notify();
                                }
                            });
                        }
                    })
                    .into_any_element(),
            );
        }

        Some(
            div()
                .flex()
                .flex_wrap()
                .gap(spacing::xs())
                .px(spacing::sm())
                .py(px(2.0))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Suggestions"))
                .children(row_children)
                .into_any_element(),
        )
    }
}
