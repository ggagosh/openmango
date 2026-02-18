use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::input::Input;
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;
use gpui_component::tree::tree;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::bson::DocumentKey;
use crate::components::Button;
use crate::state::{AppState, SessionDocument, SessionKey};
use crate::theme::{borders, spacing};

use super::super::CollectionView;
use super::super::tree::lazy_tree::collect_all_expandable_nodes;
use super::super::tree::tree_content::{SearchOptions, render_tree_row};

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
        selected_docs: std::collections::HashSet<DocumentKey>,
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
                            .bg(cx.theme().tab_bar)
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Key"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Value"),
                            )
                            .child({
                                let state = self.state.clone();
                                let view = view.clone();
                                let session_key_for_expand = session_key.clone();
                                let documents_for_expand: Vec<SessionDocument> = documents.to_vec();
                                div()
                                    .w(px(120.0))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Type"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .child(
                                                Button::new("expand-all")
                                                    .ghost()
                                                    .compact()
                                                    .icon(Icon::new(IconName::ChevronDown).xsmall())
                                                    .tooltip("Expand all")
                                                    .on_click({
                                                        let state = state.clone();
                                                        let view = view.clone();
                                                        let session_key =
                                                            session_key_for_expand.clone();
                                                        let documents =
                                                            documents_for_expand.clone();
                                                        move |_: &ClickEvent,
                                                              _window: &mut Window,
                                                              cx: &mut App| {
                                                            let Some(session_key) =
                                                                session_key.clone()
                                                            else {
                                                                return;
                                                            };
                                                            let nodes =
                                                                collect_all_expandable_nodes(
                                                                    &documents,
                                                                );
                                                            state.update(
                                                                cx,
                                                                |state, cx| {
                                                                    state
                                                                        .set_expanded_nodes(
                                                                            &session_key,
                                                                            nodes,
                                                                        );
                                                                    cx.notify();
                                                                },
                                                            );
                                                            view.update(cx, |this, cx| {
                                                                this.view_model
                                                                    .rebuild_tree(
                                                                        &this.state,
                                                                        cx,
                                                                    );
                                                                cx.notify();
                                                            });
                                                        }
                                                    }),
                                            )
                                            .child(
                                                Button::new("collapse-all")
                                                    .ghost()
                                                    .compact()
                                                    .icon(Icon::new(IconName::ChevronUp).xsmall())
                                                    .tooltip("Collapse all")
                                                    .on_click({
                                                        let state = state.clone();
                                                        let view = view.clone();
                                                        let session_key =
                                                            session_key_for_expand.clone();
                                                        move |_: &ClickEvent,
                                                              _window: &mut Window,
                                                              cx: &mut App| {
                                                            let Some(session_key) =
                                                                session_key.clone()
                                                            else {
                                                                return;
                                                            };
                                                            state.update(
                                                                cx,
                                                                |state, cx| {
                                                                    state
                                                                        .clear_expanded_nodes(
                                                                            &session_key,
                                                                        );
                                                                    cx.notify();
                                                                },
                                                            );
                                                            view.update(cx, |this, cx| {
                                                                this.view_model
                                                                    .rebuild_tree(
                                                                        &this.state,
                                                                        cx,
                                                                    );
                                                                cx.notify();
                                                            });
                                                        }
                                                    }),
                                            ),
                                    )
                            }),
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
                                        .text_color(cx.theme().muted_foreground)
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
                                        .text_color(cx.theme().muted_foreground)
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
                                let selected_docs = selected_docs.clone();
                                let tree_order: Vec<String> = self.view_model.tree_order().to_vec();
                                let search_opts = SearchOptions {
                                    query: search_query.clone(),
                                    case_sensitive: self.search_case_sensitive,
                                    whole_word: self.search_whole_word,
                                    use_regex: self.search_regex,
                                    values_only: self.search_values_only,
                                };
                                let current_match_id = current_match_id.clone();
                                let documents_focus = self.documents_focus.clone();

                                move |ix, entry, selected, _window, cx| {
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
                                        &selected_docs,
                                        &tree_order,
                                        &search_opts,
                                        current_match_id.as_deref(),
                                        documents_focus.clone(),
                                        cx,
                                    )
                                }
                            })
                            .into_any_element()
                        },
                    ))
                    .child(if show_search {
                        let search_state = self.search_state.clone();
                        let view = view.clone();
                        let case_active = self.search_case_sensitive;
                        let word_active = self.search_whole_word;
                        let regex_active = self.search_regex;
                        let values_active = self.search_values_only;
                        let active_bg = cx.theme().secondary;
                        let active_fg = cx.theme().foreground;
                        let inactive_fg = cx.theme().muted_foreground;
                        let divider_color = cx.theme().border;
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
                            .bg(cx.theme().tab_bar)
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(if let Some(search_state) = search_state {
                                Input::new(&search_state).w(px(220.0)).into_any_element()
                            } else {
                                div().into_any_element()
                            })
                            // Match mode toggles
                            .child(search_toggle_button(
                                "search-case",
                                Icon::new(IconName::CaseSensitive).xsmall(),
                                case_active,
                                "Case Sensitive",
                                active_bg,
                                active_fg,
                                inactive_fg,
                                {
                                    let view = view.clone();
                                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                        view.update(cx, |this, cx| {
                                            this.toggle_search_case_sensitive(cx);
                                            cx.notify();
                                        });
                                    }
                                },
                            ))
                            .child(search_toggle_button(
                                "search-word",
                                Icon::default().path("icons/whole-word.svg").xsmall(),
                                word_active,
                                "Whole Word",
                                active_bg,
                                active_fg,
                                inactive_fg,
                                {
                                    let view = view.clone();
                                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                        view.update(cx, |this, cx| {
                                            this.toggle_search_whole_word(cx);
                                            cx.notify();
                                        });
                                    }
                                },
                            ))
                            .child(search_toggle_button(
                                "search-regex",
                                Icon::default().path("icons/regex.svg").xsmall(),
                                regex_active,
                                "Regex",
                                active_bg,
                                active_fg,
                                inactive_fg,
                                {
                                    let view = view.clone();
                                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                        view.update(cx, |this, cx| {
                                            this.toggle_search_regex(cx);
                                            cx.notify();
                                        });
                                    }
                                },
                            ))
                            // Divider between match mode and scope
                            .child(search_divider(divider_color))
                            .child(search_toggle_button(
                                "search-values",
                                Icon::default().path("icons/braces.svg").xsmall(),
                                values_active,
                                "Values Only",
                                active_bg,
                                active_fg,
                                inactive_fg,
                                {
                                    let view = view.clone();
                                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                        view.update(cx, |this, cx| {
                                            this.toggle_search_values_only(cx);
                                            cx.notify();
                                        });
                                    }
                                },
                            ))
                            // Divider between scope and navigation
                            .child(search_divider(divider_color))
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
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(match_label.clone()),
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
                cx,
            ))
            .into_any_element()
    }
}

#[allow(clippy::too_many_arguments)]
fn search_toggle_button(
    id: impl Into<ElementId>,
    icon: Icon,
    active: bool,
    tooltip_text: impl Into<SharedString>,
    active_bg: Hsla,
    active_fg: Hsla,
    inactive_fg: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let icon = if active { icon.text_color(active_fg) } else { icon.text_color(inactive_fg) };
    let mut btn =
        Button::new(id).ghost().compact().icon(icon).tooltip(tooltip_text).on_click(on_click);
    if active {
        btn = btn.active_style(active_bg);
    }
    btn
}

fn search_divider(color: Hsla) -> Div {
    div().w(px(1.0)).h(px(16.0)).bg(color)
}
