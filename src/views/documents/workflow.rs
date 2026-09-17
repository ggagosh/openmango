//! Shared transitions used by the toolbar, pagination, and keyboard actions.

use gpui_kit::*;

use crate::components::request_unsaved_action;
use crate::state::{AppCommands, AppState, DocumentViewMode, SessionKey, UnsavedScope};

use super::CollectionView;

impl CollectionView {
    /// Saves every unsaved document in this tab, the ones the title row counts.
    pub(super) fn save_documents(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.finish_document_edit(cx) {
            return false;
        }
        let Some(key) = self.view_model.current_session() else { return false };
        let documents = self
            .state
            .read(cx)
            .session_view(&key)
            .map(|view| {
                view.dirty
                    .iter()
                    .filter(|doc| !view.saving_documents.contains(*doc))
                    .filter_map(|doc| {
                        view.drafts.get(doc).map(|draft| (doc.clone(), draft.clone()))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if documents.is_empty() {
            return false;
        }
        let state = self.state.clone();
        let count = documents.len();
        crate::components::request_connection_write(
            state.clone(),
            crate::components::WriteRequest::new(
                key.connection_id,
                key.namespace(),
                format!("Save {count} document(s)"),
                None,
            )
            .for_writes(count),
            window,
            cx,
            move |_, cx| {
                for (doc_key, document) in documents {
                    AppCommands::save_document(state.clone(), key.clone(), doc_key, document, cx);
                }
            },
        );
        true
    }

    /// Discards unsaved edits in this tab, or only in the selected documents.
    pub(super) fn discard_documents(
        &mut self,
        selected_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.view_model.current_session() else { return };
        let documents = self
            .state
            .read(cx)
            .session_view(&key)
            .filter(|view| view.saving_documents.is_empty())
            .map(|view| {
                view.dirty
                    .iter()
                    .filter(|doc| !selected_only || view.selected_docs.contains(*doc))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if documents.is_empty() {
            return;
        }
        let view = cx.entity();
        crate::components::open_confirm_dialog(
            window,
            cx,
            "Discard document changes",
            format!("Discard local edits to {} document(s)?", documents.len()),
            "Discard",
            true,
            move |_, cx| {
                view.update(cx, |this, cx| {
                    this.view_model.clear_inline_edit();
                    this.state.update(cx, |state, cx| {
                        for document in documents {
                            state.clear_draft(&key, &document);
                        }
                        state.set_invalid_inline_edit(key, false);
                        cx.notify();
                    });
                    this.view_model.rebuild_tree(&this.state, cx);
                    this.view_model.invalidate_table();
                    this.view_model.sync_dirty_state(&this.state, cx);
                    cx.notify();
                });
            },
        );
    }

    pub(super) fn finish_document_edit(&mut self, cx: &mut Context<Self>) -> bool {
        self.view_model.commit_inline_edit(&self.state, cx);
        self.view_model.editing_node_id().is_none()
    }

    /// Opens one document in the JSON view. Double-click and Enter on a table row both land here.
    pub(super) fn open_document_json(
        &mut self,
        key: SessionKey,
        doc_key: crate::bson::DocumentKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |state, cx| {
            state.select_single_doc(&key, doc_key.clone(), crate::bson::doc_root_id(&doc_key));
            cx.notify();
        });
        self.change_document_view(key, DocumentViewMode::Json, window, cx);
    }

    pub(super) fn change_document_view(
        &mut self,
        key: SessionKey,
        mode: DocumentViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.finish_document_edit(cx) {
            return;
        }
        if self.state.read(cx).session_view_mode(&key) == mode {
            return;
        }
        if mode == DocumentViewMode::Json
            && self
                .state
                .read(cx)
                .session_view(&key)
                .is_some_and(|view| !view.saving_documents.is_empty())
        {
            return;
        }
        let state = self.state.clone();
        let sessions = state.read(cx).editor_sessions();
        let editor = self
            .json_document
            .as_ref()
            .filter(|(session, _, id, _)| session == &key && sessions.window_handle(*id).is_none())
            .map(|(_, _, id, _)| *id);
        let change = move |_: &mut Window, cx: &mut App| {
            if let Some(editor) = editor {
                sessions.close(editor);
            }
            state.update(cx, |state, cx| {
                if mode == DocumentViewMode::Json {
                    let document = state.session_selected_doc(&key).or_else(|| {
                        state
                            .session_data(&key)
                            .and_then(|data| data.items.first().map(|item| item.key.clone()))
                    });
                    if let Some(document) = document {
                        state.select_single_doc(
                            &key,
                            document.clone(),
                            crate::bson::doc_root_id(&document),
                        );
                    }
                }
                state.set_view_mode(&key, mode);
                cx.notify();
            });
        };
        if let Some(editor) = editor {
            request_unsaved_action(
                self.state.clone(),
                UnsavedScope::Editor(editor),
                window,
                cx,
                change,
            );
        } else {
            change(window, cx);
        }
    }

    pub(super) fn reload_document_page(
        view: Entity<Self>,
        state: Entity<AppState>,
        key: SessionKey,
        window: &mut Window,
        cx: &mut App,
        change: impl FnOnce(&mut AppState, &SessionKey) + 'static,
    ) {
        if !view.update(cx, |this, cx| this.finish_document_edit(cx)) {
            return;
        }
        request_unsaved_action(
            state.clone(),
            UnsavedScope::Preview(key.clone()),
            window,
            cx,
            move |_, cx| {
                view.update(cx, |this, cx| {
                    if this.json_document.as_ref().is_some_and(|(session, _, _, _)| session == &key)
                        && let Some((_, _, editor, _)) = this.json_document.take()
                    {
                        let sessions = this.state.read(cx).editor_sessions();
                        if sessions.window_handle(editor).is_none() {
                            sessions.close(editor);
                        }
                    }
                    if this.view_model.is_current_session(&key) {
                        this.view_model.clear_inline_edit();
                        this.view_model.invalidate_table();
                    }
                });
                state.update(cx, |state, cx| {
                    change(state, &key);
                    cx.notify();
                });
                AppCommands::load_documents_for_session(state, key, cx);
            },
        );
    }
}
