use gpui::*;
use mongodb::bson::{Bson, oid::ObjectId};

use crate::bson::{
    PathSegment, bson_value_for_edit, document_to_shell_string, format_relaxed_json_value,
    get_bson_at_path,
};
use crate::components::open_confirm_dialog;
use crate::keyboard::{
    AddElement, AddField, ClearAggregationStage, CloseSearch, CopyDocumentJson, CopyKey, CopyValue,
    CreateIndex, DeleteAggregationStage, DeleteCollection, DeleteDocument, DiscardDocumentChanges,
    DuplicateAggregationStage, DuplicateDocument, EditDocumentJson, EditValueType, FindInResults,
    FormatAggregationStage, InsertDocument, MoveAggregationStageDown, MoveAggregationStageUp,
    PasteDocuments, RemoveMatchingValues, RemoveSelectedField, RenameField, RunAggregation,
    SaveDocument, SelectNextAggregationStage, SelectPrevAggregationStage, ShowAggregationSubview,
    ShowDocumentsSubview, ShowIndexesSubview, ShowStatsSubview, ToggleAggregationStageEnabled,
};
use crate::state::{AppCommands, CollectionSubview, StatusMessage};

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
            let Some((session_key, doc_key)) = this.selected_doc_key_for_current_session(cx) else {
                return;
            };
            CollectionView::open_document_json_editor(
                cx.entity(),
                this.state.clone(),
                session_key,
                doc_key,
                window,
                cx,
            );
        }))
        .on_action(cx.listener(|this, _: &DuplicateDocument, _window, cx| {
            let Some((session_key, _doc_key, doc)) = this.selected_document_for_current_session(cx)
            else {
                return;
            };
            let mut new_doc = doc.clone();
            new_doc.insert("_id", ObjectId::new());
            AppCommands::insert_document(this.state.clone(), session_key, new_doc, cx);
        }))
        .on_action(cx.listener(|this, _: &DeleteDocument, window, cx| {
            let Some((session_key, doc_key)) = this.selected_doc_key_for_current_session(cx) else {
                return;
            };
            let message = format!("Delete document {}? This cannot be undone.", doc_key);
            open_confirm_dialog(window, cx, "Delete document", message, "Delete", true, {
                let state = this.state.clone();
                let session_key = session_key.clone();
                let doc_key = doc_key.clone();
                move |_window, cx| {
                    AppCommands::delete_document(
                        state.clone(),
                        session_key.clone(),
                        doc_key.clone(),
                        cx,
                    );
                }
            });
        }))
        .on_action(cx.listener(|this, _: &DeleteCollection, window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            let message =
                format!("Drop collection {}? This cannot be undone.", session_key.collection);
            open_confirm_dialog(window, cx, "Drop collection", message, "Drop", true, {
                let state = this.state.clone();
                let session_key = session_key.clone();
                move |_window, cx| {
                    AppCommands::drop_collection(
                        state.clone(),
                        session_key.database.clone(),
                        session_key.collection.clone(),
                        cx,
                    );
                }
            });
        }))
        .on_action(cx.listener(|this, _: &PasteDocuments, _window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            paste_documents_from_clipboard(this.state.clone(), session_key, cx);
        }))
        .on_action(cx.listener(|this, _: &CopyDocumentJson, _window, cx| {
            let Some((_session_key, _doc_key, doc)) =
                this.selected_document_for_current_session(cx)
            else {
                return;
            };
            let json = document_to_shell_string(&doc);
            cx.write_to_clipboard(ClipboardItem::new_string(json));
        }))
        .on_action(cx.listener(|this, _: &SaveDocument, _window, cx| {
            this.view_model.commit_inline_edit(&this.state, cx);
            let Some((session_key, doc_key, doc)) = this.selected_draft_for_current_session(cx)
            else {
                return;
            };
            AppCommands::save_document(this.state.clone(), session_key, doc_key, doc, cx);
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
            let Some((session_key, meta)) = this.selected_property_context(cx) else {
                return;
            };
            let Some(doc) = this.resolve_document(&session_key, &meta.doc_key, cx) else {
                return;
            };
            if let Some(value) = get_bson_at_path(&doc, &meta.path) {
                let text = format_bson_for_clipboard(value);
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }))
        .on_action(cx.listener(|this, _: &CopyKey, _window, cx| {
            let Some((_session_key, meta)) = this.selected_property_context(cx) else {
                return;
            };
            cx.write_to_clipboard(ClipboardItem::new_string(meta.key_label));
        }))
        .on_action(cx.listener(|this, _: &DiscardDocumentChanges, _window, cx| {
            let Some((session_key, doc_key)) = this.selected_doc_key_for_current_session(cx) else {
                return;
            };
            this.state.update(cx, |state, cx| {
                state.clear_draft(&session_key, &doc_key);
                cx.notify();
            });
            this.view_model.clear_inline_edit();
            this.view_model.rebuild_tree(&this.state, cx);
            this.view_model.sync_dirty_state(&this.state, cx);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ShowDocumentsSubview, _window, cx| {
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            this.state.update(cx, |state, cx| {
                state.set_collection_subview(&session_key, CollectionSubview::Documents);
                cx.notify();
            });
        }))
        .on_action(cx.listener(|this, _: &ShowIndexesSubview, _window, cx| {
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
            let Some(session_key) = this.view_model.current_session() else {
                return;
            };
            this.state.update(cx, |state, cx| {
                state.set_collection_subview(&session_key, CollectionSubview::Aggregation);
                cx.notify();
            });
        }))
        .on_action(cx.listener(|this, _: &RunAggregation, _window, cx| {
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
            AppCommands::run_aggregation(this.state.clone(), session_key, false, cx);
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
    let is_id = matches!(meta.path.last(), Some(PathSegment::Key(key)) if key == "_id");
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

fn format_bson_for_clipboard(value: &Bson) -> String {
    match value {
        Bson::Document(doc) => document_to_shell_string(doc),
        Bson::Array(arr) => {
            let value = Bson::Array(arr.clone()).into_relaxed_extjson();
            format_relaxed_json_value(&value)
        }
        _ => bson_value_for_edit(value),
    }
}
