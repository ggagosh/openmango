use gpui::*;
use gpui_component::input::Input;
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;
use gpui_component::tree::tree;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::components::Button;
use crate::state::{AppState, SessionDocument, SessionKey};
use crate::theme::{borders, colors, spacing};

use super::super::CollectionView;
use super::super::tree::tree_content::render_tree_row;

impl CollectionView {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::views::documents) fn render_documents_subview(
        &mut self,
        documents: &[SessionDocument],
        total: u64,
        display_page: u64,
        total_pages: u64,
        range_start: u64,
        range_end: u64,
        is_loading: bool,
        session_key: Option<SessionKey>,
        state_for_prev: Entity<AppState>,
        state_for_next: Entity<AppState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let show_search = self.search_visible || self.current_search_query(cx).is_some();
        let match_total = self.search_matches.len();
        let match_position = self.search_index.map(|ix| ix + 1).unwrap_or(0);
        let match_label = if match_total == 0 {
            "0/0".to_string()
        } else {
            format!("{}/{}", match_position, match_total)
        };

        let search_query = self.current_search_query(cx);
        let current_match_id =
            self.search_index.and_then(|index| self.search_matches.get(index)).cloned();

        let view = cx.entity();
        let node_meta = self.view_model.node_meta();
        let editing_node_id = self.view_model.editing_node_id();
        let tree_state = self.view_model.tree_state();
        let inline_state = self.view_model.inline_state();

        let documents_view = div()
            .flex()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .track_focus(&self.documents_focus)
            .on_key_down({
                let view = view.clone();
                move |event, _window, cx| {
                    view.update(cx, |this, cx| {
                        if this.handle_tree_key(event, cx) {
                            cx.stop_propagation();
                        }
                    });
                }
            })
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .px(spacing::lg())
                            .py(spacing::xs())
                            .bg(colors::bg_header())
                            .border_b_1()
                            .border_color(colors::border())
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .text_color(colors::text_muted())
                                    .child("Key"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .text_color(colors::text_muted())
                                    .child("Value"),
                            )
                            .child(
                                div()
                                    .w(px(120.0))
                                    .text_xs()
                                    .text_color(colors::text_muted())
                                    .child("Type"),
                            ),
                    )
                    .child(div().flex().flex_1().min_w(px(0.0)).overflow_y_scrollbar().child(
                        if is_loading {
                            div()
                                .flex()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .gap(spacing::sm())
                                .child(Spinner::new().small())
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors::text_muted())
                                        .child("Loading documents..."),
                                )
                                .into_any_element()
                        } else if documents.is_empty() {
                            div()
                                .flex()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors::text_muted())
                                        .child("No documents found"),
                                )
                                .into_any_element()
                        } else {
                            tree(&tree_state, {
                                let view = view.clone();
                                let node_meta = node_meta.clone();
                                let editing_node_id = editing_node_id.clone();
                                let inline_state = inline_state.clone();
                                let tree_state = tree_state.clone();
                                let state_clone = self.state.clone();
                                let session_key = session_key.clone();
                                let search_query = search_query.clone();
                                let current_match_id = current_match_id.clone();
                                let documents_focus = self.documents_focus.clone();

                                move |ix, entry, selected, _window, _cx| {
                                    render_tree_row(
                                        ix,
                                        entry,
                                        selected,
                                        &node_meta,
                                        &editing_node_id,
                                        &inline_state,
                                        view.clone(),
                                        tree_state.clone(),
                                        state_clone.clone(),
                                        session_key.clone(),
                                        search_query.as_deref(),
                                        current_match_id.as_deref(),
                                        documents_focus.clone(),
                                    )
                                }
                            })
                            .into_any_element()
                        },
                    ))
                    .child(if show_search {
                        let search_state = self.search_state.clone();
                        let view = view.clone();
                        div()
                            .absolute()
                            .top(px(8.0))
                            .right(px(12.0))
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .px(spacing::sm())
                            .py(px(4.0))
                            .rounded(borders::radius_sm())
                            .bg(colors::bg_header())
                            .border_1()
                            .border_color(colors::border())
                            .child(if let Some(search_state) = search_state {
                                Input::new(&search_state).w(px(220.0)).into_any_element()
                            } else {
                                div().into_any_element()
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors::text_muted())
                                    .child(match_label.clone()),
                            )
                            .child(
                                Button::new("search-prev")
                                    .ghost()
                                    .compact()
                                    .icon(Icon::new(IconName::ChevronLeft).xsmall())
                                    .disabled(match_total == 0)
                                    .on_click({
                                        let view = view.clone();
                                        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                            view.update(cx, |this, cx| {
                                                this.prev_match(cx);
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(
                                Button::new("search-next")
                                    .ghost()
                                    .compact()
                                    .icon(Icon::new(IconName::ChevronRight).xsmall())
                                    .disabled(match_total == 0)
                                    .on_click({
                                        let view = view.clone();
                                        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                            view.update(cx, |this, cx| {
                                                this.next_match(cx);
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(
                                Button::new("search-close")
                                    .ghost()
                                    .compact()
                                    .icon(Icon::new(IconName::Close).xsmall())
                                    .on_click({
                                        let view = view.clone();
                                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                            view.update(cx, |this, cx| {
                                                this.close_search(window, cx);
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            );

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .child(documents_view)
            .child(Self::render_pagination(
                display_page,
                total_pages,
                range_start,
                range_end,
                total,
                is_loading,
                session_key,
                state_for_prev,
                state_for_next,
            ))
            .into_any_element()
    }
}
