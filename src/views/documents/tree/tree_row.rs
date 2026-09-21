//! Tree row rendering for document viewer.

use gpui_kit::component::button::ButtonVariants as _;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::tree::{TreeEntry, TreeState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use mongodb::bson::Bson;

use crate::bson::{DocumentKey, get_bson_at_path};
use crate::components::Button;
use crate::components::filter_builder::drag::{
    DragField, DragFieldPreview, DragValue, DragValuePreview,
};
use crate::state::{AppState, SessionKey};
use crate::theme::{borders, colors, spacing};
use crate::views::documents::node_meta::NodeMeta;
use crate::views::documents::reference::{
    IncomingLink, ReferenceLink, incoming_arrow, on_incoming_mouse_down, on_reference_mouse_down,
    peek_arrow,
};
use crate::views::documents::state::SearchMatcher;
use crate::views::documents::table::cell_renderer::value_details_tooltip;

use super::super::CollectionView;
use super::tree_menus::{build_document_menu, build_property_menu};

/// The hover group a row forms, so the peek arrow can appear with the rest of the row's hover
/// affordances rather than sitting in the grid permanently.
const TREE_ROW_GROUP: &str = "tree-row-group";

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
    inline_state: &Option<Entity<InputState>>,
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
            .id("doc-chevron")
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

    // A value that points somewhere becomes a link. Built once per row: deciding this reads
    // only the value already in hand, no query.
    let reference_link =
        meta.and_then(|meta| ReferenceLink::for_node(&state, session_key.as_ref(), meta));
    // The document's own `_id` is the other half of the same idea: not a link out, but where
    // "what points at this?" is asked from.
    let incoming_link =
        meta.and_then(|meta| IncomingLink::for_node(&state, session_key.as_ref(), meta));

    // A date or binary value shows its other readings on hover. The row keeps no value for a
    // field that can't be edited, so the card looks it up when it opens.
    let details_tooltip: Option<ValueTooltip> =
        meta.filter(|meta| meta.has_details).zip(session_key.clone()).map(|(meta, session_key)| {
            let state = state.clone();
            let (doc_key, path) = (meta.doc_key.clone(), meta.path.clone());
            Box::new(move |window: &mut Window, cx: &mut App| {
                let value = state
                    .read(cx)
                    .session_draft_or_document(&session_key, &doc_key)
                    .and_then(|doc| get_bson_at_path(&doc, &path).cloned())
                    .unwrap_or(Bson::Null);
                value_details_tooltip(&value, window, cx)
            }) as ValueTooltip
        });

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

    // Keyed by node, not position: expanding a node inserts rows below it, and a drag or an
    // inline edit must stay with its node. One keyed id per row; the chevron, key and value
    // inside take static ids, which this one scopes, so nothing else allocates per frame.
    let row = div()
        .id((ElementId::from("tree-row"), item_id.clone()))
        .group(TREE_ROW_GROUP)
        .flex()
        .items_center()
        .w_full()
        .gap(spacing::xs());

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
            reference_link,
            incoming_link,
            details_tooltip,
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
                build_property_menu(menu, state.clone(), session_key, meta, window, &mut *cx)
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

#[allow(clippy::too_many_arguments)]
fn render_key_column(
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

    // Static: the row around it is keyed by node.
    let mut key = div()
        .id("tree-key")
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
                // Highlights pad outward through matching negative margins, so the key keeps its
                // place and size.
                .when(is_key_match && !is_dirty, {
                    let dirty_bg = colors::bg_dirty(cx);
                    move |s: Div| {
                        s.bg(dirty_bg)
                            .rounded(borders::radius_sm())
                            .px(spacing::xs())
                            .mx(-spacing::xs())
                            .py(px(1.0))
                            .my(px(-1.0))
                    }
                })
                .when(is_current_match && is_key_match, |s: Div| {
                    s.border_1()
                        .border_color(cx.theme().primary)
                        .rounded(borders::radius_sm())
                        .px(spacing::xs())
                        .mx(-(spacing::xs() + px(1.0)))
                        .py(px(1.0))
                        .my(px(-2.0))
                })
                .child(key_label)
                // Trailing, so marking a document unsaved never pushes its key sideways.
                .when(is_root && is_dirty, |s: Div| {
                    s.child(
                        div().flex_shrink_0().size(px(6.0)).rounded_full().bg(cx.theme().primary),
                    )
                }),
        );

    if let Some(key_drag) = key_drag {
        let preview_path = key_drag.path.clone();
        let preview_type = key_drag.field_type;
        key = key.cursor_grab().on_drag(key_drag, move |_drag, grab_offset, window, cx| {
            cx.stop_propagation();
            let preview = DragFieldPreview { path: preview_path.clone(), field_type: preview_type };
            crate::components::drag::at_cursor(grab_offset, preview, window, cx)
        });
    }

    key
}

