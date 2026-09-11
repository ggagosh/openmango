//! Tree row rendering for document viewer.

use gpui_kit::component::button::ButtonVariants as _;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::component::input::{Input, NumberInput};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tree::{TreeEntry, TreeState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, FocusableExt as _, Icon, IconName, Sizable as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::bson::DocumentKey;
use crate::components::Button;
use crate::components::filter_builder::drag::{
    DragField, DragFieldPreview, DragValue, DragValuePreview,
};
use crate::state::{AppState, SessionKey};
use crate::theme::{borders, colors, spacing};
use crate::views::documents::node_meta::NodeMeta;
use crate::views::documents::state::SearchMatcher;
use crate::views::documents::types::InlineEditor;

use super::super::CollectionView;
use super::tree_menus::{build_document_menu, build_property_menu};

#[derive(Clone)]
pub(crate) struct SearchOptions {
    pub(crate) matcher: Option<SearchMatcher>,
    pub(crate) values_only: bool,
}

/// Render a single tree row with optional inline editing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_tree_row(
    ix: usize,
    entry: &TreeEntry,
    _selected: bool,
    node_meta: &Arc<HashMap<String, NodeMeta>>,
    editing_node_id: &Option<String>,
    inline_state: &Option<InlineEditor>,
    inline_error: Option<&str>,
    view: Entity<CollectionView>,
    tree_state: Entity<TreeState>,
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    selected_docs: &HashSet<DocumentKey>,
    tree_order: Arc<[String]>,
    search_opts: &SearchOptions,
    current_match_id: Option<&str>,
    drag_enabled: bool,
    documents_focus: FocusHandle,
    cx: &App,
) -> ListItem {
    let item_id = entry.item().id.to_string();
    let meta = node_meta.get(&item_id);
    let is_editing = editing_node_id.as_ref().is_some_and(|id| id == &item_id);

    let key_label =
        meta.map(|meta| meta.key_label.clone()).unwrap_or_else(|| entry.item().label.to_string());
    let value_label = meta.map(|meta| meta.value_label.clone()).unwrap_or_default();
    let value_color = meta.map(|meta| meta.value_color).unwrap_or_else(|| cx.theme().foreground);
    let type_label = meta.map(|meta| meta.type_label.clone()).unwrap_or_default();
    let is_dirty = meta.map(|meta| meta.is_dirty).unwrap_or(false);
    let is_root = meta.map(|meta| meta.path.is_empty()).unwrap_or(false);
    let is_multi_selected =
        meta.map(|m| m.path.is_empty() && selected_docs.contains(&m.doc_key)).unwrap_or(false);

    let depth = entry.depth();
    let is_folder = meta.map_or_else(|| entry.is_folder(), |meta| meta.is_folder);
    let is_expanded = entry.is_expanded();

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

    let chevron_hover = cx.theme().foreground.opacity(0.1);
    let leading = if is_folder {
        div()
            .id(("doc-chevron", ix))
            .size(px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(crate::theme::borders::radius_sm())
            .cursor_pointer()
            .hover(|s| s.bg(chevron_hover))
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
                    .text_color(cx.theme().muted_foreground),
            )
            .into_any_element()
    } else {
        div().w(px(18.0)).into_any_element()
    };

    let is_draggable_field = drag_enabled && !is_root && meta.is_some();
    // Only clone the (potentially heavy) node metadata when this row can
    // actually start a drag; most rows never do.
    let drag_meta = if is_draggable_field { meta.cloned() } else { None };
    let key_drag = if is_draggable_field {
        drag_meta.as_ref().map(|meta| {
            DragField::from_path_segments(&meta.path, &meta.type_label, meta.value.as_ref())
        })
    } else {
        None
    };
    let value_drag = if is_draggable_field {
        drag_meta.as_ref().and_then(|meta| meta.value.as_ref().map(DragValue::from_bson))
    } else {
        None
    };

    let row = div().id(("tree-row", ix)).flex().items_center().w_full().gap(spacing::xs());

    // Consume the whole Kit row, including padding, before Tree handles expansion.
    let on_mouse_down = {
        let row_session = row_session.clone();
        let row_state = row_state.clone();
        let row_tree = row_tree.clone();
        let range_node_meta = node_meta.clone();
        let range_tree_order = tree_order.clone();
        move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            cx.stop_propagation();
            let can_select = row_view.update(cx, |this, cx| {
                let editing = this.view_model.editing_node_id();
                if editing.is_some()
                    && (editing.as_deref() != Some(row_item_id.as_str()) || event.click_count != 2)
                {
                    this.finish_document_edit(cx)
                } else {
                    true
                }
            });
            if !can_select {
                return;
            }
            window.focus(&row_focus, cx);
            let is_shift = event.modifiers.shift;
            let anchor = row_tree.read(cx).selected_index();
            // Only move the anchor on non-shift clicks so repeated
            // shift+clicks always extend from the original anchor.
            if !is_shift {
                row_tree.update(cx, |tree, cx| {
                    tree.set_selected_index(Some(ix), cx);
                });
            }
            if let (Some(meta), Some(session_key)) =
                (range_node_meta.get(&row_item_id), row_session.clone())
            {
                let is_cmd = event.modifiers.secondary() || event.modifiers.control;
                row_state.update(cx, |state, cx| {
                    if is_shift && meta.path.is_empty() {
                        let anchor_ix = anchor.unwrap_or(0);
                        let lo = anchor_ix.min(ix);
                        let hi = anchor_ix.max(ix);
                        let doc_keys: HashSet<DocumentKey> = range_tree_order
                            [lo..=hi.min(range_tree_order.len().saturating_sub(1))]
                            .iter()
                            .filter_map(|id| range_node_meta.get(id))
                            .filter(|m| m.path.is_empty())
                            .map(|m| m.doc_key.clone())
                            .collect();
                        state.select_doc_range(
                            &session_key,
                            doc_keys,
                            meta.doc_key.clone(),
                            row_item_id.clone(),
                        );
                    } else if is_cmd && meta.path.is_empty() {
                        state.toggle_doc_selection(&session_key, &meta.doc_key);
                        state.set_selected_node(
                            &session_key,
                            meta.doc_key.clone(),
                            row_item_id.clone(),
                        );
                    } else {
                        state.select_single_doc(
                            &session_key,
                            meta.doc_key.clone(),
                            row_item_id.clone(),
                        );
                    }
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
    };
    let row = row
        .child(render_key_column(
            ix,
            depth,
            leading,
            &key_label,
            is_root,
            key_drag,
            is_dirty,
            search_opts,
            current_match_id,
            &item_id,
            cx,
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
            inline_error,
            node_meta.clone(),
            view.clone(),
            value_drag,
            search_opts,
            current_match_id,
            cx,
        ))
        .child(
            div()
                .w(px(120.0))
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .overflow_hidden()
                .text_ellipsis()
                .child(type_label),
        );

    let row = row.context_menu({
        let node_meta = node_meta.clone();
        let menu_item_id = item_id.clone();
        let state = state.clone();
        let view = view.clone();
        let session_key = session_key.clone();
        let tree_state = tree_state.clone();
        move |menu, window, cx| {
            let menu = menu.action_context(documents_focus.clone());
            let Some(meta) = node_meta.get(&menu_item_id).cloned() else {
                return menu;
            };
            let Some(session_key) = session_key.clone() else {
                return menu;
            };
            tree_state.update(cx, |tree, cx| tree.set_selected_index(Some(ix), cx));

            // A field always targets its own document; a selected root keeps the multi-selection.
            state.update(cx, |state, cx| {
                let already_selected = state
                    .session_view(&session_key)
                    .is_some_and(|view| view.selected_docs.contains(&meta.doc_key));
                if !meta.path.is_empty() || !already_selected {
                    state.select_single_doc(
                        &session_key,
                        meta.doc_key.clone(),
                        menu_item_id.clone(),
                    );
                } else {
                    state.set_selected_node(
                        &session_key,
                        meta.doc_key.clone(),
                        menu_item_id.clone(),
                    );
                }
                cx.notify();
            });
            let selected_count = state
                .read(cx)
                .session_view(&session_key)
                .map(|view| view.selected_docs.len())
                .unwrap_or(0);

            if meta.path.is_empty() {
                build_document_menu(
                    menu,
                    state.clone(),
                    view.clone(),
                    session_key,
                    meta.doc_key.clone(),
                    meta.is_dirty,
                    selected_count,
                    crate::state::DocumentViewMode::Tree,
                    window,
                    &mut *cx,
                )
            } else {
                build_property_menu(menu, state.clone(), session_key, meta)
            }
        }
    });

    ListItem::new(ix)
        .child(row)
        .selected(!is_editing && if is_root { is_multi_selected } else { _selected })
        .px_0()
        .py(px(2.0))
        .on_mouse_down(MouseButton::Left, on_mouse_down)
}

#[allow(dead_code)]
pub fn render_readonly_tree_row(
    ix: usize,
    entry: &TreeEntry,
    selected: bool,
    node_meta: &Arc<HashMap<String, NodeMeta>>,
    view: Entity<CollectionView>,
    tree_state: Entity<TreeState>,
    cx: &App,
) -> ListItem {
    let item_id = entry.item().id.to_string();
    let meta = node_meta.get(&item_id);

    let key_label =
        meta.map(|meta| meta.key_label.clone()).unwrap_or_else(|| entry.item().label.to_string());
    let value_label = meta.map(|meta| meta.value_label.clone()).unwrap_or_default();
    let value_color = meta.map(|meta| meta.value_color).unwrap_or_else(|| cx.theme().foreground);
    let type_label = meta.map(|meta| meta.type_label.clone()).unwrap_or_default();
    let is_root = meta.map(|meta| meta.path.is_empty()).unwrap_or(false);

    let depth = entry.depth();
    let is_folder = meta.map_or_else(|| entry.is_folder(), |meta| meta.is_folder);
    let is_expanded = entry.is_expanded();

    let agg_chevron_hover = cx.theme().foreground.opacity(0.1);
    let leading = if is_folder {
        let toggle_item_id = item_id.clone();
        let toggle_view = view.clone();
        let toggle_tree = tree_state.clone();
        div()
            .id(("agg-chevron", ix))
            .size(px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(crate::theme::borders::radius_sm())
            .cursor_pointer()
            .hover(|s| s.bg(agg_chevron_hover))
            .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                if event.click_count != 1 {
                    return;
                }
                cx.stop_propagation();
                toggle_tree.update(cx, |tree, cx| {
                    tree.set_selected_index(Some(ix), cx);
                });
                toggle_view.update(cx, |this, cx| {
                    if this.aggregation_results_expanded_nodes.contains(&toggle_item_id) {
                        this.aggregation_results_expanded_nodes.remove(&toggle_item_id);
                    } else {
                        this.aggregation_results_expanded_nodes.insert(toggle_item_id.clone());
                    }
                    cx.notify();
                });
            })
            .child(
                Icon::new(if is_expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                    .xsmall()
                    .text_color(cx.theme().muted_foreground),
            )
            .into_any_element()
    } else {
        div().w(px(18.0)).into_any_element()
    };

    let row = div().flex().items_center().w_full().gap(spacing::xs());
    // Consume the whole Kit row, including padding, before Tree handles expansion.
    let on_mouse_down = {
        let row_item_id = item_id.clone();
        let row_view = view.clone();
        let row_tree = tree_state.clone();
        move |event: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
            cx.stop_propagation();
            row_tree.update(cx, |tree, cx| {
                tree.set_selected_index(Some(ix), cx);
            });
            if event.click_count == 2 && is_folder {
                row_view.update(cx, |this, cx| {
                    if this.aggregation_results_expanded_nodes.contains(&row_item_id) {
                        this.aggregation_results_expanded_nodes.remove(&row_item_id);
                    } else {
                        this.aggregation_results_expanded_nodes.insert(row_item_id.clone());
                    }
                    cx.notify();
                });
            }
        }
    };
    let row = row
        .child(render_key_column(
            ix,
            depth,
            leading,
            &key_label,
            is_root,
            None,
            false,
            &SearchOptions { matcher: None, values_only: false },
            None,
            &item_id,
            cx,
        ))
        .child(render_value_column_readonly(&value_label, value_color))
        .child(
            div()
                .w(px(120.0))
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .overflow_hidden()
                .text_ellipsis()
                .child(type_label),
        );

    ListItem::new(ix)
        .child(row)
        .selected(selected)
        .px_0()
        .py(px(2.0))
        .on_mouse_down(MouseButton::Left, on_mouse_down)
}

#[allow(clippy::too_many_arguments)]
fn render_key_column(
    ix: usize,
    depth: usize,
    leading: AnyElement,
    key_label: &str,
    is_root: bool,
    key_drag: Option<DragField>,
    is_dirty: bool,
    search_opts: &SearchOptions,
    current_match_id: Option<&str>,
    item_id: &str,
    cx: &App,
) -> impl IntoElement {
    let key_color = colors::syntax_key(cx);
    let key_label = key_label.to_string();
    let is_key_match = !search_opts.values_only
        && search_opts.matcher.as_ref().is_some_and(|matcher| matcher.matches(&key_label));
    let is_current_match = current_match_id.is_some_and(|id| id == item_id);

    // Index-based id avoids allocating + hashing a per-node string every frame.
    let mut key = div()
        .id(("tree-key", ix))
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
                .text_color(key_color)
                .overflow_hidden()
                .text_ellipsis()
                .when(is_root && is_dirty, |s: Div| {
                    s.child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(cx.theme().primary))
                })
                .when(is_key_match && !is_dirty, {
                    let dirty_bg = colors::bg_dirty(cx);
                    move |s: Div| {
                        s.bg(dirty_bg).rounded(borders::radius_sm()).px(spacing::xs()).py(px(1.0))
                    }
                })
                .when(is_current_match && is_key_match, |s: Div| {
                    s.border_1()
                        .border_color(cx.theme().primary)
                        .rounded(borders::radius_sm())
                        .px(spacing::xs())
                        .py(px(1.0))
                })
                .child(key_label),
        );

    if let Some(key_drag) = key_drag {
        let preview_path = key_drag.path.clone();
        let preview_type = key_drag.field_type;
        key = key.cursor_move().on_drag(key_drag, move |_drag, _position, _window, cx| {
            cx.stop_propagation();
            cx.new(|_| DragFieldPreview { path: preview_path.clone(), field_type: preview_type })
        });
    }

    key
}

