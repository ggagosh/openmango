use gpui_kit::*;
use mongodb::bson::{Bson, Document, doc, oid::ObjectId};

use crate::bson::{
    PathSegment, document_to_json_string, format_bson_for_clipboard, get_bson_at_path,
};
use crate::components::{WriteConfirmation, open_confirm_dialog, request_connection_write};
use crate::keyboard::{
    AddElement, AddField, ClearAggregationStage, CloseSearch, CopyAs, CopyAsCsv, CopyAsJson,
    CopyAsJsonLines, CopyAsMarkdown, CopyAsTsv, CopyDocumentJson, CopyKey, CopyValue, CreateIndex,
    DeleteAggregationStage, DeleteCollection, DeleteDocument, DiscardDocumentChanges,
    DuplicateAggregationStage, DuplicateDocument, EditDocumentJson, EditValueType, FindInResults,
    FormatAggregationStage, InsertDocument, MoveAggregationStageDown, MoveAggregationStageUp,
    NextSearchMatch, PasteDocuments, PrevSearchMatch, RemoveMatchingValues, RemoveSelectedField,
    RenameField, RunAggregation, SaveDocument, SelectNextAggregationStage,
    SelectPrevAggregationStage, ShowAggregationSubview, ShowDocumentsSubview, ShowHistorySubview,
    ShowIndexesSubview, ShowSchemaSubview, ShowStatsSubview, ToggleAggregationStageEnabled,
};
use crate::state::{AppCommands, CollectionSubview, DocumentViewMode, StatusMessage};

use super::export::{CopyFormat, ExportScope, ViewExportSnapshot, render_to_clipboard};

use super::CollectionView;
use super::dialogs::index_create::IndexCreateDialog;
use super::dialogs::property_dialog::PropertyActionDialog;
use super::node_meta::NodeMeta;
use super::tree::tree_content::paste_documents_from_clipboard;

