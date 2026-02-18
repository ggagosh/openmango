use gpui::*;
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use mongodb::bson::{Bson, Document};

use crate::bson::{
    DocumentKey, PathSegment, bson_value_for_edit, document_to_shell_string,
    format_relaxed_json_value, get_bson_at_path, parse_documents_from_json,
};
use crate::keyboard::{
    AddElement, AddField, CopyDocumentJson, CopyKey, CopyValue, DeleteDocument,
    DiscardDocumentChanges, DuplicateDocument, EditDocumentJson, EditValueType, PasteDocuments,
    RemoveMatchingValues, RemoveSelectedField, RenameField,
};
use crate::state::{AppCommands, AppState, SessionKey, StatusMessage};
use crate::views::documents::dialogs::property_dialog::PropertyActionDialog;
use crate::views::documents::node_meta::NodeMeta;

use super::super::CollectionView;

pub(super) fn build_document_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    view: Entity<CollectionView>,
    session_key: SessionKey,
    doc_key: DocumentKey,
    is_dirty: bool,
    selected_count: usize,
) -> PopupMenu {
    let delete_label = if selected_count > 1 {
        format!("Delete {} Documents", selected_count)
    } else {
        "Delete Document".to_string()
    };
    let copy_label = if selected_count > 1 {
        format!("Copy {} Documents JSON", selected_count)
    } else {
        "Copy Document JSON".to_string()
    };
    let multi = selected_count > 1;
    menu = menu
        .item(
            PopupMenuItem::new("Edit JSON")
                .disabled(multi)
                .action(Box::new(EditDocumentJson))
                .on_click({
                    let view = view.clone();
                    let state = state.clone();
                    let session_key = session_key.clone();
                    let doc_key = doc_key.clone();
                    move |_, window, cx| {
                        CollectionView::open_document_json_editor(
                            view.clone(),
                            state.clone(),
                            session_key.clone(),
                            doc_key.clone(),
                            window,
                            cx,
                        );
                    }
                }),
        )
        .item(PopupMenuItem::new(delete_label).action(Box::new(DeleteDocument)))
        .item(PopupMenuItem::new(copy_label).action(Box::new(CopyDocumentJson)))
        .item(
            PopupMenuItem::new("Duplicate Document")
                .disabled(multi)
                .action(Box::new(DuplicateDocument))
                .on_click({
                    let state = state.clone();
                    let session_key = session_key.clone();
                    let doc_key = doc_key.clone();
                    move |_, _window, cx| {
                        if let Some(doc) = resolve_document(&state, &session_key, &doc_key, cx) {
                            let mut new_doc = doc.clone();
                            new_doc.insert("_id", mongodb::bson::oid::ObjectId::new());
                            AppCommands::insert_document(
                                state.clone(),
                                session_key.clone(),
                                new_doc,
                                cx,
                            );
                        }
                    }
                }),
        )
        .item(PopupMenuItem::new("Paste Document(s)").action(Box::new(PasteDocuments)).on_click({
            let state = state.clone();
            let session_key = session_key.clone();
            move |_, _window, cx| {
                paste_documents_from_clipboard(state.clone(), session_key.clone(), cx);
            }
        }))
        .item(
            PopupMenuItem::new("Discard Changes")
                .action(Box::new(DiscardDocumentChanges))
                .disabled(!is_dirty)
                .on_click({
                    let view = view.clone();
                    let session_key = session_key.clone();
                    let doc_key = doc_key.clone();
                    move |_, _window, cx| {
                        view.update(cx, |this, cx| {
                            this.state.update(cx, |state, cx| {
                                state.clear_draft(&session_key, &doc_key);
                                cx.notify();
                            });
                            this.view_model.clear_inline_edit();
                            this.view_model.rebuild_tree(&this.state, cx);
                            this.view_model.sync_dirty_state(&this.state, cx);
                            cx.notify();
                        });
                    }
                }),
        );

    menu
}

