//! Tree row rendering for document viewer.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Input, NumberInput};
use gpui_component::list::ListItem;
use gpui_component::menu::ContextMenuExt;
use gpui_component::switch::Switch;
use gpui_component::tree::{TreeEntry, TreeState};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::components::Button;
use crate::state::{AppState, SessionKey};
use crate::theme::{borders, colors, spacing};
use crate::views::documents::node_meta::NodeMeta;
use crate::views::documents::types::InlineEditor;

use super::super::CollectionView;
use super::tree_menus::{build_document_menu, build_property_menu};

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
    search_query: Option<&str>,
    current_match_id: Option<&str>,
    documents_focus: FocusHandle,
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

    let menu_meta = meta.cloned();
    let row_meta = meta.cloned();
    let row_session = session_key.clone();
    let row_state = state.clone();
    let row_tree = tree_state.clone();
    let row_item_id = item_id.clone();
    let row_focus = documents_focus.clone();
    let row_view = view.clone();
    let toggle_session = session_key.clone();
    let toggle_state = state.clone();
    let toggle_view = view.clone();
    let toggle_item_id = item_id.clone();

    let leading = if is_folder {
        div()
            .w(px(14.0))
            .flex()
            .items_center()
            .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                if event.click_count != 1 {
                    return;
                }
                let Some(session_key) = toggle_session.clone() else {
                    return;
                };
                toggle_state.update(cx, |state, cx| {
                    state.toggle_expanded_node(&session_key, &toggle_item_id);
                    cx.notify();
                });
                toggle_view.update(cx, |this, cx| {
                    this.view_model.rebuild_tree(&this.state, cx);
                    cx.notify();
                });
            })
            .child(
                Icon::new(if is_expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                    .xsmall()
                    .text_color(colors::text_muted()),
            )
            .into_any_element()
    } else {
        div().w(px(14.0)).into_any_element()
    };

    let row = div()
        .flex()
        .items_center()
        .w_full()
        .gap(spacing::xs())
        .rounded(borders::radius_sm())
        .border_1()
        .border_color(rgba(0x00000000))
        .when(_selected, |s: Div| s.bg(colors::list_selected()).border_color(colors::border()))
        .when(!_selected, |s: Div| s.hover(|s| s.bg(colors::list_hover())))
        // Prevent TreeState from toggling expansion on single click.
        // Also handle selection when clicking outside key/value columns.
        .on_mouse_down(MouseButton::Left, {
            let row_meta = row_meta.clone();
            let row_session = row_session.clone();
            let row_state = row_state.clone();
            let row_tree = row_tree.clone();
            move |event, window, cx| {
                window.focus(&row_focus);
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
                        if event.click_count == 2 && meta.is_folder {
                            state.toggle_expanded_node(&session_key, &row_item_id);
                        }
                        cx.notify();
                    });
                    if event.click_count == 2 && meta.is_folder {
                        row_view.update(cx, |this, cx| {
                            this.view_model.rebuild_tree(&this.state, cx);
                            cx.notify();
                        });
                    }
                }
            }
        })
        .child(render_key_column(depth, leading, &key_label, is_root, is_dirty))
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
            search_query,
            current_match_id,
            documents_focus.clone(),
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
        let state = state.clone();
        let view = view.clone();
        let session_key = session_key.clone();
        move |menu, _window, _cx| {
            let menu = menu.action_context(documents_focus.clone());
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
                build_property_menu(menu, state.clone(), session_key, meta)
            }
        }
    });

    ListItem::new(ix).selected(_selected).child(row)
}

#[allow(clippy::too_many_arguments)]
fn render_key_column(
    depth: usize,
    leading: AnyElement,
    key_label: &str,
    is_root: bool,
    is_dirty: bool,
) -> impl IntoElement {
    let key_label = key_label.to_string();

    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .flex_1()
        .min_w(px(0.0))
        .pl(px(6.0 + 14.0 * depth as f32))
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
    search_query: Option<&str>,
    current_match_id: Option<&str>,
    documents_focus: FocusHandle,
) -> impl IntoElement {
    let item_id = item_id.to_string();
    let value_label = value_label.to_string();
    let query = search_query.unwrap_or("").trim();
    let is_match = !query.is_empty() && value_label.to_lowercase().contains(query);
    let is_current_match = current_match_id.is_some_and(|id| id == item_id.as_str());
    let focus_handle = documents_focus.clone();

    div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .flex_1()
        .min_w(px(0.0))
        .when(is_dirty && !selected, |s: Div| {
            s.bg(colors::bg_dirty()).rounded(borders::radius_sm()).px(spacing::xs()).py(px(1.0))
        })
        .when(is_match && !is_dirty && !selected, |s: Div| {
            s.bg(colors::bg_dirty()).rounded(borders::radius_sm()).px(spacing::xs()).py(px(1.0))
        })
        .when(is_current_match && !selected, |s: Div| {
            s.border_1()
                .border_color(colors::accent())
                .rounded(borders::radius_sm())
                .px(spacing::xs())
                .py(px(1.0))
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
                        window.focus(&focus_handle);
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