impl CollectionView {
    pub(super) fn bind_root_actions(&mut self, root: Div, cx: &mut Context<Self>) -> Div {
        root.on_action(cx.listener(|this, _: &FindInResults, window, cx| {
            if this.search_visible {
                return;
            }
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let subview = this
                .state
                .read(cx)
                .session_subview(&session_key)
                .unwrap_or(CollectionSubview::Documents);
            if subview != CollectionSubview::Documents {
                return;
            }
            this.show_search_bar(window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &CloseSearch, window, cx| {
            if !this.search_visible {
                return;
            }
            this.close_search(window, cx);
            cx.notify();
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &NextSearchMatch, _window, cx| {
            this.next_match(cx);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &PrevSearchMatch, _window, cx| {
            this.prev_match(cx);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &InsertDocument, window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            CollectionView::open_insert_document_json_editor(
                this.state.clone(),
                session_key,
                window,
                cx,
            );
        }))
        .on_action(cx.listener(|this, _: &CreateIndex, window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let subview = this
                .state
                .read(cx)
                .session_subview(&session_key)
                .unwrap_or(CollectionSubview::Documents);
            if subview != CollectionSubview::Indexes {
                return;
            }
            IndexCreateDialog::open(this.state.clone(), session_key, window, cx);
        }))
        .on_action(cx.listener(|this, _: &EditDocumentJson, window, cx| {
            let Some((session_key, _doc_key)) = this.selected_doc_key_for_current_session(cx)
            else {
                return;
            };
            let selected_count = this
                .state
                .read(cx)
                .session(&session_key)
                .map(|s| s.view.selected_docs.len())
                .unwrap_or(0);
            if selected_count > 1 {
                return;
            }
            this.change_document_view(session_key, DocumentViewMode::Json, window, cx);
        }))
        .on_action(cx.listener(|this, _: &DuplicateDocument, _window, cx| {
            let Some((session_key, _doc_key, doc)) = this.selected_document_for_current_session(cx)
            else {
                return;
            };
            let selected_count = this
                .state
                .read(cx)
                .session(&session_key)
                .map(|s| s.view.selected_docs.len())
                .unwrap_or(0);
            if selected_count > 1 {
                return;
            }
            let mut new_doc = doc.clone();
            new_doc.insert("_id", ObjectId::new());
            crate::views::json_editor_detached::open_insert_json_editor_with_content(
                this.state.clone(),
                session_key,
                document_to_json_string(&new_doc),
                cx,
            );
        }))
        .on_action(cx.listener(|this, _: &DeleteDocument, window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let selected_docs: Vec<_> = {
                let state_ref = this.state.read(cx);
                let Some(session) = state_ref.session(&session_key) else {
                    return;
                };
                session
                    .data
                    .items
                    .iter()
                    .filter(|item| session.view.selected_docs.contains(&item.key))
                    .map(|item| item.key.clone())
                    .collect()
            };
            if selected_docs.is_empty() {
                return;
            }
            if selected_docs.len() == 1 {
                let doc_key = selected_docs.into_iter().next().unwrap();
                let message = format!("Delete document {}? This cannot be undone.", doc_key);
                let state = this.state.clone();
                let state_for_write = state.clone();
                request_connection_write(
                    state,
                    crate::components::WriteRequest::new(
                        session_key.connection_id,
                        session_key.namespace(),
                        "Delete a document",
                        Some(WriteConfirmation {
                            title: "Delete document".into(),
                            message,
                            confirm_label: "Delete".into(),
                            destructive: true,
                        }),
                    ),
                    window,
                    cx,
                    move |_window, cx| {
                        AppCommands::delete_document(state_for_write, session_key, doc_key, cx);
                    },
                );
            } else {
                let ids: Vec<Bson> = {
                    let state_ref = this.state.read(cx);
                    selected_docs
                        .iter()
                        .filter_map(|dk| {
                            state_ref
                                .document_for_key(&session_key, dk)
                                .and_then(|d| d.get("_id").cloned())
                        })
                        .collect()
                };
                if ids.is_empty() {
                    return;
                }
                let affected_count = ids.len();
                let filter = doc! { "_id": { "$in": ids } };
                let recovery = " This cannot be undone.";
                let message = format!("Delete {affected_count} documents?{recovery}");
                let state = this.state.clone();
                let state_for_write = state.clone();
                request_connection_write(
                    state,
                    crate::components::WriteRequest::new(
                        session_key.connection_id,
                        session_key.namespace(),
                        format!("Delete {affected_count} documents"),
                        Some(WriteConfirmation {
                            title: "Delete documents".into(),
                            message,
                            confirm_label: "Delete".into(),
                            destructive: true,
                        }),
                    ),
                    window,
                    cx,
                    move |_window, cx| {
                        AppCommands::delete_documents_by_filter(
                            state_for_write,
                            session_key,
                            filter,
                            cx,
                        );
                    },
                );
            }
        }))
        .on_action(cx.listener(|this, _: &DeleteCollection, window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let message =
                format!("Drop collection {}? This cannot be undone.", session_key.collection);
            let state = this.state.clone();
            let state_for_write = state.clone();
            request_connection_write(
                state,
                crate::components::WriteRequest::new(
                    session_key.connection_id,
                    session_key.namespace(),
                    "Drop a collection",
                    Some(WriteConfirmation {
                        title: "Drop collection".into(),
                        message,
                        confirm_label: "Drop".into(),
                        destructive: true,
                    }),
                ),
                window,
                cx,
                move |_window, cx| {
                    AppCommands::drop_collection(
                        state_for_write,
                        session_key.connection_id,
                        session_key.database,
                        session_key.collection,
                        cx,
                    );
                },
            );
        }))
        .on_action(cx.listener(|this, _: &PasteDocuments, window, cx| {
            if let Some((session_key, meta)) = this.selected_property_context(cx) {
                if !this.finish_document_edit(cx) {
                    return;
                }
                let result = (|| {
                    if let Some(reason) =
                        this.state.read(cx).document_field_edit_restriction(&session_key)
                    {
                        return Err(reason.to_string());
                    }
                    if matches!(meta.path.first(), Some(PathSegment::Key(key)) if key == "_id") {
                        return Err("The document _id cannot be changed.".into());
                    }
                    let text = cx
                        .read_from_clipboard()
                        .and_then(|item| item.text())
                        .ok_or("Clipboard has no text.")?;
                    let document = this
                        .resolve_document(&session_key, &meta.doc_key, cx)
                        .ok_or("Document is no longer available.")?;
                    let original = get_bson_at_path(&document, &meta.path)
                        .ok_or("Field is no longer available.")?;
                    crate::bson::parse_edited_value(original, &text)
                })();
                match result {
                    Ok(value) => {
                        this.view_model.update_draft_value(
                            &this.state,
                            &meta.doc_key,
                            &meta.path,
                            value,
                            cx,
                        );
                        this.view_model.rebuild_tree(&this.state, cx);
                        this.view_model.invalidate_table();
                        cx.notify();
                    }
                    Err(error) => this.state.update(cx, |state, cx| {
                        state.set_status_message(Some(StatusMessage::error(error)));
                        cx.notify();
                    }),
                }
                return;
            }
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            paste_documents_from_clipboard(this.state.clone(), session_key, window, cx);
        }))
        .on_action(cx.listener(|this, _: &CopyDocumentJson, _window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let selected_docs: Vec<_> = {
                let state_ref = this.state.read(cx);
                let Some(session) = state_ref.session(&session_key) else {
                    return;
                };
                session
                    .data
                    .items
                    .iter()
                    .filter(|item| session.view.selected_docs.contains(&item.key))
                    .map(|item| item.key.clone())
                    .collect()
            };
            if selected_docs.is_empty() {
                return;
            }
            if selected_docs.len() == 1 {
                let doc_key = &selected_docs[0];
                if let Some(doc) = this.resolve_document(&session_key, doc_key, cx) {
                    let json = document_to_json_string(&doc);
                    cx.write_to_clipboard(ClipboardItem::new_string(json));
                }
            } else {
                // Gather the owned documents on the main thread (cheap clone),
                // then serialize off-thread so a large multi-selection copy
                // doesn't block the UI.
                let docs: Vec<Document> = {
                    let state_ref = this.state.read(cx);
                    selected_docs
                        .iter()
                        .filter_map(|dk| state_ref.session_draft_or_document(&session_key, dk))
                        .collect()
                };
                let task = cx.background_spawn(async move {
                    let parts: Vec<String> = docs.iter().map(document_to_json_string).collect();
                    format!("[{}]", parts.join(",\n"))
                });
                cx.spawn(async move |_this, cx: &mut gpui_kit::AsyncApp| {
                    let json = task.await;
                    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(json)));
                })
                .detach();
            }
        }))
        .on_action(cx.listener(|this, _: &SaveDocument, window, cx| {
            this.save_selected_documents(window, cx);
        }))
        .on_action(cx.listener(|this, _: &EditValueType, window, cx| {
            let Some((session_key, meta)) = this.selected_property_context(cx) else {
                return;
            };
            let flags = property_flags(&meta);
            if !flags.can_edit_value {
                return;
            }
            PropertyActionDialog::open_edit_value(
                this.state.clone(),
                session_key,
                meta,
                flags.allow_bulk,
                window,
                cx,
            );
        }))
        .on_action(cx.listener(|this, _: &RenameField, window, cx| {
            let Some((session_key, meta)) = this.selected_property_context(cx) else {
                return;
            };
            let flags = property_flags(&meta);
            if !flags.can_rename_field {
                return;
            }
            PropertyActionDialog::open_rename_field(
                this.state.clone(),
                session_key,
                meta,
                flags.allow_bulk,
                window,
                cx,
            );
        }))
        .on_action(cx.listener(|this, _: &RemoveSelectedField, window, cx| {
            let Some((session_key, meta)) = this.selected_property_context(cx) else {
                return;
            };
            let flags = property_flags(&meta);
            if flags.can_remove_element {
                PropertyActionDialog::open_remove_matching(
                    this.state.clone(),
                    session_key,
                    meta,
                    false,
                    window,
                    cx,
                );
            } else if flags.can_remove_field {
                PropertyActionDialog::open_remove_field(
                    this.state.clone(),
                    session_key,
                    meta,
                    flags.allow_bulk,
                    window,
                    cx,
                );
            }
        }))
        .on_action(cx.listener(|this, _: &AddField, window, cx| {
            let Some((session_key, meta)) = this.selected_property_context(cx) else {
                return;
            };
            let flags = property_flags(&meta);
            if !flags.can_add_field {
                return;
            }
            PropertyActionDialog::open_add_field(
                this.state.clone(),
                session_key,
                meta,
                flags.allow_bulk,
                window,
                cx,
            );
        }))
        .on_action(cx.listener(|this, _: &AddElement, window, cx| {
            let Some((session_key, meta)) = this.selected_property_context(cx) else {
                return;
            };
            let flags = property_flags(&meta);
            if !flags.is_array || flags.is_array_element {
                return;
            }
            PropertyActionDialog::open_add_element(
                this.state.clone(),
                session_key,
                meta,
                flags.allow_bulk,
                window,
                cx,
            );
        }))
        .on_action(cx.listener(|this, _: &RemoveMatchingValues, window, cx| {
            let Some((session_key, meta)) = this.selected_property_context(cx) else {
                return;
            };
            let flags = property_flags(&meta);
            if !flags.is_array || flags.is_array_element {
                return;
            }
            PropertyActionDialog::open_remove_matching(
                this.state.clone(),
                session_key,
                meta,
                flags.allow_bulk,
                window,
                cx,
            );
        }))
        .on_action(cx.listener(|this, _: &CopyValue, _window, cx| {
            if let Some((session_key, meta)) = this.selected_property_context(cx)
                && let Some(doc) = this.resolve_document(&session_key, &meta.doc_key, cx)
                && let Some(value) = get_bson_at_path(&doc, &meta.path)
            {
                let text = format_bson_for_clipboard(value);
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                return;
            }
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let selected_docs: Vec<_> = {
                let state_ref = this.state.read(cx);
                let Some(session) = state_ref.session(&session_key) else {
                    return;
                };
                session
                    .data
                    .items
                    .iter()
                    .filter(|item| session.view.selected_docs.contains(&item.key))
                    .map(|item| item.key.clone())
                    .collect()
            };
            if selected_docs.is_empty() {
                return;
            }
            if selected_docs.len() == 1 {
                let doc_key = &selected_docs[0];
                if let Some(doc) = this.resolve_document(&session_key, doc_key, cx) {
                    let json = document_to_json_string(&doc);
                    cx.write_to_clipboard(ClipboardItem::new_string(json));
                }
            } else {
                // Gather the owned documents on the main thread (cheap clone),
                // then serialize off-thread so a large multi-selection copy
                // doesn't block the UI.
                let docs: Vec<Document> = {
                    let state_ref = this.state.read(cx);
                    selected_docs
                        .iter()
                        .filter_map(|dk| state_ref.session_draft_or_document(&session_key, dk))
                        .collect()
                };
                let task = cx.background_spawn(async move {
                    let parts: Vec<String> = docs.iter().map(document_to_json_string).collect();
                    format!("[{}]", parts.join(",\n"))
                });
                cx.spawn(async move |_this, cx: &mut gpui_kit::AsyncApp| {
                    let json = task.await;
                    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(json)));
                })
                .detach();
            }
        }))
        .on_action(cx.listener(|this, _: &CopyAs, _window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let (view_mode, selected_count, focused_expanded) = {
                let state_ref = this.state.read(cx);
                let vm = state_ref.session_view_mode(&session_key);
                let view = state_ref.session_view(&session_key);
                let selected_count = view.map_or(0, |view| view.selected_docs.len());
                let focused_expanded = view.is_some_and(|view| {
                    view.selected_node_id
                        .as_ref()
                        .is_some_and(|id| view.expanded_nodes.contains(id))
                });
                (vm, selected_count, focused_expanded)
            };

            match view_mode {
                DocumentViewMode::Tree | DocumentViewMode::Json => {
                    let property_ctx = this.selected_property_context(cx);
                    match tree_copy_target(
                        property_ctx.as_ref().map(|(_, meta)| meta.path.as_slice()),
                        selected_count,
                        focused_expanded,
                    ) {
                        TreeCopyTarget::FocusedProperty => {
                            if let Some((sk, meta)) = property_ctx
                                && let Some(doc) = this.resolve_document(&sk, &meta.doc_key, cx)
                                && let Some(value) = get_bson_at_path(&doc, &meta.path)
                            {
                                let text = format_bson_for_clipboard(value);
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }
                        }
                        TreeCopyTarget::FocusedDocument => {
                            if let Some((sk, meta)) = property_ctx
                                && let Some(doc) = this.resolve_document(&sk, &meta.doc_key, cx)
                            {
                                let text = document_to_json_string(&doc);
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }
                        }
                        TreeCopyTarget::FocusedDocumentId => {
                            if let Some((sk, meta)) = property_ctx
                                && let Some(doc) = this.resolve_document(&sk, &meta.doc_key, cx)
                                && let Some(id) = doc.get("_id")
                            {
                                let text = format_bson_for_clipboard(id);
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }
                        }
                        TreeCopyTarget::SelectedDocuments => {
                            copy_documents_as(this, CopyFormat::Json, ExportScope::Selected, cx);
                        }
                        TreeCopyTarget::None => {}
                    }
                }
                DocumentViewMode::Table => {
                    if selected_count > 0 {
                        copy_documents_as(this, CopyFormat::Tsv, ExportScope::Selected, cx);
                    }
                }
            }
        }))
        .on_action(cx.listener(|this, _: &CopyAsJson, _window, cx| {
            copy_documents_as(this, CopyFormat::Json, ExportScope::Selected, cx);
        }))
        .on_action(cx.listener(|this, _: &CopyAsJsonLines, _window, cx| {
            copy_documents_as(this, CopyFormat::JsonLines, ExportScope::Selected, cx);
        }))
        .on_action(cx.listener(|this, _: &CopyAsCsv, _window, cx| {
            copy_documents_as(this, CopyFormat::Csv, ExportScope::Selected, cx);
        }))
        .on_action(cx.listener(|this, _: &CopyAsMarkdown, _window, cx| {
            copy_documents_as(this, CopyFormat::Markdown, ExportScope::Selected, cx);
        }))
        .on_action(cx.listener(|this, _: &CopyAsTsv, _window, cx| {
            copy_documents_as(this, CopyFormat::Tsv, ExportScope::Selected, cx);
        }))
        .on_action(cx.listener(|this, _: &CopyKey, _window, cx| {
            let Some((_session_key, meta)) = this.selected_property_context(cx) else {
                return;
            };
            cx.write_to_clipboard(ClipboardItem::new_string(meta.key_label));
        }))
        .on_action(cx.listener(|this, _: &DiscardDocumentChanges, window, cx| {
            this.discard_selected_documents(window, cx);
        }))
        .on_action(cx.listener(|this, _: &ShowDocumentsSubview, _window, cx| {
            if !this.finish_document_edit(cx) {
                return;
            }
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            this.state.update(cx, |state, cx| {
                state.set_collection_subview(&session_key, CollectionSubview::Documents);
                cx.notify();
            });
        }))
        .on_action(cx.listener(|this, _: &ShowIndexesSubview, _window, cx| {
            if !this.finish_document_edit(cx) {
                return;
            }
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            this.state.update(cx, |state, cx| {
                state.set_collection_subview(&session_key, CollectionSubview::Indexes);
                cx.notify();
            });
            AppCommands::load_collection_indexes(this.state.clone(), session_key, false, cx);
        }))
        .on_action(cx.listener(|this, _: &ShowStatsSubview, _window, cx| {
            if !this.finish_document_edit(cx) {
                return;
            }
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let should_load = this.state.update(cx, |state, cx| {
                let should_load =
                    state.set_collection_subview(&session_key, CollectionSubview::Stats);
                cx.notify();
                should_load
            });
            if should_load {
                AppCommands::load_collection_stats(this.state.clone(), session_key, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ShowAggregationSubview, _window, cx| {
            if !this.finish_document_edit(cx) {
                return;
            }
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            this.state.update(cx, |state, cx| {
                state.set_collection_subview(&session_key, CollectionSubview::Aggregation);
                cx.notify();
            });
        }))
        .on_action(cx.listener(|this, _: &ShowHistorySubview, _window, cx| {
            if !this.finish_document_edit(cx) {
                return;
            }
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            if !this.state.read(cx).collection_history_available(
                session_key.connection_id,
                &session_key.database,
                &session_key.collection,
            ) {
                return;
            }
            this.state.update(cx, |state, cx| {
                state.set_collection_subview(&session_key, CollectionSubview::History);
                cx.notify();
            });
            AppCommands::load_collection_history(this.state.clone(), session_key, cx);
        }))
        .on_action(cx.listener(|this, _: &ShowSchemaSubview, _window, cx| {
            if !this.finish_document_edit(cx) {
                return;
            }
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let should_load = this.state.update(cx, |state, cx| {
                let should_load =
                    state.set_collection_subview(&session_key, CollectionSubview::Schema);
                cx.notify();
                should_load
            });
            if should_load {
                AppCommands::analyze_collection_schema(this.state.clone(), session_key, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &RunAggregation, window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let subview = this
                .state
                .read(cx)
                .session_subview(&session_key)
                .unwrap_or(CollectionSubview::Documents);
            if subview != CollectionSubview::Aggregation {
                return;
            }
            super::request_run_aggregation(this.state.clone(), session_key, false, window, cx);
        }))
        .on_action(cx.listener(|this, _: &FormatAggregationStage, window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let subview = this
                .state
                .read(cx)
                .session_subview(&session_key)
                .unwrap_or(CollectionSubview::Documents);
            if subview != CollectionSubview::Aggregation {
                return;
            }
            let Some(body_state) = this.aggregation_stage_body_state.clone() else {
                return;
            };
            let selected = this
                .state
                .read(cx)
                .session(&session_key)
                .and_then(|session| session.data.aggregation.selected_stage);
            if selected.is_none() {
                return;
            }
            let raw = body_state.read(cx).value().to_string();
            match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(value) => {
                    if let Ok(formatted) = serde_json::to_string_pretty(&value) {
                        body_state.update(cx, |state, cx| {
                            state.set_value(formatted, window, cx);
                        });
                    }
                }
                Err(err) => {
                    this.state.update(cx, |state, cx| {
                        state.set_status_message(Some(StatusMessage::error(format!(
                            "Invalid JSON: {err}"
                        ))));
                        cx.notify();
                    });
                }
            }
        }))
        .on_action(cx.listener(|this, _: &ClearAggregationStage, window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let subview = this
                .state
                .read(cx)
                .session_subview(&session_key)
                .unwrap_or(CollectionSubview::Documents);
            if subview != CollectionSubview::Aggregation {
                return;
            }
            let Some(body_state) = this.aggregation_stage_body_state.clone() else {
                return;
            };
            let selected = this
                .state
                .read(cx)
                .session(&session_key)
                .and_then(|session| session.data.aggregation.selected_stage);
            let Some(selected) = selected else {
                return;
            };
            body_state.update(cx, |state, cx| {
                state.set_value("{}".to_string(), window, cx);
            });
            this.state.update(cx, |state, cx| {
                state.set_pipeline_stage_body(&session_key, selected, "{}".to_string());
                cx.notify();
            });
        }))
        .on_action(cx.listener(|this, _: &SelectPrevAggregationStage, _window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let subview = this
                .state
                .read(cx)
                .session_subview(&session_key)
                .unwrap_or(CollectionSubview::Documents);
            if subview != CollectionSubview::Aggregation {
                return;
            }
            let pipeline =
                this.state.read(cx).session_data(&session_key).map(|data| data.aggregation.clone());
            let Some(pipeline) = pipeline else {
                return;
            };
            let count = pipeline.stages.len();
            if count == 0 {
                return;
            }
            let current = pipeline.selected_stage.unwrap_or(0);
            let next = current.saturating_sub(1);
            this.state.update(cx, |state, cx| {
                state.set_pipeline_selected_stage(&session_key, Some(next));
                cx.notify();
            });
        }))
        .on_action(cx.listener(|this, _: &SelectNextAggregationStage, _window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let subview = this
                .state
                .read(cx)
                .session_subview(&session_key)
                .unwrap_or(CollectionSubview::Documents);
            if subview != CollectionSubview::Aggregation {
                return;
            }
            let pipeline =
                this.state.read(cx).session_data(&session_key).map(|data| data.aggregation.clone());
            let Some(pipeline) = pipeline else {
                return;
            };
            let count = pipeline.stages.len();
            if count == 0 {
                return;
            }
            let current = pipeline.selected_stage.unwrap_or(0);
            let next = (current + 1).min(count.saturating_sub(1));
            this.state.update(cx, |state, cx| {
                state.set_pipeline_selected_stage(&session_key, Some(next));
                cx.notify();
            });
        }))
        .on_action(cx.listener(|this, _: &MoveAggregationStageUp, _window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let subview = this
                .state
                .read(cx)
                .session_subview(&session_key)
                .unwrap_or(CollectionSubview::Documents);
            if subview != CollectionSubview::Aggregation {
                return;
            }
            let pipeline =
                this.state.read(cx).session_data(&session_key).map(|data| data.aggregation.clone());
            let Some(pipeline) = pipeline else {
                return;
            };
            let Some(selected) = pipeline.selected_stage else {
                return;
            };
            if selected == 0 {
                return;
            }
            let target = selected.saturating_sub(1);
            this.state.update(cx, |state, cx| {
                state.move_pipeline_stage(&session_key, selected, target);
                cx.notify();
            });
        }))
        .on_action(cx.listener(|this, _: &MoveAggregationStageDown, _window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let subview = this
                .state
                .read(cx)
                .session_subview(&session_key)
                .unwrap_or(CollectionSubview::Documents);
            if subview != CollectionSubview::Aggregation {
                return;
            }
            let pipeline =
                this.state.read(cx).session_data(&session_key).map(|data| data.aggregation.clone());
            let Some(pipeline) = pipeline else {
                return;
            };
            let Some(selected) = pipeline.selected_stage else {
                return;
            };
            if selected + 1 >= pipeline.stages.len() {
                return;
            }
            let target = selected + 1;
            this.state.update(cx, |state, cx| {
                state.move_pipeline_stage(&session_key, selected, target);
                cx.notify();
            });
        }))
        .on_action(cx.listener(|this, _: &DuplicateAggregationStage, _window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let subview = this
                .state
                .read(cx)
                .session_subview(&session_key)
                .unwrap_or(CollectionSubview::Documents);
            if subview != CollectionSubview::Aggregation {
                return;
            }
            let selected = this
                .state
                .read(cx)
                .session(&session_key)
                .and_then(|session| session.data.aggregation.selected_stage);
            let Some(selected) = selected else {
                return;
            };
            this.state.update(cx, |state, cx| {
                state.duplicate_pipeline_stage(&session_key, selected);
                cx.notify();
            });
        }))
        .on_action(cx.listener(|this, _: &ToggleAggregationStageEnabled, _window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let subview = this
                .state
                .read(cx)
                .session_subview(&session_key)
                .unwrap_or(CollectionSubview::Documents);
            if subview != CollectionSubview::Aggregation {
                return;
            }
            let selected = this
                .state
                .read(cx)
                .session(&session_key)
                .and_then(|session| session.data.aggregation.selected_stage);
            let Some(selected) = selected else {
                return;
            };
            this.state.update(cx, |state, cx| {
                state.toggle_pipeline_stage_enabled(&session_key, selected);
                let enabled = state
                    .session(&session_key)
                    .and_then(|session| session.data.aggregation.stages.get(selected))
                    .is_some_and(|stage| stage.enabled);
                let message = if enabled { "Stage enabled" } else { "Stage disabled" };
                state.set_status_message(Some(StatusMessage::info(message)));
                cx.notify();
            });
        }))
        .on_action(cx.listener(|this, _: &DeleteAggregationStage, window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let subview = this
                .state
                .read(cx)
                .session_subview(&session_key)
                .unwrap_or(CollectionSubview::Documents);
            if subview != CollectionSubview::Aggregation {
                return;
            }
            let (selected, stage_number, operator_label) = {
                let state_ref = this.state.read(cx);
                let Some(session) = state_ref.session(&session_key) else {
                    return;
                };
                let Some(selected) = session.data.aggregation.selected_stage else {
                    return;
                };
                let operator_label = session
                    .data
                    .aggregation
                    .stages
                    .get(selected)
                    .map(|stage| stage.operator.trim())
                    .filter(|label| !label.is_empty())
                    .unwrap_or("stage")
                    .to_string();
                (selected, selected + 1, operator_label)
            };

            let message = format!(
                "Delete Stage {} ({}). This cannot be undone.",
                stage_number, operator_label
            );
            let state = this.state.clone();
            open_confirm_dialog(window, cx, "Delete stage", message, "Delete", true, {
                let session_key = session_key.clone();
                move |_window, cx| {
                    state.update(cx, |state, cx| {
                        state.remove_pipeline_stage(&session_key, selected);
                        state.set_status_message(Some(StatusMessage::info("Stage deleted")));
                        cx.notify();
                    });
                }
            });
        }))
    }
}

