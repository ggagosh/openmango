//! Tree row rendering for document viewer.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Input, NumberInput};
use gpui_component::list::ListItem;
use gpui_component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_component::switch::Switch;
use gpui_component::tree::{TreeEntry, TreeState};
use gpui_component::{Icon, IconName, Sizable as _};
use mongodb::bson::{Bson, Document};
use serde_json;

use crate::bson::{
    DocumentKey, bson_value_for_edit, document_to_relaxed_extjson_string, get_bson_at_path,
};
use crate::components::{Button, open_confirm_dialog};
use crate::state::{AppCommands, AppState, SessionKey};
use crate::theme::{borders, colors, spacing};
use crate::views::documents::node_meta::NodeMeta;
use crate::views::documents::types::InlineEditor;

use super::CollectionView;

/// Render a single tree row with optional inline editing.
#[allow(clippy::too_many_arguments)]
pub fn render_tree_row(
    ix: usize,
    entry: &TreeEntry,
    _selected: bool,
    node_meta: &Arc<HashMap<String, NodeMeta>>,
    editing_node_id: &Option<String>,
    inline_state: &Option<InlineEditor>,
    view: Entity<CollectionView>,
    tree_state: Entity<TreeState>,
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
) -> ListItem {
    let item_id = entry.item().id.to_string();
    let meta = node_meta.get(&item_id);
    let is_editing = editing_node_id.as_ref().is_some_and(|id| id == &item_id);

    let key_label =
        meta.map(|meta| meta.key_label.clone()).unwrap_or_else(|| entry.item().label.to_string());
    let value_label = meta.map(|meta| meta.value_label.clone()).unwrap_or_default();
    let value_color = meta.map(|meta| meta.value_color).unwrap_or(colors::text_primary());
    let type_label = meta.map(|meta| meta.type_label.clone()).unwrap_or_default();
    let is_dirty = meta.map(|meta| meta.is_dirty).unwrap_or(false);
    let is_root = meta.map(|meta| meta.path.is_empty()).unwrap_or(false);

    let depth = entry.depth();
    let is_folder = entry.is_folder();
    let is_expanded = entry.is_expanded();

    let leading = if is_folder {
        Icon::new(if is_expanded { IconName::ChevronDown } else { IconName::ChevronRight })
            .xsmall()
            .text_color(colors::text_muted())
            .into_any_element()
    } else {
        div().w(px(14.0)).into_any_element()
    };

    let menu_meta = meta.cloned();
    let row_meta = meta.cloned();
    let row_session = session_key.clone();
    let row_state = state.clone();
    let row_tree = tree_state.clone();
    let row_item_id = item_id.clone();

    let row = div()
        .flex()
        .items_center()
        .w_full()
        .gap(spacing::xs())
        // Prevent TreeState from toggling expansion on single click.
        // Also handle selection when clicking outside key/value columns.
        .on_mouse_down(MouseButton::Left, {
            let row_meta = row_meta.clone();
            let row_session = row_session.clone();
            let row_state = row_state.clone();
            let row_tree = row_tree.clone();
            move |_, window, cx| {
                window.blur();
                cx.stop_propagation();
                row_tree.update(cx, |tree, cx| {
                    tree.set_selected_index(Some(ix), cx);
                });
                if let (Some(meta), Some(session_key)) = (row_meta.clone(), row_session.clone()) {
                    row_state.update(cx, |state, cx| {
                        state.set_selected_node(
                            &session_key,
                            meta.doc_key.clone(),
                            row_item_id.clone(),
                        );
                        cx.notify();
                    });
                }
            }
        })
        .child(render_key_column(
            ix,
            &item_id,
            depth,
            leading,
            &key_label,
            is_root,
            is_dirty,
            node_meta.clone(),
            view.clone(),
            tree_state.clone(),
            state.clone(),
            session_key.clone(),
        ))
        .child(render_value_column(
            ix,
            &item_id,
            is_editing,
            is_dirty,
            _selected,
            &value_label,
            value_color,
            inline_state,
            node_meta.clone(),
            view.clone(),
            tree_state.clone(),
            state.clone(),
            session_key.clone(),
        ))
        .child(
            div()
                .w(px(120.0))
                .text_sm()
                .text_color(colors::text_muted())
                .overflow_hidden()
                .text_ellipsis()
                .child(type_label),
        );

    let row = row.context_menu({
        let menu_meta = menu_meta.clone();
        let menu_item_id = item_id.clone();
        let state = state.clone();
        let view = view.clone();
        let session_key = session_key.clone();
        move |menu, _window, _cx| {
            let Some(meta) = menu_meta.clone() else {
                return menu;
            };
            let Some(session_key) = session_key.clone() else {
                return menu;
            };

            if meta.path.is_empty() {
                build_document_menu(
                    menu,
                    state.clone(),
                    view.clone(),
                    session_key,
                    meta.doc_key.clone(),
                    meta.is_dirty,
                )
            } else {
                build_property_menu(
                    menu,
                    state.clone(),
                    view.clone(),
                    session_key,
                    menu_item_id.clone(),
                    meta,
                )
            }
        }
    });

    ListItem::new(ix).selected(_selected).child(row)
}

