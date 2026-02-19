//! Non-modal JSON editor tab view.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::input::{Input, InputEvent, InputState};

use crate::bson::{
    format_relaxed_json_value, parse_document_from_json, parse_value_from_relaxed_json,
};
use crate::components::Button;
use crate::state::{AppCommands, AppEvent, AppState, JsonEditorTarget, StatusMessage};
use crate::theme::{fonts, spacing};

pub struct JsonEditorView {
    state: Entity<AppState>,
    editor_state: Option<Entity<InputState>>,
    active_tab_id: Option<uuid::Uuid>,
    inline_notice: Option<(bool, String)>,
    _subscriptions: Vec<Subscription>,
}

impl JsonEditorView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];
        subscriptions.push(cx.subscribe(&state, |this, state, event, cx| {
            let Some(tab_id) = this.active_tab_id else {
                return;
            };
            let Some(tab) = state.read(cx).json_editor_tab(tab_id).cloned() else {
                return;
            };

            match event {
                AppEvent::DocumentSaved { session, document } => {
                    if tab.session_key == *session
                        && matches!(
                            tab.target,
                            JsonEditorTarget::Document {
                                doc_key: ref tab_doc_key,
                                ..
                            } if tab_doc_key == document
                        )
                    {
                        this.inline_notice = Some((false, "Saved".to_string()));
                        cx.notify();
                    }
                }
                AppEvent::DocumentSaveFailed { session, error } => {
                    if tab.session_key == *session
                        && matches!(tab.target, JsonEditorTarget::Document { .. })
                    {
                        this.inline_notice = Some((true, format!("Save failed: {error}")));
                        cx.notify();
                    }
                }
                AppEvent::DocumentInserted => {
                    if matches!(tab.target, JsonEditorTarget::Insert) {
                        this.inline_notice = Some((false, "Inserted".to_string()));
                        cx.notify();
                    }
                }
                AppEvent::DocumentInsertFailed { error } => {
                    if matches!(tab.target, JsonEditorTarget::Insert) {
                        this.inline_notice = Some((true, format!("Insert failed: {error}")));
                        cx.notify();
                    }
                }
                _ => {}
            }
        }));
        Self {
            state,
            editor_state: None,
            active_tab_id: None,
            inline_notice: None,
            _subscriptions: subscriptions,
        }
    }

    fn ensure_editor_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_state.is_some() {
            return;
        }

        let editor_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("javascript")
                .line_number(true)
                .searchable(true)
                .soft_wrap(true)
        });

        let app_state = self.state.clone();
        let input_sub =
            cx.subscribe_in(&editor_state, window, move |_view, state, event, _window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }

                let value = state.read(cx).value().to_string();
                app_state.update(cx, |app_state, _cx| {
                    if let Some(tab_id) = app_state.active_json_editor_tab_id() {
                        app_state.set_json_editor_tab_content(tab_id, value);
                    }
                });
            });
        self._subscriptions.push(input_sub);
        self.editor_state = Some(editor_state);
    }

    fn sync_editor_from_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active_tab_id = self.state.read(cx).active_json_editor_tab_id();
        if self.active_tab_id == active_tab_id {
            return;
        }
        self.active_tab_id = active_tab_id;
        self.inline_notice = None;

        let next_content = active_tab_id
            .and_then(|tab_id| {
                self.state.read(cx).json_editor_tab(tab_id).map(|tab| tab.content.clone())
            })
            .unwrap_or_default();

        if let Some(editor_state) = self.editor_state.clone() {
            editor_state.update(cx, |state, cx| {
                state.set_value(next_content, window, cx);
            });
        }
    }

    fn set_notice(&mut self, is_error: bool, message: impl Into<String>) {
        self.inline_notice = Some((is_error, message.into()));
    }

    fn set_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        let message = message.into();
        self.set_notice(true, message.clone());
        self.state.update(cx, |state, cx| {
            state.set_status_message(Some(StatusMessage::error(message.clone())));
            cx.notify();
        });
    }

    fn format_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor_state) = self.editor_state.clone() else {
            return;
        };
        let raw = editor_state.read(cx).value().to_string();
        let formatted = match parse_value_from_relaxed_json(&raw) {
            Ok(value) => format_relaxed_json_value(&value),
            Err(err) => {
                self.set_error(format!("Invalid JSON: {err}"), cx);
                return;
            }
        };

        editor_state.update(cx, |state, cx| {
            state.set_value(formatted.clone(), window, cx);
        });
        self.state.update(cx, |state, _cx| {
            if let Some(tab_id) = state.active_json_editor_tab_id() {
                state.set_json_editor_tab_content(tab_id, formatted);
            }
        });
        self.set_notice(false, "Formatted");
    }

    fn save_or_insert(&mut self, cx: &mut Context<Self>) {
        let Some(editor_state) = self.editor_state.clone() else {
            return;
        };
        let raw = editor_state.read(cx).value().to_string();
        let document = match parse_document_from_json(&raw) {
            Ok(document) => document,
            Err(err) => {
                self.set_error(format!("Invalid JSON: {err}"), cx);
                return;
            }
        };

        let Some(tab_id) = self.active_tab_id else {
            return;
        };
        let Some(tab) = self.state.read(cx).json_editor_tab(tab_id).cloned() else {
            return;
        };

        match tab.target {
            JsonEditorTarget::Insert => {
                self.set_notice(false, "Inserting...");
                AppCommands::insert_document(self.state.clone(), tab.session_key, document, cx);
            }
            JsonEditorTarget::Document { doc_key, baseline_document } => {
                let latest =
                    self.state.read(cx).session_draft_or_document(&tab.session_key, &doc_key);
                let Some(latest) = latest else {
                    self.set_error("Document no longer exists.", cx);
                    return;
                };
                if latest != baseline_document {
                    self.set_error(
                        "Document changed since opening this tab. Reload and retry to avoid overwrite.",
                        cx,
                    );
                    return;
                }

                self.set_notice(false, "Saving...");
                AppCommands::save_document(
                    self.state.clone(),
                    tab.session_key,
                    doc_key,
                    document,
                    cx,
                );
            }
        }
    }
}