#[allow(clippy::too_many_arguments)]
fn render_value_column(
    item_id: &str,
    is_editing: bool,
    is_dirty: bool,
    selected: bool,
    value_label: &str,
    value_color: Hsla,
    inline_state: &Option<Entity<InputState>>,
    inline_error: Option<&str>,
    node_meta: Arc<HashMap<String, NodeMeta>>,
    view: Entity<CollectionView>,
    value_drag: Option<DragValue>,
    reference_link: Option<ReferenceLink>,
    incoming_link: Option<IncomingLink>,
    details_tooltip: Option<ValueTooltip>,
    search_opts: &SearchOptions,
    current_match_id: Option<&str>,
    cx: &App,
) -> impl IntoElement {
    let item_id = item_id.to_string();
    let value_label = value_label.to_string();
    let is_match =
        search_opts.matcher.as_ref().is_some_and(|matcher| matcher.matches(&value_label));
    let is_current_match = current_match_id.is_some_and(|id| id == item_id.as_str());

    // Static: the row around it is keyed by node.
    let mut value =
        div().id("tree-value").flex().items_center().gap(spacing::xs()).flex_1().min_w(px(0.0));
    if !selected && !is_editing {
        value = with_value_chrome(
            value,
            (is_dirty || is_match).then(|| colors::bg_dirty(cx)),
            is_current_match.then(|| cx.theme().primary),
        );
    }
    value = value
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
            render_inline_editor(inline_state, inline_error, view.clone(), cx)
        } else if let Some(link) = reference_link.clone() {
            // Underlined on hover and followed on Cmd+click. Plain click still selects and
            // double-click still edits, because this handler ignores everything else.
            value_text(value_label, value_color)
                .id("tree-value-link")
                .cursor_pointer()
                .hover(|style| style.underline())
                .on_mouse_down(MouseButton::Left, on_reference_mouse_down(link))
                .into_any_element()
        } else if let Some(link) = incoming_link.clone() {
            // The same gesture as a reference, asking the same question the other way round.
            value_text(value_label, value_color)
                .id("tree-value-incoming")
                .cursor_pointer()
                .hover(|style| style.underline())
                .on_mouse_down(MouseButton::Left, on_incoming_mouse_down(link))
                .into_any_element()
        } else if let Some(tooltip) = details_tooltip {
            value_text(value_label, value_color)
                .id("tree-value-details")
                .tooltip(tooltip)
                .into_any_element()
        } else {
            value_text(value_label, value_color).into_any_element()
        })
        .when_some(reference_link, |this, link| this.child(peek_arrow(link, TREE_ROW_GROUP, cx)))
        .when_some(incoming_link, |this, link| this.child(incoming_arrow(link, TREE_ROW_GROUP)));

    if let Some(value_drag) = value_drag
        && !is_editing
    {
        let preview = value_drag.preview.clone();
        let preview_type = value_drag.field_type;
        value = value.cursor_grab().on_drag(value_drag, move |_drag, grab_offset, window, cx| {
            cx.stop_propagation();
            let preview = DragValuePreview { preview: preview.clone(), field_type: preview_type };
            crate::components::drag::at_cursor(grab_offset, preview, window, cx)
        });
    }

    value
}

/// Builds a value's hover card when it opens.
type ValueTooltip = Box<dyn Fn(&mut Window, &mut App) -> AnyView>;

/// The value text shares one line box with its inline editor: the kit input's single-line height.
const VALUE_LINE_HEIGHT: Rems = Rems(1.25);

fn value_text(label: String, color: Hsla) -> Div {
    div()
        .text_sm()
        .line_height(VALUE_LINE_HEIGHT)
        .text_color(color)
        .overflow_hidden()
        .text_ellipsis()
        .child(label)
}

/// Paints the dirty/match highlight and the current-match ring on a layer behind the value text,
/// growing left and up/down only (the type column sits to the right). The layer takes no layout
/// space, so marking or selecting a value never moves its text. Call before adding the text.
fn with_value_chrome<E: Styled + ParentElement>(
    value: E,
    bg: Option<Hsla>,
    border: Option<Hsla>,
) -> E {
    if bg.is_none() && border.is_none() {
        return value;
    }
    let border_width = if border.is_some() { px(1.0) } else { px(0.0) };
    let mut layer = div()
        .absolute()
        .top(-(px(1.0) + border_width))
        .bottom(-(px(1.0) + border_width))
        .left(-(spacing::xs() + border_width))
        .right(px(0.0))
        .rounded(borders::radius_sm());
    if let Some(bg) = bg {
        layer = layer.bg(bg);
    }
    if let Some(color) = border {
        layer = layer.border_1().border_color(color);
    }
    value.relative().child(layer)
}

