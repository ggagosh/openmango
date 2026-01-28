use crate::state::{AppCommands, CollectionStats, CollectionSubview, SessionKey};
use crate::theme::{colors, spacing};
use gpui::*;
use gpui_component::input::{InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;

use super::CollectionView;
impl Render for CollectionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.search_state.is_none() {
            let search_state = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Find in values (Cmd/Ctrl+F)")
                    .clean_on_escape()
            });
            let subscription =
                cx.subscribe_in(&search_state, window, move |view, _state, event, _window, cx| {
                    match event {
                        InputEvent::Change => {
                            view.update_search_results(cx);
                            cx.notify();
                        }
                        InputEvent::PressEnter { .. } => {
                            view.next_match(cx);
                            cx.notify();
                        }
                        _ => {}
                    }
                });
            self.search_state = Some(search_state);
            self.search_subscription = Some(subscription);
        }

        let state_ref = self.state.read(cx);
        let collection_name =
            state_ref.selected_collection_name().unwrap_or_else(|| "Unknown".to_string());
        let db_name = state_ref.selected_database_name().unwrap_or_else(|| "Unknown".to_string());
        let session_key = self.view_model.current_session();
        let snapshot =
            session_key.as_ref().and_then(|session_key| state_ref.session_snapshot(session_key));
        let (
            documents,
            total,
            page,
            per_page,
            is_loading,
            selected_doc,
            dirty_selected,
            filter_raw,
            sort_raw,
            projection_raw,
            query_options_open,
            subview,
            stats,
            stats_loading,
            stats_error,
            indexes,
            indexes_loading,
            indexes_error,
            aggregation,
        ) = if let Some(snapshot) = snapshot {
            (
                snapshot.items,
                snapshot.total,
                snapshot.page,
                snapshot.per_page,
                snapshot.is_loading,
                snapshot.selected_doc,
                snapshot.dirty_selected,
                snapshot.filter_raw,
                snapshot.sort_raw,
                snapshot.projection_raw,
                snapshot.query_options_open,
                snapshot.subview,
                snapshot.stats,
                snapshot.stats_loading,
                snapshot.stats_error,
                snapshot.indexes,
                snapshot.indexes_loading,
                snapshot.indexes_error,
                snapshot.aggregation,
            )
        } else {
            (
                Vec::new(),
                0,
                0,
                50,
                false,
                None,
                false,
                String::new(),
                String::new(),
                String::new(),
                false,
                CollectionSubview::Documents,
                None::<CollectionStats>,
                false,
                None,
                None,
                false,
                None,
                Default::default(),
            )
        };
        let filter_active = !matches!(filter_raw.trim(), "" | "{}");
        let sort_active = !matches!(sort_raw.trim(), "" | "{}");
        let projection_active = !matches!(projection_raw.trim(), "" | "{}");
        let per_page_u64 = per_page.max(1) as u64;
        let total_pages = total.div_ceil(per_page_u64).max(1);
        let display_page = page.min(total_pages.saturating_sub(1));
        let range_start = if total == 0 { 0 } else { display_page * per_page_u64 + 1 };
        let range_end = if total == 0 { 0 } else { ((display_page + 1) * per_page_u64).min(total) };

        if let Some(session_key) = session_key.clone() {
            if subview == CollectionSubview::Indexes
                && indexes.is_none()
                && !indexes_loading
                && indexes_error.is_none()
            {
                AppCommands::load_collection_indexes(
                    self.state.clone(),
                    session_key.clone(),
                    false,
                    cx,
                );
            }

            if subview == CollectionSubview::Stats
                && stats.is_none()
                && !stats_loading
                && stats_error.is_none()
            {
                AppCommands::load_collection_stats(self.state.clone(), session_key, cx);
            }
        }

        if self.filter_state.is_none() {
            let filter_state = cx.new(|cx| {
                let mut state =
                    InputState::new(window, cx).placeholder("Filter (JSON)").clean_on_escape();
                state.set_value("{}".to_string(), window, cx);
                state
            });
            let subscription =
                cx.subscribe_in(&filter_state, window, move |view, state, event, window, cx| {
                    match event {
                        InputEvent::PressEnter { .. } => {
                            if let Some(session_key) = view.view_model.current_session()
                                && let Some(filter_state) = view.filter_state.clone()
                            {
                                CollectionView::apply_filter(
                                    view.state.clone(),
                                    session_key,
                                    filter_state,
                                    window,
                                    cx,
                                );
                            }
                        }
                        InputEvent::Blur => {
                            let current = state.read(cx).value().to_string();
                            if current.trim().is_empty() {
                                state.update(cx, |input, cx| {
                                    input.set_value("{}".to_string(), window, cx);
                                });
                            }
                        }
                        _ => {}
                    }
                });
            self.filter_state = Some(filter_state);
            self.filter_subscription = Some(subscription);
        }

        if self.sort_state.is_none() {
            let sort_state = cx.new(|cx| {
                let mut state =
                    InputState::new(window, cx).placeholder("Sort (JSON)").clean_on_escape();
                state.set_value("{}".to_string(), window, cx);
                state
            });
            let subscription =
                cx.subscribe_in(&sort_state, window, move |view, state, event, window, cx| {
                    match event {
                        InputEvent::PressEnter { .. } => {
                            if let Some(session_key) = view.view_model.current_session()
                                && let (Some(sort_state), Some(projection_state)) =
                                    (view.sort_state.clone(), view.projection_state.clone())
                            {
                                CollectionView::apply_query_options(
                                    view.state.clone(),
                                    session_key,
                                    sort_state,
                                    projection_state,
                                    window,
                                    cx,
                                );
                            }
                        }
                        InputEvent::Blur => {
                            let current = state.read(cx).value().to_string();
                            if current.trim().is_empty() {
                                state.update(cx, |input, cx| {
                                    input.set_value("{}".to_string(), window, cx);
                                });
                            }
                        }
                        _ => {}
                    }
                });
            self.sort_state = Some(sort_state);
            self.sort_subscription = Some(subscription);
        }

        if self.projection_state.is_none() {
            let projection_state = cx.new(|cx| {
                let mut state =
                    InputState::new(window, cx).placeholder("Projection (JSON)").clean_on_escape();
                state.set_value("{}".to_string(), window, cx);
                state
            });
            let subscription = cx.subscribe_in(
                &projection_state,
                window,
                move |view, state, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        if let Some(session_key) = view.view_model.current_session()
                            && let (Some(sort_state), Some(projection_state)) =
                                (view.sort_state.clone(), view.projection_state.clone())
                        {
                            CollectionView::apply_query_options(
                                view.state.clone(),
                                session_key,
                                sort_state,
                                projection_state,
                                window,
                                cx,
                            );
                        }
                    }
                    InputEvent::Blur => {
                        let current = state.read(cx).value().to_string();
                        if current.trim().is_empty() {
                            state.update(cx, |input, cx| {
                                input.set_value("{}".to_string(), window, cx);
                            });
                        }
                    }
                    _ => {}
                },
            );
            self.projection_state = Some(projection_state);
            self.projection_subscription = Some(subscription);
        }

        if self.input_session != session_key {
            self.input_session = session_key.clone();
            if let Some(filter_state) = self.filter_state.clone() {
                filter_state.update(cx, |state, cx| {
                    if filter_raw.trim().is_empty() {
                        state.set_value("{}".to_string(), window, cx);
                    } else {
                        state.set_value(filter_raw.clone(), window, cx);
                    }
                });
            }
            if let Some(sort_state) = self.sort_state.clone() {
                sort_state.update(cx, |state, cx| {
                    if sort_raw.trim().is_empty() {
                        state.set_value("{}".to_string(), window, cx);
                    } else {
                        state.set_value(sort_raw.clone(), window, cx);
                    }
                });
            }
            if let Some(projection_state) = self.projection_state.clone() {
                projection_state.update(cx, |state, cx| {
                    if projection_raw.trim().is_empty() {
                        state.set_value("{}".to_string(), window, cx);
                    } else {
                        state.set_value(projection_raw.clone(), window, cx);
                    }
                });
            }
        }

        let filter_state = self.filter_state.clone();
        let sort_state = self.sort_state.clone();
        let projection_state = self.projection_state.clone();

        let state_for_prev = self.state.clone();
        let state_for_next = self.state.clone();

        let mut key_context = String::from("Documents");
        match subview {
            CollectionSubview::Indexes => key_context.push_str(" Indexes"),
            CollectionSubview::Stats => key_context.push_str(" Stats"),
            CollectionSubview::Aggregation => key_context.push_str(" Aggregation"),
            CollectionSubview::Documents => {}
        }

        let mut root = div().key_context(key_context.as_str());
        root = self.bind_root_actions(root, cx);
        root = root.flex().flex_col().flex_1().h_full().bg(colors::bg_app()).child(
            self.render_header(
                &collection_name,
                &db_name,
                total,
                session_key.clone(),
                selected_doc,
                dirty_selected,
                is_loading,
                filter_state,
                filter_active,
                sort_state,
                projection_state,
                sort_active,
                projection_active,
                query_options_open,
                subview,
                stats_loading,
                aggregation.loading,
                window,
                cx,
            ),
        );

        match subview {
            CollectionSubview::Documents => {
                root = root.child(self.render_documents_subview(
                    &documents,
                    total,
                    display_page,
                    total_pages,
                    range_start,
                    range_end,
                    is_loading,
                    session_key,
                    state_for_prev,
                    state_for_next,
                    cx,
                ));
            }
            CollectionSubview::Indexes => {
                root = root.child(self.render_indexes_view(
                    indexes,
                    indexes_loading,
                    indexes_error,
                    session_key,
                ));
            }
            CollectionSubview::Stats => {
                root = root.child(self.render_stats_view(
                    stats,
                    stats_loading,
                    stats_error,
                    session_key,
                ));
            }
            CollectionSubview::Aggregation => {
                root =
                    root.child(self.render_aggregation_view(aggregation, session_key, window, cx));
            }
        }

        root
    }
}

impl CollectionView {
    fn render_stats_view(
        &self,
        stats: Option<CollectionStats>,
        stats_loading: bool,
        stats_error: Option<String>,
        session_key: Option<SessionKey>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .overflow_y_scrollbar()
            .p(spacing::lg())
            .child(Self::render_stats_row(
                stats,
                stats_loading,
                stats_error,
                session_key,
                self.state.clone(),
            ))
            .into_any_element()
    }
}
