//! Detached JSON editor window.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Root;
use gpui_component::input::{Input, InputEvent, InputState};
use mongodb::bson::{Bson, Document, oid::ObjectId};

use crate::bson::{
    DocumentKey, document_to_shell_string, format_relaxed_json_value, parse_bson_from_relaxed_json,
    parse_document_from_json, parse_value_from_relaxed_json,
};
use crate::components::Button;
use crate::keyboard::CloseEditorWindow;
use crate::state::{
    AppCommands, AppEvent, AppState, EditorSessionId, EditorSessionStore, EditorSessionTarget,
    SessionKey,
};
use crate::theme::{fonts, spacing};

const DETACHED_WINDOW_WIDTH: f32 = 980.0;
const DETACHED_WINDOW_HEIGHT: f32 = 760.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncIssue {
    ConflictChanged,
    MissingOriginal,
    ReloadFailed,
    LocalDraftConflict,
}

pub struct DetachedJsonEditorView {
    state: Entity<AppState>,
    sessions: EditorSessionStore,
    session_id: EditorSessionId,
    window_handle: Option<AnyWindowHandle>,
    editor_state: Option<Entity<InputState>>,
    inline_notice: Option<(bool, String)>,
    sync_issue: Option<SyncIssue>,
    pending_editor_content: Option<String>,
    awaiting_create_as_new: bool,
    _subscriptions: Vec<Subscription>,
}