pub(in crate::views::documents) fn copy_documents_as(
    this: &mut CollectionView,
    format: CopyFormat,
    scope: ExportScope,
    cx: &mut Context<CollectionView>,
) {
    let Some(session_key) = this.view_model.current_session() else {
        return;
    };

    let (text, count) = {
        let state_ref = this.state.read(cx);
        let Some(session) = state_ref.session(&session_key) else {
            return;
        };

        let snapshot = ViewExportSnapshot::from_session_state(
            &session.data.items,
            &session.view.selected_docs,
            if session.view.view_mode == DocumentViewMode::Table {
                &session.view.table_column_order
            } else {
                &[]
            },
            &if session.view.view_mode == DocumentViewMode::Table {
                session.view.table_hidden_columns.clone()
            } else {
                Default::default()
            },
            &session.view.table_pinned_columns,
            &session.view.drafts,
            session_key.collection.clone(),
            session_key.database.clone(),
            scope,
        );

        if snapshot.documents.is_empty() {
            return;
        }

        (render_to_clipboard(&snapshot, format), snapshot.documents.len())
    };

    cx.write_to_clipboard(ClipboardItem::new_string(text));
    this.state.update(cx, |state, cx| {
        state.set_status_message(Some(StatusMessage::info(format!(
            "Copied {} document{} as {}",
            count,
            if count == 1 { "" } else { "s" },
            format.label()
        ))));
        cx.notify();
    });
}