#[allow(clippy::too_many_arguments)]
fn render_value_column(
    ix: usize,
    item_id: &str,
    is_editing: bool,
    is_dirty: bool,
    selected: bool,
    value_label: &str,
    value_color: Hsla,
    inline_state: &Option<InlineEditor>,
    inline_error: Option<&str>,
    node_meta: Arc<HashMap<String, NodeMeta>>,
    view: Entity<CollectionView>,
    value_drag: Option<DragValue>,
    search_opts: &SearchOptions,
    current_match_id: Option<&str>,
    cx: &App,
) -> impl IntoElement {
    let item_id = item_id.to_string();
    let value_label = value_label.to_string();
    let is_match =
        search_opts.matcher.as_ref().is_some_and(|matcher| matcher.matches(&value_label));
    let is_current_match = current_match_id.is_some_and(|id| id == item_id.as_str());

    // Index-based id avoids allocating + hashing a per-node string every frame.
    let mut value = div()
        .id(("tree-value", ix))
        .flex()
        .items_center()
        .gap(spacing::xs())
        .flex_1()
        .min_w(px(0.0))
        .when(is_dirty && !selected && !is_editing, {
            let dirty_bg = colors::bg_dirty(cx);
            move |s| s.bg(dirty_bg).rounded(borders::radius_sm()).px(spacing::xs()).py(px(1.0))
        })
        .when(is_match && !is_dirty && !selected && !is_editing, {
            let dirty_bg = colors::bg_dirty(cx);
            move |s| s.bg(dirty_bg).rounded(borders::radius_sm()).px(spacing::xs()).py(px(1.0))
        })
        .when(is_current_match && !selected && !is_editing, |s| {
            s.border_1()
                .border_color(cx.theme().primary)
                .rounded(borders::radius_sm())
                .px(spacing::xs())
                .py(px(1.0))
        })
        .when(!is_editing, {
            let item_id = item_id.clone();
            let node_meta = node_meta.clone();
            let view = view.clone();
            move |this| {
                this.on_mouse_down(
                    MouseButton::Left,
                    move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                        if let Some(meta) = node_meta.get(&item_id) {
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
            render_inline_editor(ix, inline_state, inline_error, view.clone(), cx)
        } else {
            div()
                .text_sm()
                .text_color(value_color)
                .overflow_hidden()
                .text_ellipsis()
                .child(value_label)
                .into_any_element()
        });

    if let Some(value_drag) = value_drag
        && !is_editing
    {
        let preview = value_drag.preview.clone();
        let preview_type = value_drag.field_type;
        value = value.cursor_move().on_drag(value_drag, move |_drag, _position, _window, cx| {
            cx.stop_propagation();
            cx.new(|_| DragValuePreview { preview: preview.clone(), field_type: preview_type })
        });
    }

    value
}

fn render_inline_editor(
    ix: usize,
    inline_state: &Option<InlineEditor>,
    inline_error: Option<&str>,
    view: Entity<CollectionView>,
    cx: &App,
) -> AnyElement {
    let Some(inline_state) = inline_state else {
        return div().into_any_element();
    };
    let border_color = if inline_error.is_some() { cx.theme().danger } else { cx.theme().ring };

    let editor = match inline_state {
        InlineEditor::Text(state) => Input::new(state)
            .font_family(crate::theme::fonts::mono())
            .xsmall()
            .text_sm()
            .focus_bordered(false)
            .border_color(border_color)
            .rounded(borders::radius_xs())
            .flex_1()
            .min_w(px(0.0))
            .into_any_element(),
        InlineEditor::Number(state) => NumberInput::new(state)
            .font_family(crate::theme::fonts::mono())
            .xsmall()
            .text_sm()
            .focus_ring(false)
            .border_color(border_color)
            .rounded(borders::radius_xs())
            .flex_1()
            .min_w(px(0.0))
            .max_w(px(320.0))
            .into_any_element(),
        InlineEditor::Bool(current) => {
            let current = *current;
            div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .child(Switch::new(("inline-bool", ix)).checked(current).xsmall().on_click({
                    let view = view.clone();
                    move |checked, _window, cx| {
                        view.update(cx, |this, cx| {
                            this.view_model.set_inline_bool(*checked);
                            let state = this.state.clone();
                            this.view_model.sync_inline_edit_draft(&state, cx);
                            cx.notify();
                        });
                    }
                }))
                .child(
                    div().text_xs().text_color(cx.theme().secondary_foreground).child(if current {
                        "true"
                    } else {
                        "false"
                    }),
                )
                .into_any_element()
        }
    };

    div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .flex_1()
        .min_w(px(0.0))
        .max_w(px(640.0))
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            // Keep focus and Done/Cancel handling inside the editing controls.
            cx.stop_propagation();
        })
        .child(editor)
        .child(
            Button::new("inline-save")
                .xsmall()
                .ghost()
                .label("Done")
                .tooltip("Keep this field change in the document draft (Enter)")
                .disabled(inline_error.is_some())
                .on_click({
                    let view = view.clone();
                    move |_, window, cx| {
                        view.update(cx, |this, cx| {
                            this.view_model.commit_inline_edit(&this.state, cx);
                            if this.view_model.inline_state().is_none() {
                                window.focus(&this.documents_focus, cx);
                            }
                            cx.notify();
                        });
                    }
                }),
        )
        .child(
            Button::new("inline-cancel")
                .xsmall()
                .ghost()
                .label("Cancel")
                .tooltip("Restore this field's previous value (Escape)")
                .on_click({
                    let view = view.clone();
                    move |_, window, cx| {
                        view.update(cx, |this, cx| {
                            this.view_model.cancel_inline_edit(&this.state, cx);
                            window.focus(&this.documents_focus, cx);
                            cx.notify();
                        });
                    }
                }),
        )
        .into_any_element()
}

#[allow(dead_code)]
fn render_value_column_readonly(value_label: &str, value_color: Hsla) -> impl IntoElement {
    div().flex().items_center().gap(spacing::xs()).flex_1().min_w(px(0.0)).child(
        div()
            .text_sm()
            .text_color(value_color)
            .overflow_hidden()
            .text_ellipsis()
            .child(value_label.to_string()),
    )
}
