//! Bulk update/replace dialog for documents.

use gpui::*;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputState};
use gpui_component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use mongodb::bson::{Bson, Document, doc};

use crate::bson::{DocumentKey, document_to_relaxed_extjson_string, parse_document_from_json};
use crate::components::{Button, open_confirm_dialog};
use crate::state::{AppCommands, AppEvent, AppState, SessionKey};
use crate::theme::{colors, spacing};

use super::bulk_update_support::{
    BulkUpdateMode, BulkUpdateScope, parse_update_doc, validate_update_doc,
};
use super::shared::{escape_key_subscription, status_text, styled_dropdown_button};

pub struct BulkUpdateDialog {
    state: Entity<AppState>,
    session_key: SessionKey,
    selected_doc: Option<DocumentKey>,
    scope: BulkUpdateScope,
    mode: BulkUpdateMode,
    filter_state: Entity<InputState>,
    update_state: Entity<InputState>,
    error_message: Option<String>,
    updating: bool,
    _subscriptions: Vec<Subscription>,
}

impl BulkUpdateDialog {
    pub fn open(
        state: Entity<AppState>,
        session_key: SessionKey,
        selected_doc: Option<DocumentKey>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view =
            cx.new(|cx| Self::new(state.clone(), session_key, selected_doc, window, cx));
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("Bulk Update / Replace").w(px(760.0)).child(dialog_view.clone())
        });
    }

    fn new(
        state: Entity<AppState>,
        session_key: SessionKey,
        selected_doc: Option<DocumentKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("json")
                .line_number(true)
                .searchable(true)
                .soft_wrap(true)
        });
        let update_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("json")
                .line_number(true)
                .searchable(true)
                .soft_wrap(true)
        });

        let current_filter = state.read(cx).session_filter(&session_key).unwrap_or_default();
        let filter_value = document_to_relaxed_extjson_string(&current_filter);
        filter_state.update(cx, |state, cx| {
            state.set_value(filter_value, window, cx);
        });

        let scope = if selected_doc.is_some() {
            BulkUpdateScope::SelectedDocument
        } else if !current_filter.is_empty() {
            BulkUpdateScope::FilteredQuery
        } else {
            BulkUpdateScope::AllDocuments
        };

        let mut dialog = Self {
            state,
            session_key,
            selected_doc,
            scope,
            mode: BulkUpdateMode::Update,
            filter_state,
            update_state,
            error_message: None,
            updating: false,
            _subscriptions: Vec::new(),
        };

        let template = dialog.mode.template();
        dialog.update_state.update(cx, |state, cx| {
            state.set_value(template.to_string(), window, cx);
        });

        let subscription =
            cx.subscribe_in(&dialog.state, window, move |view, _state, event, window, cx| {
                match event {
                    AppEvent::DocumentsUpdated { session, .. } if session == &view.session_key => {
                        if view.updating {
                            view.updating = false;
                            view.error_message = None;
                            window.close_dialog(cx);
                        }
                    }
                    AppEvent::DocumentsUpdateFailed { session, error }
                        if session == &view.session_key =>
                    {
                        view.updating = false;
                        view.error_message = Some(error.clone());
                        cx.notify();
                    }
                    _ => {}
                }
            });
        dialog._subscriptions.push(subscription);

        dialog._subscriptions.push(escape_key_subscription(cx));

        dialog
    }

    fn set_scope(&mut self, scope: BulkUpdateScope, cx: &mut Context<Self>) {
        self.scope = scope;
        self.error_message = None;
        cx.notify();
    }

    fn set_mode(&mut self, mode: BulkUpdateMode, window: &mut Window, cx: &mut Context<Self>) {
        let previous_template = self.mode.template();
        let next_template = mode.template();
        let current = self.update_state.read(cx).value().to_string();
        let current_trimmed = current.trim();

        self.mode = mode;
        self.error_message = None;

        if current_trimmed.is_empty() || current_trimmed == previous_template.trim() {
            self.update_state.update(cx, |state, cx| {
                state.set_value(next_template.to_string(), window, cx);
            });
        }
        cx.notify();
    }

    fn current_filter(&self, cx: &mut Context<Self>) -> Document {
        self.state.read(cx).session_filter(&self.session_key).unwrap_or_default()
    }

    fn selected_document(&self, doc_key: &DocumentKey, cx: &mut Context<Self>) -> Option<Document> {
        self.state.read(cx).session_draft_or_document(&self.session_key, doc_key)
    }

    fn selected_id(&self, cx: &mut Context<Self>) -> Result<Bson, String> {
        let Some(doc_key) = self.selected_doc.as_ref() else {
            return Err("No document selected.".to_string());
        };
        let Some(doc) = self.selected_document(doc_key, cx) else {
            return Err("Selected document is unavailable.".to_string());
        };
        doc.get("_id").cloned().ok_or_else(|| "Selected document is missing _id.".to_string())
    }

    fn build_filter(&self, cx: &mut Context<Self>) -> Result<Document, String> {
        match self.scope {
            BulkUpdateScope::SelectedDocument => {
                let id = self.selected_id(cx)?;
                Ok(doc! { "_id": id })
            }
            BulkUpdateScope::FilteredQuery => {
                let filter = self.current_filter(cx);
                if filter.is_empty() {
                    Err("No active filter to apply.".to_string())
                } else {
                    Ok(filter)
                }
            }
            BulkUpdateScope::AllDocuments => Ok(Document::new()),
            BulkUpdateScope::CustomFilter => {
                let raw = self.filter_state.read(cx).value().to_string();
                if raw.trim().is_empty() {
                    return Err("Custom filter is empty.".to_string());
                }
                parse_document_from_json(&raw).map_err(|err| format!("Invalid filter JSON: {err}"))
            }
        }
    }

    fn is_read_only(&self, cx: &mut Context<Self>) -> bool {
        self.state.read(cx).active_connection().map(|conn| conn.config.read_only).unwrap_or(false)
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.updating {
            return;
        }
        if self.is_read_only(cx) {
            self.error_message = Some("Read-only connection: writes are disabled.".to_string());
            cx.notify();
            return;
        }

        self.error_message = None;
        let update_raw = self.update_state.read(cx).value().to_string();
        let update_doc = match parse_update_doc(&update_raw) {
            Ok(doc) => doc,
            Err(err) => {
                self.error_message = Some(err);
                cx.notify();
                return;
            }
        };
        let selected_id = self.selected_id(cx).ok();
        if let Err(err) =
            validate_update_doc(self.mode, self.scope, &update_doc, selected_id.as_ref())
        {
            self.error_message = Some(err);
            cx.notify();
            return;
        }
        let filter = match self.build_filter(cx) {
            Ok(filter) => filter,
            Err(err) => {
                self.error_message = Some(err);
                cx.notify();
                return;
            }
        };

        let mode = self.mode;
        let scope = self.scope;
        let title = format!("{} documents", mode.label());
        let target = match scope {
            BulkUpdateScope::SelectedDocument => "the selected document".to_string(),
            BulkUpdateScope::FilteredQuery => "documents matching the current filter".to_string(),
            BulkUpdateScope::AllDocuments => "all documents in this collection".to_string(),
            BulkUpdateScope::CustomFilter => "documents matching the custom filter".to_string(),
        };
        let message = format!("{} {}? This cannot be undone.", mode.label(), target);
        let confirm_label = mode.label();

        let state = self.state.clone();
        let session_key = self.session_key.clone();
        let view = cx.entity();
        open_confirm_dialog(window, cx, title, message, confirm_label, true, move |_window, cx| {
            let filter = filter.clone();
            let update_doc = update_doc.clone();
            view.update(cx, |this, cx| {
                this.updating = true;
                this.error_message = None;
                AppCommands::update_documents_by_filter(
                    state.clone(),
                    session_key.clone(),
                    filter,
                    update_doc,
                    cx,
                );
                cx.notify();
            });
        });
    }

    fn scope_button(
        &self,
        view: Entity<Self>,
        has_selected: bool,
        has_filter: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        styled_dropdown_button("bulk-scope", self.scope.label(), cx).dropdown_menu_with_anchor(
            Corner::BottomLeft,
            {
                let view = view.clone();
                move |menu: PopupMenu, _window, _cx| {
                    menu.item(
                        PopupMenuItem::new(BulkUpdateScope::SelectedDocument.label())
                            .disabled(!has_selected)
                            .on_click({
                                let view = view.clone();
                                move |_, _, cx| {
                                    view.update(cx, |this, cx| {
                                        this.set_scope(BulkUpdateScope::SelectedDocument, cx);
                                    });
                                }
                            }),
                    )
                    .item(
                        PopupMenuItem::new(BulkUpdateScope::FilteredQuery.label())
                            .disabled(!has_filter)
                            .on_click({
                                let view = view.clone();
                                move |_, _, cx| {
                                    view.update(cx, |this, cx| {
                                        this.set_scope(BulkUpdateScope::FilteredQuery, cx);
                                    });
                                }
                            }),
                    )
                    .item(PopupMenuItem::new(BulkUpdateScope::AllDocuments.label()).on_click({
                        let view = view.clone();
                        move |_, _, cx| {
                            view.update(cx, |this, cx| {
                                this.set_scope(BulkUpdateScope::AllDocuments, cx);
                            });
                        }
                    }))
                    .item(
                        PopupMenuItem::new(BulkUpdateScope::CustomFilter.label()).on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_scope(BulkUpdateScope::CustomFilter, cx);
                                });
                            }
                        }),
                    )
                }
            },
        )
    }

    fn mode_button(&self, view: Entity<Self>, cx: &mut Context<Self>) -> impl IntoElement {
        styled_dropdown_button("bulk-mode", self.mode.label(), cx).dropdown_menu_with_anchor(
            Corner::BottomLeft,
            {
                let view = view.clone();
                move |menu: PopupMenu, _window, _cx| {
                    menu.item(PopupMenuItem::new(BulkUpdateMode::Update.label()).on_click({
                        let view = view.clone();
                        move |_, window, cx| {
                            view.update(cx, |this, cx| {
                                this.set_mode(BulkUpdateMode::Update, window, cx);
                            });
                        }
                    }))
                    .item(
                        PopupMenuItem::new(BulkUpdateMode::Replace.label()).on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_mode(BulkUpdateMode::Replace, window, cx);
                                });
                            }
                        }),
                    )
                }
            },
        )
    }
}