#[allow(clippy::too_many_arguments)]
fn render_key_column(
    ix: usize,
    item_id: &str,
    depth: usize,
    leading: AnyElement,
    key_label: &str,
    is_root: bool,
    is_dirty: bool,
    node_meta: Arc<HashMap<String, NodeMeta>>,
    view: Entity<CollectionView>,
    tree_state: Entity<TreeState>,
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
) -> impl IntoElement {
    let item_id = item_id.to_string();
    let key_label = key_label.to_string();

    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .flex_1()
        .min_w(px(0.0))
        .pl(px(6.0 + 14.0 * depth as f32))
        .on_mouse_down(MouseButton::Left, {
            let item_id = item_id.clone();
            move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                window.blur();
                cx.stop_propagation();
                tree_state.update(cx, |tree, cx| {
                    tree.set_selected_index(Some(ix), cx);
                });

                if let Some(meta) = node_meta.get(&item_id) {
                    if let Some(session_key) = session_key.clone() {
                        state.update(cx, |state, cx| {
                            state.set_selected_node(
                                &session_key,
                                meta.doc_key.clone(),
                                item_id.clone(),
                            );
                            if event.click_count == 2 && meta.is_folder {
                                state.toggle_expanded_node(&session_key, &item_id);
                            }
                            cx.notify();
                        });
                    }
                    view.update(cx, |this, cx| {
                        if event.click_count == 2 && meta.is_folder {
                            this.view_model.rebuild_tree(&this.state, cx);
                        }
                        cx.notify();
                    });
                }
            }
        })
        .child(leading)
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .text_sm()
                .text_color(colors::syntax_key())
                .overflow_hidden()
                .text_ellipsis()
                .when(is_root && is_dirty, |s: Div| {
                    s.child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(colors::accent()))
                })
                .child(key_label),
        )
}

#[allow(clippy::too_many_arguments)]
fn render_value_column(
    ix: usize,
    item_id: &str,
    is_editing: bool,
    is_dirty: bool,
    selected: bool,
    value_label: &str,
    value_color: Rgba,
    inline_state: &Option<InlineEditor>,
    node_meta: Arc<HashMap<String, NodeMeta>>,
    view: Entity<CollectionView>,
    tree_state: Entity<TreeState>,
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
) -> impl IntoElement {
    let item_id = item_id.to_string();
    let value_label = value_label.to_string();

    div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .flex_1()
        .min_w(px(0.0))
        .when(is_dirty && !selected, |s: Div| {
            s.bg(colors::bg_dirty()).rounded(borders::radius_sm()).px(spacing::xs()).py(px(1.0))
        })
        .when(!is_editing, {
            let item_id = item_id.clone();
            let node_meta = node_meta.clone();
            let view = view.clone();
            let tree_state = tree_state.clone();
            let state = state.clone();
            move |this: Div| {
                this.on_mouse_down(
                    MouseButton::Left,
                    move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                        window.blur();
                        cx.stop_propagation();
                        tree_state.update(cx, |tree, cx| {
                            tree.set_selected_index(Some(ix), cx);
                        });
                        if let Some(meta) = node_meta.get(&item_id) {
                            if let Some(session_key) = session_key.clone() {
                                state.update(cx, |state, cx| {
                                    state.set_selected_node(
                                        &session_key,
                                        meta.doc_key.clone(),
                                        item_id.clone(),
                                    );
                                    cx.notify();
                                });
                            }
                            view.update(cx, |this, cx| {
                                if event.click_count == 2 && meta.is_editable {
                                    this.view_model.begin_inline_edit(
                                        item_id.clone(),
                                        meta,
                                        window,
                                        &this.state,
                                        cx,
                                    );
                                }
                                cx.notify();
                            });
                        }
                    },
                )
            }
        })
        .child(if is_editing {
            render_inline_editor(ix, inline_state, view.clone())
        } else {
            div()
                .text_sm()
                .text_color(value_color)
                .overflow_hidden()
                .text_ellipsis()
                .child(value_label)
                .into_any_element()
        })
}