impl DetachedJsonEditorView {
    pub fn new(
        state: Entity<AppState>,
        sessions: EditorSessionStore,
        session_id: EditorSessionId,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe(&state, |this, state, event, cx| {
            let Some(session) = this.sessions.snapshot(this.session_id) else {
                return;
            };

            match event {
                AppEvent::DocumentSaved { session: saved_session, document } => {
                    if session.session_key == *saved_session
                        && matches!(
                            session.target,
                            EditorSessionTarget::Document {
                                doc_key: ref tab_doc_key,
                                ..
                            } if tab_doc_key == document
                        )
                    {
                        if let Some(latest) =
                            state.read(cx).session_draft_or_document(saved_session, document)
                        {
                            this.sessions.refresh_document_baseline(this.session_id, latest);
                        }
                        this.awaiting_create_as_new = false;
                        this.clear_sync_issue();
                        this.close_window(cx);
                    }
                }
                AppEvent::DocumentSaveFailed { session: failed_session, error } => {
                    if session.session_key == *failed_session
                        && matches!(session.target, EditorSessionTarget::Document { .. })
                    {
                        this.awaiting_create_as_new = false;
                        this.clear_sync_issue();
                        this.set_notice(true, format!("Save failed: {error}"));
                        cx.notify();
                    }
                }
                AppEvent::DocumentInserted => {
                    if matches!(session.target, EditorSessionTarget::Insert)
                        || this.awaiting_create_as_new
                    {
                        this.awaiting_create_as_new = false;
                        this.clear_sync_issue();
                        this.close_window(cx);
                    }
                }
                AppEvent::DocumentInsertFailed { error } => {
                    if matches!(session.target, EditorSessionTarget::Insert)
                        || this.awaiting_create_as_new
                    {
                        this.awaiting_create_as_new = false;
                        if matches!(session.target, EditorSessionTarget::Document { .. }) {
                            this.sync_issue = Some(SyncIssue::MissingOriginal);
                        } else {
                            this.clear_sync_issue();
                        }
                        this.set_notice(true, format!("Insert failed: {error}"));
                        cx.notify();
                    }
                }
                _ => {}
            }
        }));
        Self {
            state,
            sessions,
            session_id,
            window_handle: None,
            editor_state: None,
            inline_notice: None,
            sync_issue: None,
            pending_editor_content: None,
            awaiting_create_as_new: false,
            _subscriptions: subscriptions,
        }
    }

    fn ensure_editor_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_state.is_some() {
            return;
        }

        let initial_content = self
            .sessions
            .snapshot(self.session_id)
            .map(|session| session.content)
            .unwrap_or_default();
        let editor_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("javascript")
                .line_number(true)
                .searchable(true)
                .soft_wrap(true)
        });

        editor_state.update(cx, |state, cx| {
            state.set_value(initial_content, window, cx);
        });
        let focus = editor_state.read(cx).focus_handle(cx);
        window.focus(&focus);

        let sessions = self.sessions.clone();
        let session_id = self.session_id;
        let input_sub =
            cx.subscribe_in(&editor_state, window, move |_view, state, event, _window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }

                let value = state.read(cx).value().to_string();
                sessions.update_content(session_id, value);
            });
        self._subscriptions.push(input_sub);
        self.editor_state = Some(editor_state);
    }

    fn apply_pending_editor_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(content) = self.pending_editor_content.take() else {
            return;
        };
        if let Some(editor_state) = self.editor_state.clone() {
            editor_state.update(cx, |state, cx| {
                state.set_value(content, window, cx);
            });
        }
    }

    fn set_notice(&mut self, is_error: bool, message: impl Into<String>) {
        self.inline_notice = Some((is_error, message.into()));
    }

    fn set_error(&mut self, message: impl Into<String>) {
        self.clear_sync_issue();
        self.set_notice(true, message);
    }

    fn set_sync_issue(&mut self, issue: SyncIssue, message: impl Into<String>) {
        self.sync_issue = Some(issue);
        self.set_notice(true, message);
    }

    fn clear_sync_issue(&mut self) {
        self.sync_issue = None;
    }

    fn format_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor_state) = self.editor_state.clone() else {
            return;
        };
        let raw = editor_state.read(cx).value().to_string();
        let formatted = match parse_value_from_relaxed_json(&raw) {
            Ok(value) => format_relaxed_json_value(&value),
            Err(err) => {
                self.set_error(format!("Invalid JSON: {err}"));
                return;
            }
        };

        editor_state.update(cx, |state, cx| {
            state.set_value(formatted.clone(), window, cx);
        });
        self.sessions.update_content(self.session_id, formatted);
        self.clear_sync_issue();
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
                self.set_error(format!("Invalid JSON: {err}"));
                return;
            }
        };

        let Some(session) = self.sessions.snapshot(self.session_id) else {
            self.set_error("Editor session is no longer available.");
            return;
        };

        match session.target {
            EditorSessionTarget::Insert => {
                self.awaiting_create_as_new = false;
                self.clear_sync_issue();
                self.set_notice(false, "Inserting...");
                AppCommands::insert_document(self.state.clone(), session.session_key, document, cx);
            }
            EditorSessionTarget::Document { doc_key, original_id, baseline_document } => {
                let original_id = (*original_id).clone();
                let baseline_document = (*baseline_document).clone();
                let Some(updated_id) = document.get("_id").cloned() else {
                    self.set_error("Edited document must keep the original _id.");
                    return;
                };

                if updated_id != original_id {
                    self.set_error(
                        "_id was changed. Editing an existing document cannot change _id.",
                    );
                    return;
                }

                self.check_latest_and_save(
                    session.session_key,
                    doc_key,
                    original_id,
                    baseline_document,
                    document,
                    cx,
                );
            }
        }
    }

    fn check_latest_and_save(
        &mut self,
        session_key: SessionKey,
        doc_key: DocumentKey,
        original_id: Bson,
        baseline_document: Document,
        updated_document: Document,
        cx: &mut Context<Self>,
    ) {
        let local_draft = self.state.read(cx).session_draft(&session_key, &doc_key);
        if let Some(local_draft) = local_draft
            && local_draft != updated_document
        {
            self.set_sync_issue(
                SyncIssue::LocalDraftConflict,
                "Main view has unapplied inline edits for this document. Load inline draft or apply/discard inline changes first.",
            );
            return;
        }

        let (Some(client), manager) = ({
            let state_ref = self.state.read(cx);
            (
                state_ref.active_connection_client(session_key.connection_id),
                state_ref.connection_manager(),
            )
        }) else {
            self.set_error("Connection is no longer active.");
            return;
        };

        self.awaiting_create_as_new = false;
        self.set_notice(false, "Checking latest document...");

        let database = session_key.database.clone();
        let collection = session_key.collection.clone();
        let id_for_task = original_id.clone();
        let task = cx.background_spawn(async move {
            manager.find_document_by_id(&client, &database, &collection, &id_for_task)
        });

        cx.spawn({
            let session_key = session_key.clone();
            let doc_key = doc_key.clone();
            let updated_document = updated_document.clone();
            async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
                let result: Result<Option<Document>, crate::error::Error> = task.await;
                let _ = cx.update(|cx| {
                    let _ = view.update(cx, |this, cx| match result {
                        Ok(Some(current)) => {
                            if current != baseline_document {
                                this.set_sync_issue(
                                    SyncIssue::ConflictChanged,
                                    "Document changed on server. Reload to review latest version before saving.",
                                );
                                cx.notify();
                                return;
                            }

                            this.clear_sync_issue();
                            this.set_notice(false, "Saving...");
                            AppCommands::save_document(
                                this.state.clone(),
                                session_key.clone(),
                                doc_key.clone(),
                                updated_document.clone(),
                                cx,
                            );
                        }
                        Ok(None) => {
                            this.set_sync_issue(
                                SyncIssue::MissingOriginal,
                                "Original document was deleted. Reload or create current JSON as new.",
                            );
                        }
                        Err(err) => {
                            this.set_sync_issue(
                                SyncIssue::ReloadFailed,
                                format!("Failed to reload latest document: {err}"),
                            );
                        }
                    });
                });
            }
        })
        .detach();
    }

    fn load_inline_draft(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.snapshot(self.session_id) else {
            self.set_error("Editor session is no longer available.");
            return;
        };

        let (session_key, doc_key) = match session.target {
            EditorSessionTarget::Document { doc_key, .. } => (session.session_key, doc_key),
            EditorSessionTarget::Insert => {
                self.set_error("Inline draft is only available when editing an existing document.");
                return;
            }
        };

        let Some(draft) = self.state.read(cx).session_draft(&session_key, &doc_key) else {
            self.set_error("No inline draft is available for this document.");
            return;
        };

        let content = document_to_shell_string(&draft);
        self.sessions.update_content(self.session_id, content.clone());
        self.pending_editor_content = Some(content);
        self.clear_sync_issue();
        self.set_notice(false, "Loaded inline draft from main view.");
        cx.notify();
    }

    fn close_window(&mut self, cx: &mut Context<Self>) {
        self.sessions.close(self.session_id);
        if let Some(handle) = self.window_handle {
            let _ = handle.update(cx, |_view, window, _cx| {
                window.remove_window();
            });
        }
    }

    fn reload_document(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.snapshot(self.session_id) else {
            self.set_error("Editor session is no longer available.");
            return;
        };

        let (session_key, original_id) = match session.target {
            EditorSessionTarget::Document { original_id, .. } => {
                (session.session_key, (*original_id).clone())
            }
            EditorSessionTarget::Insert => {
                self.set_error("Reload is only available when editing an existing document.");
                return;
            }
        };

        let (Some(client), manager) = ({
            let state_ref = self.state.read(cx);
            (
                state_ref.active_connection_client(session_key.connection_id),
                state_ref.connection_manager(),
            )
        }) else {
            self.set_error("Connection is no longer active.");
            return;
        };

        self.set_notice(false, "Reloading latest document...");

        let database = session_key.database.clone();
        let collection = session_key.collection.clone();
        let id_for_task = original_id.clone();
        let task = cx.background_spawn(async move {
            manager.find_document_by_id(&client, &database, &collection, &id_for_task)
        });

        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: Result<Option<Document>, crate::error::Error> = task.await;
            let _ = cx.update(|cx| {
                let _ = view.update(cx, |this, cx| match result {
                    Ok(Some(current)) => {
                        let content = document_to_shell_string(&current);
                        this.sessions
                            .refresh_document_baseline(this.session_id, current.clone());
                        this.sessions.update_content(this.session_id, content.clone());
                        this.pending_editor_content = Some(content);
                        this.clear_sync_issue();
                        this.set_notice(false, "Reloaded latest document.");
                        cx.notify();
                    }
                    Ok(None) => {
                        this.set_sync_issue(
                            SyncIssue::MissingOriginal,
                            "Original document no longer exists. Create current JSON as new if needed.",
                        );
                    }
                    Err(err) => {
                        this.set_sync_issue(
                            SyncIssue::ReloadFailed,
                            format!("Failed to reload latest document: {err}"),
                        );
                    }
                });
            });
        })
        .detach();
    }

    fn create_as_new(&mut self, cx: &mut Context<Self>) {
        let Some(editor_state) = self.editor_state.clone() else {
            return;
        };
        let raw = editor_state.read(cx).value().to_string();
        let document = match parse_document_from_json(&raw) {
            Ok(document) => document,
            Err(err) => {
                self.set_error(format!("Invalid JSON: {err}"));
                return;
            }
        };

        let Some(session) = self.sessions.snapshot(self.session_id) else {
            self.set_error("Editor session is no longer available.");
            return;
        };

        if !matches!(session.target, EditorSessionTarget::Document { .. }) {
            self.set_error("Create as new is only available when editing an existing document.");
            return;
        }

        let mut document = document;
        document.insert("_id", ObjectId::new());

        self.awaiting_create_as_new = true;
        self.set_notice(false, "Creating as new document...");
        AppCommands::insert_document(self.state.clone(), session.session_key, document, cx);
    }
}