impl Render for BulkUpdateDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let has_selected = self.selected_doc.is_some();
        let has_filter = !self.current_filter(cx).is_empty();

        let status =
            status_text(self.error_message.as_ref(), self.updating, "Applying update...", "");

        let scope_row = div()
            .flex()
            .items_center()
            .gap(spacing::sm())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_xs().text_color(colors::text_secondary()).child("Scope"))
                    .child(self.scope_button(view.clone(), has_selected, has_filter, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_xs().text_color(colors::text_secondary()).child("Mode"))
                    .child(self.mode_button(view.clone(), cx)),
            );

        let custom_filter_row = if self.scope == BulkUpdateScope::CustomFilter {
            div()
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .child(div().text_xs().text_color(colors::text_secondary()).child("Custom filter"))
                .child(
                    Input::new(&self.filter_state)
                        .font_family(crate::theme::fonts::mono())
                        .h(px(140.0))
                        .w_full()
                        .disabled(self.updating),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };

        let update_label = if self.mode == BulkUpdateMode::Replace {
            "Replacement document"
        } else {
            "Update document"
        };

        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .p(spacing::md())
            .child(scope_row)
            .child(custom_filter_row)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_xs().text_color(colors::text_secondary()).child(update_label))
                    .child(
                        Input::new(&self.update_state)
                            .font_family(crate::theme::fonts::mono())
                            .h(px(240.0))
                            .w_full()
                            .disabled(self.updating),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt(spacing::xs())
                    .child(div().min_h(px(18.0)).text_sm().text_color(status.1).child(status.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(Button::new("cancel-bulk-update").label("Cancel").on_click(
                                |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    window.close_dialog(cx);
                                },
                            ))
                            .child(
                                Button::new("apply-bulk-update")
                                    .primary()
                                    .label(self.mode.label())
                                    .disabled(self.updating)
                                    .on_click({
                                        let view = view.clone();
                                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                            view.update(cx, |this, cx| {
                                                this.submit(window, cx);
                                            });
                                        }
                                    }),
                            ),
                    ),
            )
    }
}