impl Render for JsonEditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_editor_state(window, cx);
        self.sync_editor_from_active_tab(window, cx);

        let Some(tab_id) = self.active_tab_id else {
            return div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Open a JSON editor tab to edit or insert a document");
        };
        let Some(tab) = self.state.read(cx).json_editor_tab(tab_id).cloned() else {
            return div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("JSON editor tab is unavailable");
        };

        let subtitle = format!("{}/{}", tab.session_key.database, tab.session_key.collection);
        let mode_label = match &tab.target {
            JsonEditorTarget::Insert => "Insert document",
            JsonEditorTarget::Document { .. } => "Edit document",
        };
        let primary_label = match &tab.target {
            JsonEditorTarget::Insert => "Insert",
            JsonEditorTarget::Document { .. } => "Save",
        };
        let notice = self.inline_notice.clone();

        let editor = self.editor_state.clone().unwrap();

        let view = cx.entity();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(spacing::md())
                    .py(spacing::sm())
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().tab_bar.opacity(0.35))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div().text_sm().font_weight(FontWeight::MEDIUM).child(mode_label),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(subtitle),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                Button::new("json-editor-format")
                                    .compact()
                                    .label("Format JSON")
                                    .on_click({
                                        let view = view.clone();
                                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                            view.update(cx, |this, cx| {
                                                this.format_json(window, cx)
                                            });
                                        }
                                    }),
                            )
                            .child(
                                Button::new("json-editor-save")
                                    .primary()
                                    .compact()
                                    .label(primary_label)
                                    .on_click({
                                        let view = view.clone();
                                        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                            view.update(cx, |this, cx| this.save_or_insert(cx));
                                        }
                                    }),
                            ),
                    ),
            )
            .when_some(notice, |this, (is_error, message)| {
                this.child(
                    div()
                        .px(spacing::md())
                        .pt(px(6.0))
                        .text_xs()
                        .text_color(if is_error {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(message),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .p(spacing::md())
                    .child(Input::new(&editor).font_family(fonts::mono()).h_full().w_full()),
            )
    }
}
