//! Shared transitions used by the toolbar, pagination, and keyboard actions.

use gpui_kit::*;

use crate::components::request_unsaved_action;
use crate::state::{AppCommands, AppState, DocumentViewMode, SessionKey, UnsavedScope};

use super::CollectionView;

impl CollectionView {
    pub(super) fn save_selected_documents(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.finish_document_edit(cx) {
            return false;
        }
        let Some(key) = self.view_model.current_session() else { return false };
        let documents = self
            .state
            .read(cx)
            .session(&key)
            .map(|session| {
                session
                    .data
                    .items
                    .iter()
                    .filter(|item| session.view.selected_docs.contains(&item.key))
                    .filter_map(|item| {
                        session
                            .view
                            .drafts
                            .get(&item.key)
                            .cloned()
                            .map(|doc| (item.key.clone(), doc))
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
                format!("Save {count} selected document(s)"),
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

    pub(super) fn discard_selected_documents(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.view_model.current_session() else { return };
        if self
            .state
            .read(cx)
            .session_view(&key)
            .is_some_and(|view| !view.saving_documents.is_empty())
        {
            return;
        }
        let selected = self
            .state
            .read(cx)
            .session(&key)
            .map(|session| {
                session
                    .view
                    .selected_docs
                    .iter()
                    .filter(|doc| session.view.dirty.contains(*doc))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if selected.is_empty() {
            return;
        }
        let view = cx.entity();
        crate::components::open_confirm_dialog(
            window,
            cx,
            "Discard document changes",
            format!("Discard local edits to {} selected document(s)?", selected.len()),
            "Discard",
            true,
            move |_, cx| {
                view.update(cx, |this, cx| {
                    this.view_model.clear_inline_edit();
                    this.state.update(cx, |state, cx| {
                        for document in selected {
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