pub(in crate::views::documents) fn copy_aggregation_as(
    this: &mut CollectionView,
    format: CopyFormat,
    cx: &mut Context<CollectionView>,
) {
    let Some(session_key) = this.view_model.current_session() else {
        return;
    };

    let (text, count) = {
        let state_ref = this.state.read(cx);
        let Some(session) = state_ref.session(&session_key) else {
            return;
        };
        let Some(results) = session.data.aggregation.results.as_ref() else {
            return;
        };
        if results.is_empty() {
            return;
        }

        let snapshot = ViewExportSnapshot::from_documents(
            (**results).clone(),
            session_key.collection.clone(),
            session_key.database.clone(),
        );
        (render_to_clipboard(&snapshot, format), snapshot.documents.len())
    };

    cx.write_to_clipboard(ClipboardItem::new_string(text));
    this.state.update(cx, |state, cx| {
        state.set_status_message(Some(StatusMessage::info(format!(
            "Copied {} result{} as {}",
            count,
            if count == 1 { "" } else { "s" },
            format.label()
        ))));
        cx.notify();
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TreeCopyTarget {
    FocusedProperty,
    FocusedDocument,
    FocusedDocumentId,
    SelectedDocuments,
    None,
}

fn tree_copy_target(
    focused_path: Option<&[PathSegment]>,
    selected_doc_count: usize,
    focused_expanded: bool,
) -> TreeCopyTarget {
    match focused_path {
        Some(path) if !path.is_empty() => TreeCopyTarget::FocusedProperty,
        Some(_) if !focused_expanded && selected_doc_count <= 1 => {
            TreeCopyTarget::FocusedDocumentId
        }
        Some(_) if selected_doc_count == 0 => TreeCopyTarget::FocusedDocument,
        _ if selected_doc_count > 0 => TreeCopyTarget::SelectedDocuments,
        _ => TreeCopyTarget::None,
    }
}

struct PropertyFlags {
    allow_bulk: bool,
    can_edit_value: bool,
    can_rename_field: bool,
    can_remove_field: bool,
    can_remove_element: bool,
    can_add_field: bool,
    is_array: bool,
    is_array_element: bool,
}

fn property_flags(meta: &NodeMeta) -> PropertyFlags {
    let is_array_element = matches!(meta.path.last(), Some(PathSegment::Index(_)));
    let has_index = meta.path.iter().any(|segment| matches!(segment, PathSegment::Index(_)));
    let allow_bulk = !has_index;
    let is_id = matches!(meta.path.first(), Some(PathSegment::Key(key)) if key == "_id");
    let is_array = matches!(meta.value, Some(Bson::Array(_)));
    let can_edit_value = !is_id;
    let can_rename_field = !is_id && !is_array_element;
    let can_remove_field = !is_id && !is_array_element;
    let can_remove_element = is_array_element && meta.value.is_some();
    let can_add_field = !is_array_element;

    PropertyFlags {
        allow_bulk,
        can_edit_value,
        can_rename_field,
        can_remove_field,
        can_remove_element,
        can_add_field,
        is_array,
        is_array_element,
    }
}

#[cfg(test)]
mod tests {
    use super::{TreeCopyTarget, format_bson_for_clipboard, tree_copy_target};
    use crate::bson::PathSegment;
    use mongodb::bson::{Bson, oid::ObjectId};

    #[test]
    fn tree_copy_prefers_focused_property_over_selected_parent_document() {
        let path = [PathSegment::Key("name".to_string())];

        assert_eq!(tree_copy_target(Some(&path), 1, false), TreeCopyTarget::FocusedProperty);
    }

    #[test]
    fn tree_copy_uses_documents_for_expanded_rows_and_multiple_selection() {
        assert_eq!(tree_copy_target(Some(&[]), 1, true), TreeCopyTarget::SelectedDocuments);
        assert_eq!(tree_copy_target(Some(&[]), 2, false), TreeCopyTarget::SelectedDocuments);
        assert_eq!(tree_copy_target(None, 2, false), TreeCopyTarget::SelectedDocuments);
    }

    #[test]
    fn tree_copy_can_copy_focused_document_without_selected_docs_fallback() {
        assert_eq!(tree_copy_target(Some(&[]), 0, true), TreeCopyTarget::FocusedDocument);
        assert_eq!(tree_copy_target(None, 0, false), TreeCopyTarget::None);
    }

    #[test]
    fn tree_copy_uses_only_id_for_a_collapsed_document() {
        assert_eq!(tree_copy_target(Some(&[]), 1, false), TreeCopyTarget::FocusedDocumentId);
        assert_eq!(tree_copy_target(Some(&[]), 0, false), TreeCopyTarget::FocusedDocumentId);

        let hex = "507f1f77bcf86cd799439011";
        assert_eq!(
            format_bson_for_clipboard(&Bson::ObjectId(ObjectId::parse_str(hex).unwrap())),
            hex
        );
        assert_eq!(format_bson_for_clipboard(&Bson::String("custom-id".into())), "custom-id");
        assert_eq!(format_bson_for_clipboard(&Bson::Int64(42)), "42");
    }
}