pub(super) fn build_property_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    session_key: SessionKey,
    meta: NodeMeta,
) -> PopupMenu {
    let key_label = meta.key_label.clone();
    let doc_key = meta.doc_key.clone();
    let path = meta.path.clone();
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

    menu = menu.item(
        PopupMenuItem::new("Edit Value / Type...")
            .action(Box::new(EditValueType))
            .disabled(!can_edit_value)
            .on_click({
                let state = state.clone();
                let session_key = session_key.clone();
                let meta = meta.clone();
                move |_, window, cx| {
                    if !can_edit_value {
                        return;
                    }
                    PropertyActionDialog::open_edit_value(
                        state.clone(),
                        session_key.clone(),
                        meta.clone(),
                        allow_bulk,
                        window,
                        cx,
                    );
                }
            }),
    );

    if can_rename_field {
        menu = menu.item(
            PopupMenuItem::new("Rename Field...").action(Box::new(RenameField)).on_click({
                let state = state.clone();
                let session_key = session_key.clone();
                let meta = meta.clone();
                move |_, window, cx| {
                    PropertyActionDialog::open_rename_field(
                        state.clone(),
                        session_key.clone(),
                        meta.clone(),
                        allow_bulk,
                        window,
                        cx,
                    );
                }
            }),
        );
    }

    if can_remove_field {
        menu = menu.item(
            PopupMenuItem::new("Remove Field...").action(Box::new(RemoveSelectedField)).on_click({
                let state = state.clone();
                let session_key = session_key.clone();
                let meta = meta.clone();
                move |_, window, cx| {
                    PropertyActionDialog::open_remove_field(
                        state.clone(),
                        session_key.clone(),
                        meta.clone(),
                        allow_bulk,
                        window,
                        cx,
                    );
                }
            }),
        );
    }

    if can_remove_element {
        menu = menu.item(
            PopupMenuItem::new("Remove Element...").action(Box::new(RemoveSelectedField)).on_click(
                {
                    let state = state.clone();
                    let session_key = session_key.clone();
                    let meta = meta.clone();
                    move |_, window, cx| {
                        PropertyActionDialog::open_remove_matching(
                            state.clone(),
                            session_key.clone(),
                            meta.clone(),
                            false,
                            window,
                            cx,
                        );
                    }
                },
            ),
        );
    }

    if can_add_field {
        menu = menu.item(
            PopupMenuItem::new("Add Field/Value...").action(Box::new(AddField)).on_click({
                let state = state.clone();
                let session_key = session_key.clone();
                let meta = meta.clone();
                move |_, window, cx| {
                    PropertyActionDialog::open_add_field(
                        state.clone(),
                        session_key.clone(),
                        meta.clone(),
                        allow_bulk,
                        window,
                        cx,
                    );
                }
            }),
        );
    }

    if is_array && !is_array_element {
        menu = menu.item(
            PopupMenuItem::new("Add Element...").action(Box::new(AddElement)).on_click({
                let state = state.clone();
                let session_key = session_key.clone();
                let meta = meta.clone();
                move |_, window, cx| {
                    PropertyActionDialog::open_add_element(
                        state.clone(),
                        session_key.clone(),
                        meta.clone(),
                        allow_bulk,
                        window,
                        cx,
                    );
                }
            }),
        );
        menu = menu.item(
            PopupMenuItem::new("Remove Matching Values...")
                .action(Box::new(RemoveMatchingValues))
                .on_click({
                    let state = state.clone();
                    let session_key = session_key.clone();
                    let meta = meta.clone();
                    move |_, window, cx| {
                        PropertyActionDialog::open_remove_matching(
                            state.clone(),
                            session_key.clone(),
                            meta.clone(),
                            allow_bulk,
                            window,
                            cx,
                        );
                    }
                }),
        );
    }

    menu = menu.item(PopupMenuItem::new("Copy Value").action(Box::new(CopyValue)).on_click({
        let state = state.clone();
        let session_key = session_key.clone();
        let doc_key = doc_key.clone();
        let path = path.clone();
        move |_, _window, cx| {
            if let Some(doc) = resolve_document(&state, &session_key, &doc_key, cx)
                && let Some(value) = get_bson_at_path(&doc, &path)
            {
                let text = format_bson_for_clipboard(value);
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }));
    menu = menu.separator();
    menu = menu.item(PopupMenuItem::new("Copy Key").action(Box::new(CopyKey)).on_click({
        let key_label = key_label.clone();
        move |_, _window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(key_label.clone()));
        }
    }));

    menu
}

fn resolve_document(
    state: &Entity<AppState>,
    session_key: &SessionKey,
    doc_key: &DocumentKey,
    cx: &App,
) -> Option<Document> {
    state.read(cx).session_draft_or_document(session_key, doc_key)
}

pub(crate) fn paste_documents_from_clipboard(
    state: Entity<AppState>,
    session_key: SessionKey,
    cx: &mut App,
) {
    let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
        state.update(cx, |state, cx| {
            state.set_status_message(Some(StatusMessage::error(
                "Clipboard is empty or does not contain text",
            )));
            cx.notify();
        });
        return;
    };

    let docs = match parse_documents_from_json(&text) {
        Ok(docs) => docs,
        Err(err) => {
            state.update(cx, |state, cx| {
                state
                    .set_status_message(Some(StatusMessage::error(format!("Invalid JSON: {err}"))));
                cx.notify();
            });
            return;
        }
    };

    if docs.is_empty() {
        state.update(cx, |state, cx| {
            state.set_status_message(Some(StatusMessage::error("No documents found")));
            cx.notify();
        });
        return;
    }

    let docs = docs
        .into_iter()
        .map(|mut doc| {
            doc.remove("_id");
            doc
        })
        .collect::<Vec<_>>();

    AppCommands::insert_documents(state, session_key, docs, cx);
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
