//! Shared transitions used by the toolbar, pagination, and keyboard actions.

use gpui_kit::*;
use mongodb::bson::Bson;

use crate::bson::{DocumentKey, PathSegment, is_editable_value, path_to_id};
use crate::components::request_unsaved_action;
use crate::state::{AppCommands, AppState, DocumentViewMode, SessionKey, UnsavedScope};

use super::CollectionView;
use super::dialogs::property_dialog::PropertyActionDialog;
use super::node_meta::NodeMeta;

/// Whether a table cell edits this value in its one-line box. Booleans flip instead, and what a
/// single line cannot hold or would only allow "null" in goes to the value dialog.
fn edits_in_cell(value: &Bson, path: &[PathSegment]) -> bool {
    is_editable_value(value, path)
        && !matches!(value, Bson::Null | Bson::Boolean(_))
        && !matches!(value, Bson::String(text) if text.contains(['\n', '\r']))
}

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
        let preview = documents.iter().fold(
            crate::components::WritePreview::new(&key.database),
            |preview, (doc_key, draft)| {
                let loaded = state.read(cx).document_edit_baseline(&key, doc_key);
                match (loaded, draft.get("_id")) {
                    (Some(loaded), Some(id)) => {
                        preview.save_edit(&key.collection, id.clone(), &loaded, draft)
                    }
                    _ => preview,
                }
            },
        );
        crate::components::request_connection_write(
            state.clone(),
            crate::components::WriteRequest::new(
                key.connection_id,
                key.namespace(),
                format!("Save {count} document(s)"),
                None,
            )
            .for_writes(count)
            .with_preview(preview),
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

    /// Double-click on a table cell. A value that fits one line is edited in the cell, like a
    /// spreadsheet, with a calendar under dates; a boolean flips. An object, an array, a
    /// multi-line string, null, any other type, or a field this document lacks goes to the value
    /// dialog, which edits just that field as JSON or as a chosen type. `_id` cannot change, so
    /// it opens the document instead.
    pub(crate) fn edit_table_cell(
        &mut self,
        doc_key: DocumentKey,
        field: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.view_model.current_session() else { return };
        if field == "_id" {
            self.open_document_json(key, doc_key, window, cx);
            return;
        }
        // Say why before any editor opens; the dialog would otherwise refuse only on submit.
        if let Some(reason) = self.state.read(cx).document_field_edit_restriction(&key) {
            self.state.update(cx, |state, cx| {
                state.set_status_message(Some(crate::state::StatusMessage::error(reason)));
                cx.notify();
            });
            return;
        }
        let value = self
            .state
            .read(cx)
            .session_draft_or_document(&key, &doc_key)
            .and_then(|document| document.get(&field).cloned());
        let path = vec![PathSegment::Key(field.clone())];
        if let Some(Bson::Boolean(flag)) = value {
            // Staged like any edit: the cell shows it changed, and flipping again undoes it.
            let flipped = Bson::Boolean(!flag);
            if self.view_model.update_draft_value(&self.state, &doc_key, &path, flipped, cx) {
                self.view_model.rebuild_tree(&self.state, cx);
                self.view_model.invalidate_table();
            }
            cx.notify();
            return;
        }
        let in_cell = value.as_ref().is_some_and(|value| edits_in_cell(value, &path));
        let is_date = matches!(value, Some(Bson::DateTime(_)));
        // Only what editing reads is filled in; the cell draws itself.
        let meta = NodeMeta {
            key_label: field.clone(),
            value_label: String::new(),
            value_color: Hsla::default(),
            type_label: String::new(),
            is_folder: matches!(value, Some(Bson::Document(_) | Bson::Array(_))),
            is_editable: in_cell,
            is_dirty: false,
            has_details: false,
            doc_key: doc_key.clone(),
            path,
            value,
        };
        if !in_cell {
            PropertyActionDialog::open_edit_value(self.state.clone(), key, meta, true, window, cx);
            cx.notify();
            return;
        }
        let node_id = path_to_id(&doc_key, &meta.path);
        self.view_model.begin_inline_edit(node_id, &meta, window, &self.state, cx);
        if let Some(input) = self.view_model.inline_editor_for(&doc_key, &field).cloned() {
            // Typing replaces the value; an arrow key keeps it to edit.
            input.update(cx, |input, cx| input.select_all(window, cx));
            if is_date {
                self.view_model.open_cell_calendar(window, cx);
            }
        }
        cx.notify();
    }

    /// Tab and Shift+Tab while a table cell is edited: keep the change and edit the next or
    /// previous cell of the row that edits in place. Cells that flip or open a dialog are
    /// skipped, so Tab never toggles a value or opens a window. Returns whether it handled the key.
    pub(super) fn tab_table_cell(
        &mut self,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((doc_key, field)) = self.view_model.editing_field() else { return false };
        let Some(key) = self.view_model.current_session() else { return false };
        let Some(table) = self.view_model.table_state().cloned() else { return false };
        if !self.finish_document_edit(cx) {
            // An invalid value stays open with its error.
            return true;
        }
        let columns: Vec<String> =
            table.read(cx).delegate().table_cols.columns.iter().map(|c| c.key.clone()).collect();
        let document = self.state.read(cx).session_draft_or_document(&key, &doc_key);
        let edits_here = |col_ix: &usize| {
            let column = &columns[*col_ix];
            let path = [PathSegment::Key(column.clone())];
            document
                .as_ref()
                .and_then(|document| document.get(column))
                .is_some_and(|value| edits_in_cell(value, &path))
        };
        let next = columns.iter().position(|column| column == &field).and_then(|at| {
            if backwards {
                (0..at).rev().find(edits_here)
            } else {
                (at + 1..columns.len()).find(edits_here)
            }
        });
        match next {
            Some(col_ix) => {
                table.update(cx, |table, cx| table.scroll_to_col(col_ix, cx));
                self.edit_table_cell(doc_key, columns[col_ix].clone(), window, cx);
            }
            None => self.focus_documents(window, cx),
        }
        true
    }

    /// Opens one document in the JSON view. Enter on a table row and double-click on its `_id`
    /// land here.
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