/// The inline value editor. Its padding and border sit outside the text box through negative
/// margins, and it shares the value text's line box, so double-clicking to edit moves neither the
/// value text nor the row — the in-place editing pattern of spreadsheets and file renames.
fn inline_value_input(inline_state: &Entity<InputState>, has_error: bool, cx: &App) -> Input {
    let border = px(1.0);
    Input::new(inline_state)
        .font_family(crate::theme::fonts::mono())
        .xsmall()
        .text_sm()
        .line_height(VALUE_LINE_HEIGHT)
        .h_auto()
        .px(spacing::xs())
        .py(px(0.0))
        .ml(-(spacing::xs() + border))
        .my(-border)
        .focus_bordered(false)
        .border_color(if has_error { cx.theme().danger } else { cx.theme().ring })
        .rounded(borders::radius_xs())
        .flex_1()
        .min_w(px(0.0))
}

fn render_inline_editor(
    inline_state: &Option<Entity<InputState>>,
    inline_error: Option<&str>,
    view: Entity<CollectionView>,
    cx: &App,
) -> AnyElement {
    let Some(inline_state) = inline_state else {
        return div().into_any_element();
    };
    let editor = inline_value_input(inline_state, inline_error.is_some(), cx);

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

#[cfg(test)]
mod tests {
    use gpui_kit::component::Root;
    use gpui_kit::component::input::InputState;
    use gpui_kit::{
        AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
        ParentElement as _, Render, Styled as _, TestAppContext, Window, div, px,
    };

    use super::{inline_value_input, value_text, with_value_chrome};

    struct Rows {
        input: Entity<InputState>,
    }

    impl Render for Rows {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            // Mirrors the app root's line height and the tree row's padding.
            let row = |name: &'static str| {
                div().debug_selector(move || name.into()).flex().items_center().py(px(2.0))
            };
            div()
                .w(px(600.0))
                .line_height(crate::theme::fonts::ui_line_height())
                .child(
                    row("resting-row").child(
                        value_text("value".into(), gpui_kit::black())
                            .debug_selector(|| "resting-text".into()),
                    ),
                )
                .child(
                    row("editing-row").child(
                        div()
                            .debug_selector(|| "editing-text".into())
                            .flex()
                            .flex_1()
                            .child(inline_value_input(&self.input, false, cx)),
                    ),
                )
        }
    }

    #[gpui_kit::test]
    fn editing_a_value_keeps_the_row_height(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply_design_tokens(cx);
        });
        let (_, cx) = cx.add_window_view(|window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value("value"));
            let rows = cx.new(|_| Rows { input });
            Root::new(rows, window, cx).bordered(false)
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let mut height =
            |name| cx.debug_bounds(name).unwrap_or_else(|| panic!("{name}")).size.height;
        // Horizontally the margins equal the input's padding plus border, so text keeps its x.
        assert_eq!(height("editing-row"), height("resting-row"), "row height while editing");
    }

    struct MarkedRows;

    impl Render for MarkedRows {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            // Mirrors the tree row: flexible key and value columns, then the fixed type column.
            let row = |name: &'static str, chrome: Option<(Option<Hsla>, Option<Hsla>)>| {
                let value = div().flex().flex_1().min_w(px(0.0));
                let value = match chrome {
                    Some((bg, border)) => with_value_chrome(value, bg, border),
                    None => value,
                };
                div()
                    .flex()
                    .items_center()
                    .gap(crate::theme::spacing::xs())
                    .py(px(2.0))
                    .child(div().flex_1().min_w(px(0.0)).child("key"))
                    .child(
                        value.child(
                            value_text("value".into(), gpui_kit::black())
                                .debug_selector(move || format!("{name}-text")),
                        ),
                    )
                    .child(div().w(px(120.0)).child("String"))
            };
            let tint = Some(gpui_kit::red());
            div()
                .w(px(600.0))
                .line_height(crate::theme::fonts::ui_line_height())
                .child(row("resting", None))
                .child(row("dirty", Some((tint, None))))
                .child(row("current-match", Some((tint, tint))))
                .child(row("after", None))
        }
    }

    #[gpui_kit::test]
    fn marking_a_value_keeps_its_text_in_place(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply_design_tokens(cx);
        });
        let (_, cx) = cx.add_window_view(|window, cx| {
            let rows = cx.new(|_| MarkedRows);
            Root::new(rows, window, cx).bordered(false)
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let resting = cx.debug_bounds("resting-text").expect("resting");
        let dirty = cx.debug_bounds("dirty-text").expect("dirty");
        let current = cx.debug_bounds("current-match-text").expect("current match");
        let after = cx.debug_bounds("after-text").expect("after");
        for (name, marked) in [("dirty", dirty), ("current match", current)] {
            assert_eq!(marked.origin.x, resting.origin.x, "{name} text x");
            assert_eq!(marked.size, resting.size, "{name} text size");
        }
        // Rows stack, so an equal pitch means marking changed neither row height nor text y.
        let pitch = dirty.origin.y - resting.origin.y;
        assert_eq!(current.origin.y - dirty.origin.y, pitch, "dirty row pitch");
        assert_eq!(after.origin.y - current.origin.y, pitch, "current match row pitch");
    }
}