impl Render for DetachedJsonEditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.window_handle.is_none() {
            self.window_handle = Some(window.window_handle());
        }
        self.ensure_editor_state(window, cx);
        self.apply_pending_editor_content(window, cx);

        let Some(session) = self.sessions.snapshot(self.session_id) else {
            return div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("This JSON editor session is unavailable.");
        };

        let subtitle =
            format!("{}/{}", session.session_key.database, session.session_key.collection);
        let mode_label = match &session.target {
            EditorSessionTarget::Insert => "Insert document",
            EditorSessionTarget::Document { .. } => "Edit document",
        };
        let primary_label = match &session.target {
            EditorSessionTarget::Insert => "Insert & Close",
            EditorSessionTarget::Document { .. } => "Save & Close",
        };
        let notice = self.inline_notice.clone();
        let sync_issue = self.sync_issue;

        let Some(editor) = self.editor_state.clone() else {
            return div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Editor is initializing...");
        };

        let view = cx.entity();
        div()
            .key_context("JsonEditorWindow")
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &CloseEditorWindow, window, cx| {
                cx.stop_propagation();
                this.sessions.close(this.session_id);
                window.remove_window();
            }))
            .on_key_down({
                let view = view.clone();
                move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                    let key = event.keystroke.key.to_ascii_lowercase();
                    let modifiers = event.keystroke.modifiers;
                    let cmd_or_ctrl = modifiers.secondary() || modifiers.control;
                    if cmd_or_ctrl && !modifiers.alt && !modifiers.shift && key == "w" {
                        cx.stop_propagation();
                        view.update(cx, |this, _cx| {
                            this.sessions.close(this.session_id);
                        });
                        window.remove_window();
                    } else if cmd_or_ctrl && !modifiers.alt && !modifiers.shift && key == "n" {
                        // Prevent spawning additional app/editor windows from detached editor focus.
                        cx.stop_propagation();
                    } else if cmd_or_ctrl && key == "s" {
                        cx.stop_propagation();
                        view.update(cx, |this, cx| this.save_or_insert(cx));
                    } else if cmd_or_ctrl && modifiers.shift && key == "f" {
                        cx.stop_propagation();
                        view.update(cx, |this, cx| this.format_json(window, cx));
                    }
                }
            })
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
                                    .child(format!("{}  •  {}", subtitle, session.title())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                Button::new("json-editor-window-format")
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
                                Button::new("json-editor-window-save")
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
                let mut row = div()
                    .px(spacing::md())
                    .pt(px(6.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .text_xs()
                            .text_color(if is_error {
                                cx.theme().danger
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(message),
                    );

                if is_error {
                    if matches!(
                        sync_issue,
                        Some(SyncIssue::ConflictChanged)
                            | Some(SyncIssue::MissingOriginal)
                            | Some(SyncIssue::ReloadFailed)
                    ) {
                        row = row.child(
                            Button::new("json-editor-window-reload")
                                .compact()
                                .label("Reload")
                                .on_click({
                                    let view = view.clone();
                                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                        view.update(cx, |this, cx| this.reload_document(cx));
                                    }
                                }),
                        );
                    }

                    if matches!(sync_issue, Some(SyncIssue::LocalDraftConflict)) {
                        row = row.child(
                            Button::new("json-editor-window-load-inline-draft")
                                .compact()
                                .label("Load Inline Draft")
                                .on_click({
                                    let view = view.clone();
                                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                        view.update(cx, |this, cx| this.load_inline_draft(cx));
                                    }
                                }),
                        );
                    }

                    if matches!(sync_issue, Some(SyncIssue::MissingOriginal)) {
                        row = row.child(
                            Button::new("json-editor-window-create-new")
                                .compact()
                                .label("Create as New")
                                .on_click({
                                    let view = view.clone();
                                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                        view.update(cx, |this, cx| this.create_as_new(cx));
                                    }
                                }),
                        );
                    }
                }

                this.child(row)
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

pub fn open_document_json_editor_window(
    state: Entity<AppState>,
    session_key: SessionKey,
    doc_key: DocumentKey,
    cx: &mut App,
) {
    if state.read(cx).active_connection_by_id(session_key.connection_id).is_none() {
        return;
    }

    state.update(cx, |state, cx| {
        if state.promote_preview_collection_tab(&session_key) {
            cx.notify();
        }
    });

    let sessions = state.read(cx).editor_sessions();
    if let Some(existing_id) = sessions.find_any_document_session() {
        if focus_existing_window(&sessions, existing_id, cx) {
            return;
        }
        sessions.close(existing_id);
    }

    let Some(document) = state.read(cx).session_draft_or_document(&session_key, &doc_key) else {
        log::warn!("Could not open JSON editor: document is no longer available");
        return;
    };

    let original_id = document
        .get("_id")
        .cloned()
        .or_else(|| parse_bson_from_relaxed_json(doc_key.as_str()).ok());
    let Some(original_id) = original_id else {
        log::warn!("Could not open JSON editor: failed to resolve original _id");
        return;
    };

    let session_id = sessions.create_document_session(
        session_key,
        doc_key,
        original_id,
        document.clone(),
        document_to_shell_string(&document),
    );
    match open_detached_json_editor_window(state, sessions.clone(), session_id, cx) {
        Ok(window) => {
            focus_window_handle(&window, cx);
            sessions.register_window(session_id, window.into());
        }
        Err(err) => {
            sessions.close(session_id);
            log::error!("Failed to open JSON editor window: {err}");
        }
    }
}

pub fn open_insert_json_editor_window(
    state: Entity<AppState>,
    session_key: SessionKey,
    cx: &mut App,
) {
    if state.read(cx).active_connection_by_id(session_key.connection_id).is_none() {
        return;
    }

    state.update(cx, |state, cx| {
        if state.promote_preview_collection_tab(&session_key) {
            cx.notify();
        }
    });

    let sessions = state.read(cx).editor_sessions();
    if let Some(existing_id) = sessions.find_any_insert_session() {
        if focus_existing_window(&sessions, existing_id, cx) {
            return;
        }
        sessions.close(existing_id);
    }

    let session_id = sessions.create_insert_session(session_key, "{}".to_string());
    match open_detached_json_editor_window(state, sessions.clone(), session_id, cx) {
        Ok(window) => {
            focus_window_handle(&window, cx);
            sessions.register_window(session_id, window.into());
        }
        Err(err) => {
            sessions.close(session_id);
            log::error!("Failed to open JSON editor window: {err}");
        }
    }
}

fn focus_existing_window(
    sessions: &EditorSessionStore,
    session_id: EditorSessionId,
    cx: &mut App,
) -> bool {
    let Some(handle) = sessions.window_handle(session_id) else {
        return false;
    };

    handle
        .update(cx, |_view, window, cx| {
            window.activate_window();
            cx.activate(true);
            window.defer(cx, |window, cx| {
                window.activate_window();
                cx.activate(true);
            });
        })
        .is_ok()
}

fn focus_window_handle(window: &WindowHandle<Root>, cx: &mut App) {
    let _ = window.update(cx, |_view, window, cx| {
        window.activate_window();
        cx.activate(true);
        window.defer(cx, |window, cx| {
            window.activate_window();
            cx.activate(true);
        });
    });
}

fn open_detached_json_editor_window(
    state: Entity<AppState>,
    sessions: EditorSessionStore,
    session_id: EditorSessionId,
    cx: &mut App,
) -> gpui::Result<WindowHandle<Root>> {
    let title = sessions
        .snapshot(session_id)
        .map(|session| session.title())
        .unwrap_or_else(|| "JSON Editor".to_string());
    let title: SharedString = format!("{title} - OpenMango").into();
    let bounds =
        Bounds::centered(None, size(px(DETACHED_WINDOW_WIDTH), px(DETACHED_WINDOW_HEIGHT)), cx);

    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_background: WindowBackgroundAppearance::Opaque,
            titlebar: Some(TitlebarOptions { title: Some(title), ..Default::default() }),
            ..Default::default()
        },
        move |window, cx| {
            let close_sessions = sessions.clone();
            window.on_window_should_close(cx, move |_window, cx| {
                close_sessions.close(session_id);
                if cx.windows().len() == 1 {
                    cx.quit();
                }
                true
            });

            let view_state = state.clone();
            let view_sessions = sessions.clone();
            let editor_view =
                cx.new(|cx| DetachedJsonEditorView::new(view_state, view_sessions, session_id, cx));
            cx.new(|cx| Root::new(editor_view, window, cx))
        },
    )
}