fn render_inline_editor(
    ix: usize,
    inline_state: &Option<InlineEditor>,
    view: Entity<CollectionView>,
) -> AnyElement {
    let Some(inline_state) = inline_state else {
        return div().into_any_element();
    };

    let editor = match inline_state {
        InlineEditor::Text(state) => Input::new(state)
            .font_family(crate::theme::fonts::mono())
            .small()
            .flex_1()
            .into_any_element(),
        InlineEditor::Number(state) => NumberInput::new(state)
            .font_family(crate::theme::fonts::mono())
            .small()
            .flex_1()
            .into_any_element(),
        InlineEditor::Bool(current) => {
            let current = *current;
            div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .child(Switch::new(("inline-bool", ix)).checked(current).small().on_click({
                    let view = view.clone();
                    move |checked, _window, cx| {
                        view.update(cx, |this, cx| {
                            this.view_model.set_inline_bool(*checked);
                            cx.notify();
                        });
                    }
                }))
                .child(div().text_xs().text_color(colors::text_secondary()).child(if current {
                    "true"
                } else {
                    "false"
                }))
                .into_any_element()
        }
    };

    div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .flex_1()
        .min_w(px(0.0))
        .child(editor)
        .child(Button::new("inline-save").compact().primary().label("Save").on_click({
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |this, cx| {
                    this.view_model.commit_inline_edit(&this.state, cx);
                    cx.notify();
                });
            }
        }))
        .child(Button::new("inline-cancel").compact().ghost().label("Cancel").on_click({
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |this, cx| {
                    this.view_model.clear_inline_edit();
                    cx.notify();
                });
            }
        }))
        .into_any_element()
}

fn build_document_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    view: Entity<CollectionView>,
    session_key: SessionKey,
    doc_key: DocumentKey,
    is_dirty: bool,
) -> PopupMenu {
    menu = menu
        .item(PopupMenuItem::new("Edit JSON").on_click({
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
        }))
        .item(PopupMenuItem::new("Delete Document").on_click({
            let state = state.clone();
            let session_key = session_key.clone();
            let doc_key = doc_key.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let message = format!("Delete document {}? This cannot be undone.", doc_key);
                open_confirm_dialog(window, cx, "Delete document", message, "Delete", true, {
                    let state = state.clone();
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
            }
        }))
        .item(PopupMenuItem::new("Copy Document JSON").on_click({
            let state = state.clone();
            let session_key = session_key.clone();
            let doc_key = doc_key.clone();
            move |_, _window, cx| {
                if let Some(doc) = resolve_document(&state, &session_key, &doc_key, cx) {
                    let json = document_to_relaxed_extjson_string(&doc);
                    cx.write_to_clipboard(ClipboardItem::new_string(json));
                }
            }
        }))
        .item(PopupMenuItem::new("Discard Changes").disabled(!is_dirty).on_click({
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
        }));

    menu
}

fn build_property_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    view: Entity<CollectionView>,
    session_key: SessionKey,
    item_id: String,
    meta: NodeMeta,
) -> PopupMenu {
    let key_label = meta.key_label.clone();
    let doc_key = meta.doc_key.clone();
    let path = meta.path.clone();
    let is_editable = meta.is_editable;
    let meta_for_edit = meta.clone();

    menu = menu
        .item(PopupMenuItem::new("Edit Value").disabled(!is_editable).on_click({
            let view = view.clone();
            let item_id = item_id.clone();
            let meta = meta_for_edit.clone();
            move |_, window, cx| {
                if !meta.is_editable {
                    return;
                }
                view.update(cx, |this, cx| {
                    this.view_model.begin_inline_edit(
                        item_id.clone(),
                        &meta,
                        window,
                        &this.state,
                        cx,
                    );
                    cx.notify();
                });
            }
        }))
        .item(PopupMenuItem::new("Copy Value").on_click({
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
        }))
        .separator()
        .item(PopupMenuItem::new("Copy Key").on_click({
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
    let state_ref = state.read(cx);
    state_ref
        .session(session_key)
        .and_then(|session| session.view.drafts.get(doc_key).cloned())
        .or_else(|| state_ref.document_for_key(session_key, doc_key))
}

fn format_bson_for_clipboard(value: &Bson) -> String {
    match value {
        Bson::Document(doc) => document_to_relaxed_extjson_string(doc),
        Bson::Array(arr) => {
            let value = Bson::Array(arr.clone()).into_relaxed_extjson();
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| format!("{value:?}"))
        }
        _ => bson_value_for_edit(value),
    }
}
