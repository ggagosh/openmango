use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::*;
use mongodb::bson::{Bson, Document};

use crate::bson::{
    DocumentKey, PathSegment, document_to_json_string, format_bson_for_clipboard,
    format_relaxed_json_value, get_bson_at_path, parse_document_from_json,
    parse_documents_from_json,
};
use crate::components::request_connection_write;
use crate::keyboard::{
    AddElement, AddField, CopyAsCsv, CopyAsJson, CopyAsJsonLines, CopyAsMarkdown, CopyAsTsv,
    CopyDocumentJson, CopyKey, CopyValue, DeleteDocument, DiscardDocumentChanges,
    DuplicateDocument, EditDocumentJson, EditValueType, PasteDocuments, RemoveMatchingValues,
    RemoveSelectedField, RenameField,
};
use crate::state::{AppCommands, AppState, DocumentViewMode, SessionKey, StatusMessage};
use crate::views::documents::dialogs::property_dialog::PropertyActionDialog;
use crate::views::documents::export::CopyFormat;
use crate::views::documents::node_meta::NodeMeta;

use super::super::CollectionView;

#[allow(clippy::too_many_arguments)]
pub(in crate::views::documents) fn build_document_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    _view: Entity<CollectionView>,
    session_key: SessionKey,
    doc_key: DocumentKey,
    is_dirty: bool,
    selected_count: usize,
    view_mode: DocumentViewMode,
    window: &mut Window,
    cx: &mut App,
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
    let document_id = resolve_document(&state, &session_key, &doc_key, cx)
        .and_then(|doc| doc.get("_id").map(format_bson_for_clipboard));
    menu = menu
        .item(
            PopupMenuItem::new("Edit JSON")
                .icon(Icon::new(crate::assets::AppIcon::Braces))
                .disabled(multi)
                .action(Box::new(EditDocumentJson)),
        )
        .item(
            PopupMenuItem::new(delete_label)
                .icon(Icon::new(IconName::Delete))
                .action(Box::new(DeleteDocument)),
        )
        .item(
            PopupMenuItem::new(copy_label)
                .icon(Icon::new(IconName::Copy))
                .action(Box::new(CopyDocumentJson)),
        )
        .item(
            PopupMenuItem::new("Copy ID")
                .icon(Icon::new(IconName::Copy))
                .disabled(multi || document_id.is_none())
                .on_click(move |_, _window, cx| {
                    if let Some(id) = &document_id {
                        cx.write_to_clipboard(ClipboardItem::new_string(id.clone()));
                    }
                }),
        );

    let formats = match view_mode {
        DocumentViewMode::Tree | DocumentViewMode::Json => CopyFormat::tree_formats(),
        DocumentViewMode::Table => CopyFormat::table_formats(),
    };
    let copy_as_submenu = PopupMenu::build(window, cx, |mut menu, _window, _cx| {
        for &fmt in formats {
            let action: Box<dyn Action> = match fmt {
                CopyFormat::Json => Box::new(CopyAsJson),
                CopyFormat::JsonLines => Box::new(CopyAsJsonLines),
                CopyFormat::Csv => Box::new(CopyAsCsv),
                CopyFormat::Markdown => Box::new(CopyAsMarkdown),
                CopyFormat::Tsv => Box::new(CopyAsTsv),
            };
            menu = menu.item(PopupMenuItem::new(fmt.label()).icon(fmt.icon()).action(action));
        }
        menu
    });
    menu = menu
        .item(PopupMenuItem::submenu("Copy As", copy_as_submenu).icon(Icon::new(IconName::Copy)));

    menu = menu
        .item(
            PopupMenuItem::new("Duplicate as new document…")
                .icon(Icon::new(IconName::Copy))
                .disabled(multi)
                .action(Box::new(DuplicateDocument)),
        )
        .item(PopupMenuItem::new("Paste as new documents…").action(Box::new(PasteDocuments)))
        .item(
            PopupMenuItem::new("Discard selected changes…")
                .icon(Icon::new(IconName::Undo))
                .action(Box::new(DiscardDocumentChanges))
                .disabled(!is_dirty),
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
    let is_id = matches!(meta.path.first(), Some(PathSegment::Key(key)) if key == "_id");
    let is_array = matches!(meta.value, Some(Bson::Array(_)));
    let can_edit_value = !is_id;
    menu = menu.item(
        PopupMenuItem::new("Paste value")
            .disabled(!can_edit_value)
            .action(Box::new(PasteDocuments)),
    );
    let can_rename_field = !is_id && !is_array_element;
    let can_remove_field = !is_id && !is_array_element;
    let can_remove_element = is_array_element && meta.value.is_some();
    let can_add_field = !is_array_element;

    menu = menu.item(
        PopupMenuItem::new("Edit Value / Type...")
            .icon(Icon::new(IconName::Settings2))
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
            PopupMenuItem::new("Rename Field...")
                .icon(Icon::new(IconName::Settings2))
                .action(Box::new(RenameField))
                .on_click({
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
            PopupMenuItem::new("Remove Field...")
                .icon(Icon::new(IconName::Minus))
                .action(Box::new(RemoveSelectedField))
                .on_click({
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
            PopupMenuItem::new("Remove Element...")
                .icon(Icon::new(IconName::Minus))
                .action(Box::new(RemoveSelectedField))
                .on_click({
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
                }),
        );
    }

    if can_add_field {
        menu = menu.item(
            PopupMenuItem::new("Add Field/Value...")
                .icon(Icon::new(IconName::Plus))
                .action(Box::new(AddField))
                .on_click({
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
            PopupMenuItem::new("Add Element...")
                .icon(Icon::new(IconName::Plus))
                .action(Box::new(AddElement))
                .on_click({
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
                .icon(Icon::new(IconName::Delete))
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

    // ── Copy group ───────────────────────────────────────────────
    menu = menu.separator();
    menu = menu.item(
        PopupMenuItem::new("Copy Value")
            .icon(Icon::new(IconName::Copy))
            .action(Box::new(CopyValue))
            .on_click({
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
            }),
    );
    menu = menu.item(
        PopupMenuItem::new("Copy Key")
            .icon(Icon::new(IconName::Copy))
            .action(Box::new(CopyKey))
            .on_click({
                let key_label = key_label.clone();
                move |_, _window, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(key_label.clone()));
                }
            }),
    );
    menu = menu.item(
        PopupMenuItem::new("Copy Field Path").icon(Icon::new(IconName::Copy)).on_click({
            let path = path.clone();
            move |_, _window, cx| {
                let dot_path = path_to_dot_notation(&path);
                cx.write_to_clipboard(ClipboardItem::new_string(dot_path));
            }
        }),
    );
    menu = menu.item(
        PopupMenuItem::new("Copy field and value as JSON")
            .icon(Icon::new(crate::assets::AppIcon::Braces))
            .on_click({
                let state = state.clone();
                let session_key = session_key.clone();
                let doc_key = doc_key.clone();
                let path = path.clone();
                let key_label = key_label.clone();
                move |_, _window, cx| {
                    if let Some(doc) = resolve_document(&state, &session_key, &doc_key, cx)
                        && let Some(value) = get_bson_at_path(&doc, &path)
                    {
                        let mut field = Document::new();
                        field.insert(key_label.clone(), value.clone());
                        let text = document_to_json_string(&field);
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
            }),
    );

    // ── Filter group ──────────────────────────────────────────────
    let has_value = meta.value.is_some();
    let is_filterable = has_value && !meta.is_folder;
    menu = menu.separator();
    menu = menu.item(
        PopupMenuItem::new("Filter by This Value")
            .icon(Icon::new(crate::assets::AppIcon::Filter))
            .disabled(!is_filterable)
            .on_click({
                let state = state.clone();
                let session_key = session_key.clone();
                let doc_key = doc_key.clone();
                let path = path.clone();
                move |_, _window, cx| {
                    apply_value_filter(&state, &session_key, &doc_key, &path, false, cx);
                }
            }),
    );
    menu = menu.item(
        PopupMenuItem::new("Exclude This Value")
            .icon(Icon::new(crate::assets::AppIcon::FilterX))
            .disabled(!is_filterable)
            .on_click({
                let state = state.clone();
                let session_key = session_key.clone();
                let doc_key = doc_key.clone();
                let path = path.clone();
                move |_, _window, cx| {
                    apply_value_filter(&state, &session_key, &doc_key, &path, true, cx);
                }
            }),
    );

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
    window: &mut Window,
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

    let state_for_write = state.clone();
    let target = session_key.namespace();
    request_connection_write(
        state,
        crate::components::WriteRequest::new(
            session_key.connection_id,
            target,
            format!("Insert {} documents from the clipboard", docs.len()),
            Some(crate::components::WriteConfirmation {
                title: "Paste as new documents".into(),
                message: format!(
                    "Insert {} document(s) into {} with new _id values?",
                    docs.len(),
                    session_key.namespace()
                ),
                confirm_label: "Insert documents".into(),
                destructive: false,
            }),
        ),
        window,
        cx,
        move |_window, cx| {
            AppCommands::insert_documents(state_for_write, session_key, docs, cx);
        },
    );
}

fn path_to_dot_notation(path: &[PathSegment]) -> String {
    let mut result = String::new();
    for (i, seg) in path.iter().enumerate() {
        match seg {
            PathSegment::Key(k) => {
                if i > 0 {
                    result.push('.');
                }
                result.push_str(k);
            }
            PathSegment::Index(idx) => {
                result.push_str(&format!("[{}]", idx));
            }
        }
    }
    result
}

fn bson_value_to_filter_json(value: &Bson) -> String {
    let ext = value.clone().into_relaxed_extjson();
    format_relaxed_json_value(&ext)
}

fn apply_value_filter(
    state: &Entity<AppState>,
    session_key: &SessionKey,
    doc_key: &DocumentKey,
    path: &[PathSegment],
    exclude: bool,
    cx: &mut App,
) {
    let Some(doc) = resolve_document(state, session_key, doc_key, cx) else {
        return;
    };
    let Some(value) = get_bson_at_path(&doc, path) else {
        return;
    };

    let field = path_to_dot_notation(path);
    let value_json = bson_value_to_filter_json(value);
    let filter_raw = if exclude {
        format!("{{\"{}\": {{\"$ne\": {}}}}}", field, value_json)
    } else {
        format!("{{\"{}\": {}}}", field, value_json)
    };

    let filter_doc = parse_document_from_json(&filter_raw).ok();

    let sk = session_key.clone();
    state.update(cx, |state, cx| {
        state.set_filter(&sk, filter_raw, filter_doc);
        cx.notify();
    });
    AppCommands::load_documents_for_session(state.clone(), sk, cx);
}
