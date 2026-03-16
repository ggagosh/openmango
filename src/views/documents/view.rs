use std::rc::Rc;

use crate::state::{AppCommands, CollectionStats, CollectionSubview, SchemaAnalysis, SessionKey};
use crate::theme::spacing;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::RopeExt as _;
use gpui_component::Sizable as _;
use gpui_component::calendar::{Calendar, CalendarEvent, CalendarState, Date};
use gpui_component::h_flex;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;

use crate::components::filter_builder::FilterBuilderPanel;

use super::CollectionView;
use super::header::render_stats_row;
use super::query::{
    filter_query_validation_error, format_filter_query, is_valid_query, normalized_filter_query,
    query_validation_error,
};
use super::query_completion::{
    FilterCompletionProvider, QueryCompletionProvider, QueryInputKind,
    is_query_input_in_string_or_comment,
};
use super::schema_filter_completion::SchemaFilterCompletionProvider;

fn collapse_to_single_line(s: &str) -> String {
    if !s.contains('\n') {
        return s.to_string();
    }
    s.lines().map(|l| l.trim()).collect::<Vec<_>>().join(" ")
}

fn parse_time_part(val: &str, max: u32) -> u32 {
    val.trim().parse::<u32>().unwrap_or(0).min(max)
}

fn subscribe_time_input(
    cx: &mut Context<'_, CollectionView>,
    window: &mut Window,
    state: &Entity<InputState>,
    max: u32,
) -> Subscription {
    cx.subscribe_in(
        state,
        window,
        move |_view, state, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                let val = state.read(cx).value().to_string();
                let digits: String = val.chars().filter(|c| c.is_ascii_digit()).take(2).collect();
                if digits != val {
                    state.update(cx, |input, cx| {
                        input.set_value(digits, window, cx);
                    });
                }
            }
            InputEvent::Blur => {
                let val = state.read(cx).value().to_string();
                if !val.is_empty() {
                    let clamped = format!("{:02}", parse_time_part(&val, max));
                    if clamped != val {
                        state.update(cx, |input, cx| {
                            input.set_value(clamped, window, cx);
                        });
                    }
                }
            }
            _ => {}
        },
    )
}

fn move_cursor_inside_query_object(
    input: &mut InputState,
    window: &mut Window,
    cx: &mut Context<InputState>,
) {
    if input.value().trim() == "{}" {
        let position = input.text().offset_to_position(1);
        input.set_cursor_position(position, window, cx);
    }
}

fn set_filter_object_default(
    input: &mut InputState,
    window: &mut Window,
    cx: &mut Context<InputState>,
) {
    input.set_value("{}".to_string(), window, cx);
    move_cursor_inside_query_object(input, window, cx);
}

impl Render for CollectionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.search_state.is_none() {
            let search_state = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Find in document (Cmd/Ctrl+F)")
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
            selected_docs,
            selected_count,
            any_selected_dirty,
            filter_raw,
            sort_raw,
            projection_raw,
            query_options_open,
            filter_builder_open,
            subview,
            stats,
            stats_loading,
            stats_error,
            indexes,
            indexes_loading,
            indexes_error,
            aggregation,
            explain,
            schema,
            schema_loading,
            schema_error,
            schema_selected_field,
            schema_expanded_fields,
            schema_filter,
        ) = if let Some(snapshot) = snapshot {
            (
                snapshot.items,
                snapshot.total,
                snapshot.page,
                snapshot.per_page,
                snapshot.is_loading,
                snapshot.selected_doc,
                snapshot.selected_docs,
                snapshot.selected_count,
                snapshot.any_selected_dirty,
                snapshot.filter_raw,
                snapshot.sort_raw,
                snapshot.projection_raw,
                snapshot.query_options_open,
                snapshot.filter_builder_open,
                snapshot.subview,
                snapshot.stats,
                snapshot.stats_loading,
                snapshot.stats_error,
                snapshot.indexes,
                snapshot.indexes_loading,
                snapshot.indexes_error,
                snapshot.aggregation,
                snapshot.explain,
                snapshot.schema,
                snapshot.schema_loading,
                snapshot.schema_error,
                snapshot.schema_selected_field,
                snapshot.schema_expanded_fields,
                snapshot.schema_filter,
            )
        } else {
            (
                Vec::new(),
                0,
                0,
                50,
                false,
                None,
                std::collections::HashSet::new(),
                0,
                false,
                String::new(),
                String::new(),
                String::new(),
                false,
                false,
                CollectionSubview::Documents,
                None::<CollectionStats>,
                false,
                None,
                None,
                false,
                None,
                Default::default(),
                Default::default(),
                None::<SchemaAnalysis>,
                false,
                None::<String>,
                None::<String>,
                std::collections::HashSet::new(),
                String::new(),
            )
        };
        let filter_active = !matches!(filter_raw.trim(), "" | "{}");
        let sort_active = !matches!(sort_raw.trim(), "" | "{}");
        let projection_active = !matches!(projection_raw.trim(), "" | "{}");
        let filter_valid = self.filter_error_message.is_none();

        let filter_dirty = if let Some(ref fs) = self.filter_state {
            let input_text = normalized_filter_query(fs.read(cx).value().as_ref());
            let applied = normalized_filter_query(&filter_raw);
            input_text != applied
        } else {
            false
        };
        self.filter_dirty = filter_dirty;
        let sort_valid = !self.sort_error;
        let projection_valid = !self.projection_error;
        let per_page_u64 = per_page.max(1) as u64;
        let total_pages = total.div_ceil(per_page_u64).max(1);
        let display_page = page.min(total_pages.saturating_sub(1));
        let range_start = if total == 0 { 0 } else { display_page * per_page_u64 + 1 };
        let range_end = if total == 0 { 0 } else { ((display_page + 1) * per_page_u64).min(total) };

        if self.filter_state.is_none() {
            let filter_state = cx.new(|cx| {
                let mut state = InputState::new(window, cx)
                    .code_editor("javascript")
                    .multi_line(false)
                    .submit_on_enter(true)
                    .placeholder("find {}")
                    .clean_on_escape();
                state.lsp.completion_provider =
                    Some(Rc::new(FilterCompletionProvider::new(self.state.clone())));
                state.set_value("{}".to_string(), window, cx);
                state
            });
            let subscription =
                cx.subscribe_in(&filter_state, window, move |view, state, event, window, cx| {
                    match event {
                        InputEvent::Change => {
                            if view.syncing_query_inputs {
                                return;
                            }
                            let (current_text, cursor) = {
                                let input = state.read(cx);
                                (input.value().to_string(), input.cursor())
                            };
                            let in_string_or_comment =
                                is_query_input_in_string_or_comment(&current_text, cursor);
                            if view.filter_auto_pair.try_auto_pair(
                                state,
                                in_string_or_comment,
                                window,
                                cx,
                            ) {
                                return;
                            }
                            let (text, cursor) = {
                                let input = state.read(cx);
                                (input.value().to_string(), input.cursor())
                            };
                            view.filter_auto_pair.sync(&text);
                            let next_error = filter_query_validation_error(&text);
                            let validation_changed = view.filter_error_message != next_error;
                            view.filter_error_message = next_error;

                            // Detect ISODate("") or Date("") pattern for calendar popup.
                            let show_calendar = cursor <= text.len()
                                && (text[..cursor].ends_with("ISODate(\"")
                                    || text[..cursor].ends_with("Date(\""))
                                && text[cursor..].starts_with("\")");
                            let mut should_notify = validation_changed;
                            if show_calendar {
                                view.calendar_insert_offset = Some(cursor);
                                if !view.calendar_open {
                                    view.calendar_open = true;
                                    for s in [
                                        &view.calendar_hour,
                                        &view.calendar_minute,
                                        &view.calendar_second,
                                    ]
                                    .into_iter()
                                    .flatten()
                                    {
                                        s.update(cx, |input, cx| {
                                            input.set_value(String::new(), window, cx);
                                        });
                                    }
                                    should_notify = true;
                                }
                            } else if view.calendar_open {
                                view.calendar_open = false;
                                view.calendar_insert_offset = None;
                                should_notify = true;
                            }
                            if should_notify {
                                cx.notify();
                            }
                        }
                        InputEvent::PressEnter { .. } => {
                            let raw = state.read(cx).value().to_string();
                            match format_filter_query(&raw) {
                                Ok(formatted) => {
                                    state.update(cx, |input, cx| {
                                        input.set_value(formatted.clone(), window, cx);
                                        move_cursor_inside_query_object(input, window, cx);
                                    });
                                    view.filter_auto_pair.sync(&formatted);
                                    view.filter_error_message = None;
                                    if let Some(session_key) = view.view_model.current_session() {
                                        CollectionView::apply_filter(
                                            view.state.clone(),
                                            session_key,
                                            state.clone(),
                                            window,
                                            cx,
                                        );
                                    }
                                    if let Some(panel) = view.filter_builder_panel.clone()
                                        && let Ok(doc) =
                                            crate::bson::parse_document_from_json(formatted.trim())
                                    {
                                        panel.update(cx, |p, cx| {
                                            p.populate_from_document(&doc, window, cx);
                                        });
                                    }
                                    cx.notify();
                                }
                                Err(err) => {
                                    view.filter_error_message = Some(err);
                                    cx.notify();
                                }
                            }
                        }
                        InputEvent::Blur => {
                            let current = state.read(cx).value().to_string();
                            if current.trim().is_empty() {
                                state.update(cx, |input, cx| {
                                    set_filter_object_default(input, window, cx);
                                });
                                view.filter_auto_pair.sync("{}");
                                let had_error = view.filter_error_message.take().is_some();
                                if had_error {
                                    cx.notify();
                                }
                            }
                        }
                        InputEvent::Focus => {
                            state.update(cx, |input, cx| {
                                if input.value().trim().is_empty() {
                                    set_filter_object_default(input, window, cx);
                                } else {
                                    move_cursor_inside_query_object(input, window, cx);
                                }
                            });
                            if let Some(session_key) = view.view_model.current_session() {
                                let should_analyze_schema = {
                                    let state_ref = view.state.read(cx);
                                    state_ref.session(&session_key).is_some_and(|session| {
                                        session.data.schema.is_none()
                                            && state_ref.collection_meta(&session_key).is_none()
                                            && !session.data.schema_loading
                                            && session.data.schema_error.is_none()
                                    })
                                };
                                if should_analyze_schema {
                                    AppCommands::analyze_collection_schema(
                                        view.state.clone(),
                                        session_key,
                                        cx,
                                    );
                                }
                            }
                        }
                    }
                });
            let calendar_state = cx.new(|cx| CalendarState::new(window, cx));
            let hour_state = cx.new(|cx| InputState::new(window, cx).placeholder("HH"));
            let minute_state = cx.new(|cx| InputState::new(window, cx).placeholder("MM"));
            let second_state = cx.new(|cx| InputState::new(window, cx).placeholder("SS"));
            self._subscriptions.push(subscribe_time_input(cx, window, &hour_state, 23));
            self._subscriptions.push(subscribe_time_input(cx, window, &minute_state, 59));
            self._subscriptions.push(subscribe_time_input(cx, window, &second_state, 59));
            let filter_for_calendar = filter_state.clone();
            let hour_for_calendar = hour_state.clone();
            let minute_for_calendar = minute_state.clone();
            let second_for_calendar = second_state.clone();
            let calendar_subscription = cx.subscribe_in(
                &calendar_state,
                window,
                move |view: &mut CollectionView, _calendar, event: &CalendarEvent, _window, cx| {
                    let CalendarEvent::Selected(date) = event;
                    if let Date::Single(Some(naive_date)) = date
                        && let Some(offset) = view.calendar_insert_offset
                    {
                        let h = parse_time_part(&hour_for_calendar.read(cx).value(), 23);
                        let m = parse_time_part(&minute_for_calendar.read(cx).value(), 59);
                        let s = parse_time_part(&second_for_calendar.read(cx).value(), 59);
                        let formatted = format!(
                            "{}T{:02}:{:02}:{:02}.000Z",
                            naive_date.format("%Y-%m-%d"),
                            h,
                            m,
                            s,
                        );
                        let insert_offset = offset;
                        filter_for_calendar.update(cx, |input, cx| {
                            let text = input.value().to_string();
                            if insert_offset <= text.len() {
                                let mut new_text =
                                    String::with_capacity(text.len() + formatted.len());
                                new_text.push_str(&text[..insert_offset]);
                                new_text.push_str(&formatted);
                                new_text.push_str(&text[insert_offset..]);
                                input.set_value(new_text, _window, cx);
                                let position = input
                                    .text()
                                    .offset_to_position(insert_offset + formatted.len());
                                input.set_cursor_position(position, _window, cx);
                            }
                        });
                    }
                    view.calendar_open = false;
                    view.calendar_insert_offset = None;
                    cx.notify();
                },
            );
            self.filter_state = Some(filter_state);
            self.filter_subscription = Some(subscription);
            self.calendar_state = Some(calendar_state);
            self.calendar_hour = Some(hour_state);
            self.calendar_minute = Some(minute_state);
            self.calendar_second = Some(second_state);
            self._subscriptions.push(calendar_subscription);
        }

        if self.sort_state.is_none() {
            let sort_state = cx.new(|cx| {
                let mut state = InputState::new(window, cx)
                    .code_editor("javascript")
                    .multi_line(false)
                    .submit_on_enter(true)
                    .placeholder("sort")
                    .clean_on_escape();
                state.lsp.completion_provider = Some(Rc::new(QueryCompletionProvider::new(
                    self.state.clone(),
                    QueryInputKind::Sort,
                )));
                state
            });
            let subscription =
                cx.subscribe_in(&sort_state, window, move |view, state, event, window, cx| {
                    match event {
                        InputEvent::Change => {
                            if view.syncing_query_inputs {
                                return;
                            }
                            let (current_text, cursor) = {
                                let input = state.read(cx);
                                (input.value().to_string(), input.cursor())
                            };
                            let in_string_or_comment =
                                is_query_input_in_string_or_comment(&current_text, cursor);
                            if view.sort_auto_pair.try_auto_pair(
                                state,
                                in_string_or_comment,
                                window,
                                cx,
                            ) {
                                return;
                            }
                            let raw = state.read(cx).value().to_string();
                            view.sort_auto_pair.sync(&raw);
                            let next_error = query_validation_error(&raw).is_some();
                            if view.sort_error != next_error {
                                view.sort_error = next_error;
                                cx.notify();
                            } else {
                                view.sort_error = next_error;
                            }
                        }
                        InputEvent::PressEnter { .. } => {
                            let raw = state.read(cx).value().to_string();
                            if !is_valid_query(&raw) {
                                view.sort_error = true;
                                cx.notify();
                                return;
                            }
                            view.sort_error = false;
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
                                view.sort_auto_pair.sync("");
                            }
                        }
                        InputEvent::Focus => {
                            state.update(cx, |input, cx| {
                                move_cursor_inside_query_object(input, window, cx)
                            });
                        }
                    }
                });
            self.sort_state = Some(sort_state);
            self.sort_subscription = Some(subscription);
        }

        if self.projection_state.is_none() {
            let projection_state = cx.new(|cx| {
                let mut state = InputState::new(window, cx)
                    .code_editor("javascript")
                    .multi_line(false)
                    .submit_on_enter(true)
                    .placeholder("project {}")
                    .clean_on_escape();
                state.lsp.completion_provider = Some(Rc::new(QueryCompletionProvider::new(
                    self.state.clone(),
                    QueryInputKind::Projection,
                )));
                state
            });
            let subscription = cx.subscribe_in(
                &projection_state,
                window,
                move |view, state, event, window, cx| match event {
                    InputEvent::Change => {
                        if view.syncing_query_inputs {
                            return;
                        }
                        let (current_text, cursor) = {
                            let input = state.read(cx);
                            (input.value().to_string(), input.cursor())
                        };
                        let in_string_or_comment =
                            is_query_input_in_string_or_comment(&current_text, cursor);
                        if view.projection_auto_pair.try_auto_pair(
                            state,
                            in_string_or_comment,
                            window,
                            cx,
                        ) {
                            return;
                        }
                        let raw = state.read(cx).value().to_string();
                        view.projection_auto_pair.sync(&raw);
                        let next_error = query_validation_error(&raw).is_some();
                        if view.projection_error != next_error {
                            view.projection_error = next_error;
                            cx.notify();
                        } else {
                            view.projection_error = next_error;
                        }
                    }
                    InputEvent::PressEnter { .. } => {
                        let raw = state.read(cx).value().to_string();
                        if !is_valid_query(&raw) {
                            view.projection_error = true;
                            cx.notify();
                            return;
                        }
                        view.projection_error = false;
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
                            view.projection_auto_pair.sync("");
                        }
                    }
                    InputEvent::Focus => {
                        state.update(cx, |input, cx| {
                            move_cursor_inside_query_object(input, window, cx)
                        });
                    }
                },
            );
            self.projection_state = Some(projection_state);
            self.projection_subscription = Some(subscription);
        }

        if self.schema_filter_state.is_none() {
            let schema_filter_state = cx.new(|cx| {
                let mut state = InputState::new(window, cx)
                    .code_editor("text")
                    .line_number(false)
                    .auto_indent(false)
                    .submit_on_enter(true)
                    .placeholder("Filter fields...")
                    .clean_on_escape();
                state.lsp.completion_provider =
                    Some(Rc::new(SchemaFilterCompletionProvider::new(self.state.clone())));
                state
            });
            let subscription = cx.subscribe_in(
                &schema_filter_state,
                window,
                move |view, state, event, _window, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }

                    let Some(session_key) = view.view_model.current_session() else {
                        return;
                    };
                    if view.state.read(cx).session_subview(&session_key)
                        != Some(CollectionSubview::Schema)
                    {
                        return;
                    }

                    let raw = state.read(cx).value().to_string();
                    view.state.update(cx, |state, cx| {
                        state.set_schema_filter(&session_key, raw.clone());
                        cx.notify();
                    });
                },
            );
            self.schema_filter_state = Some(schema_filter_state);
            self.schema_filter_subscription = Some(subscription);
        }

        if self.input_session != session_key {
            self.input_session = session_key.clone();
            self.syncing_query_inputs = true;
            if let Some(filter_state) = self.filter_state.clone() {
                let val = if filter_raw.trim().is_empty() {
                    "{}".to_string()
                } else {
                    collapse_to_single_line(&filter_raw)
                };
                filter_state.update(cx, |state, cx| {
                    state.set_value(val.clone(), window, cx);
                });
                self.filter_auto_pair.sync(&val);
                self.filter_error_message = filter_query_validation_error(&val);
            }
            if let Some(sort_state) = self.sort_state.clone() {
                let val = sort_raw.clone();
                sort_state.update(cx, |state, cx| {
                    state.set_value(val.clone(), window, cx);
                });
                self.sort_auto_pair.sync(&val);
                self.sort_error = query_validation_error(&val).is_some();
            }
            if let Some(projection_state) = self.projection_state.clone() {
                let val = projection_raw.clone();
                projection_state.update(cx, |state, cx| {
                    state.set_value(val.clone(), window, cx);
                });
                self.projection_auto_pair.sync(&val);
                self.projection_error = query_validation_error(&val).is_some();
            }
            self.syncing_query_inputs = false;
        } else if let Some(filter_state) = self.filter_state.clone() {
            // Sync filter input when filter_raw was changed externally (e.g. AI "Open Collection").
            // Only sync when the input is not focused to avoid overwriting the user's typing.
            let expected = if filter_raw.trim().is_empty() {
                "{}".to_string()
            } else {
                collapse_to_single_line(&filter_raw)
            };
            let current = filter_state.read(cx).value().to_string();
            let is_focused = filter_state.read(cx).focus_handle(cx).is_focused(window);
            if !is_focused && !self.calendar_open && current != expected {
                self.syncing_query_inputs = true;
                filter_state.update(cx, |state, cx| {
                    state.set_value(expected.clone(), window, cx);
                });
                self.filter_auto_pair.sync(&expected);
                self.filter_error_message = filter_query_validation_error(&expected);
                self.syncing_query_inputs = false;
            }
        }

        if self.schema_filter_session != session_key {
            self.schema_filter_session = session_key.clone();
            if let Some(schema_filter_state) = self.schema_filter_state.clone() {
                let val = schema_filter.clone();
                schema_filter_state.update(cx, |state, cx| {
                    state.set_value(val, window, cx);
                });
            }
        } else if let Some(schema_filter_state) = self.schema_filter_state.clone() {
            let current = schema_filter_state.read(cx).value().to_string();
            if current != schema_filter {
                let val = schema_filter.clone();
                schema_filter_state.update(cx, |state, cx| {
                    state.set_value(val, window, cx);
                });
            }
        }

        let filter_state = self.filter_state.clone();
        let sort_state = self.sort_state.clone();
        let projection_state = self.projection_state.clone();
        let schema_filter_state = self.schema_filter_state.clone();

        let per_page_i64 = per_page;

        let mut key_context = String::from("Documents");
        match subview {
            CollectionSubview::Indexes => key_context.push_str(" Indexes"),
            CollectionSubview::Stats => key_context.push_str(" Stats"),
            CollectionSubview::Aggregation => key_context.push_str(" Aggregation"),
            CollectionSubview::Schema => key_context.push_str(" Schema"),
            CollectionSubview::Documents => {}
        }

        let col_visibility_search = self.view_model.ensure_col_visibility_search(window, cx);

        let mut root = div().key_context(key_context.as_str());
        root = self.bind_root_actions(root, cx);
        let view_entity = cx.entity();
        root = root.flex().flex_col().flex_1().h_full().bg(cx.theme().background).child(
            div()
                .relative()
                .child(self.render_header(
                    &collection_name,
                    &db_name,
                    total,
                    session_key.clone(),
                    selected_doc,
                    selected_count,
                    any_selected_dirty,
                    is_loading,
                    filter_state,
                    filter_valid,
                    filter_active,
                    sort_state,
                    projection_state,
                    sort_valid,
                    projection_valid,
                    sort_active,
                    projection_active,
                    query_options_open,
                    filter_builder_open,
                    subview,
                    stats_loading,
                    aggregation.loading,
                    explain.loading,
                    schema_loading,
                    col_visibility_search,
                    window,
                    cx,
                ))
                .when_some(
                    self.calendar_open.then_some(self.calendar_state.clone()).flatten(),
                    |this, calendar_state| {
                        let border = cx.theme().border;
                        let popover_bg = cx.theme().popover;
                        let popover_fg = cx.theme().popover_foreground;
                        let muted_fg = cx.theme().muted_foreground;
                        let hour = self.calendar_hour.clone();
                        let minute = self.calendar_minute.clone();
                        let second = self.calendar_second.clone();
                        this.child(
                            deferred(
                                anchored().snap_to_window_with_margin(px(8.)).child(
                                    div()
                                        .occlude()
                                        .mt_1p5()
                                        .p_3()
                                        .border_1()
                                        .border_color(border)
                                        .shadow_lg()
                                        .rounded(px(8.))
                                        .bg(popover_bg)
                                        .text_color(popover_fg)
                                        .on_mouse_up_out(MouseButton::Left, {
                                            let view_entity = view_entity.clone();
                                            move |_, _, cx| {
                                                view_entity.update(cx, |view, cx| {
                                                    view.calendar_open = false;
                                                    view.calendar_insert_offset = None;
                                                    cx.notify();
                                                });
                                            }
                                        })
                                        .child(Calendar::new(&calendar_state).number_of_months(1))
                                        .child(
                                            h_flex()
                                                .mt_2()
                                                .pt_2()
                                                .border_t_1()
                                                .border_color(border)
                                                .items_center()
                                                .justify_center()
                                                .gap_1()
                                                .when_some(hour, |this, h| {
                                                    this.child(Input::new(&h).small().w(px(42.)))
                                                })
                                                .child(
                                                    div().text_color(muted_fg).text_sm().child(":"),
                                                )
                                                .when_some(minute, |this, m| {
                                                    this.child(Input::new(&m).small().w(px(42.)))
                                                })
                                                .child(
                                                    div().text_color(muted_fg).text_sm().child(":"),
                                                )
                                                .when_some(second, |this, s| {
                                                    this.child(Input::new(&s).small().w(px(42.)))
                                                }),
                                        ),
                                ),
                            )
                            .with_priority(2),
                        )
                    },
                ),
        );

        let content = match subview {
            CollectionSubview::Documents => self.render_documents_subview(
                &documents,
                total,
                display_page,
                total_pages,
                per_page_i64,
                range_start,
                range_end,
                is_loading,
                session_key.clone(),
                selected_docs,
                window,
                cx,
            ),
            CollectionSubview::Indexes => self.render_indexes_view(
                indexes,
                indexes_loading,
                indexes_error,
                session_key.clone(),
                cx,
            ),
            CollectionSubview::Stats => {
                self.render_stats_view(stats, stats_loading, stats_error, session_key.clone(), cx)
            }
            CollectionSubview::Aggregation => {
                self.render_aggregation_view(aggregation, session_key.clone(), window, cx)
            }
            CollectionSubview::Schema => self.render_schema_view(
                schema,
                schema_loading,
                schema_error,
                schema_selected_field,
                schema_expanded_fields,
                schema_filter,
                schema_filter_state,
                session_key.clone(),
                cx,
            ),
        };

        let explain_layer =
            self.render_explain_modal_layer(&explain, session_key.clone(), subview, cx);

        let show_builder = filter_builder_open && subview == CollectionSubview::Documents;
        if show_builder
            && let Some(sk) = session_key.clone()
            && let Some(filter_state) = self.filter_state.clone()
        {
            let needs_create = self.filter_builder_panel.is_none()
                || self.filter_builder_session.as_ref() != Some(&sk);
            if needs_create {
                let existing_filter = filter_raw.trim().to_string();
                let panel = cx.new(|cx| {
                    let mut p = FilterBuilderPanel::new(
                        self.state.clone(),
                        sk.clone(),
                        filter_state.clone(),
                        window,
                        cx,
                    );
                    if !existing_filter.is_empty()
                        && existing_filter != "{}"
                        && let Ok(doc) = crate::bson::parse_document_from_json(&existing_filter)
                    {
                        p.populate_from_document(&doc, window, cx);
                    }
                    p
                });
                self.filter_builder_panel = Some(panel);
                self.filter_builder_session = Some(sk);
            }
        }

        let content_with_builder = if show_builder {
            if let Some(panel) = self.filter_builder_panel.clone() {
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(div().flex().flex_col().flex_1().min_w(px(0.0)).child(content))
                    .child(div().w(px(560.0)).flex_shrink_0().h_full().child(panel))
                    .into_any_element()
            } else {
                content
            }
        } else {
            content
        };

        root = root.relative().child(content_with_builder).child(explain_layer);

        root
    }
}

impl CollectionView {
    #[allow(clippy::too_many_arguments)]
    fn render_schema_view(
        &mut self,
        schema: Option<SchemaAnalysis>,
        schema_loading: bool,
        schema_error: Option<String>,
        selected_field: Option<String>,
        expanded_fields: std::collections::HashSet<String>,
        schema_filter: String,
        schema_filter_state: Option<Entity<InputState>>,
        session_key: Option<SessionKey>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use super::views::schema_view::render_schema_panel;
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .child(render_schema_panel(
                schema,
                schema_loading,
                schema_error,
                selected_field,
                expanded_fields,
                schema_filter,
                schema_filter_state,
                session_key,
                self.state.clone(),
                cx,
            ))
            .into_any_element()
    }

    fn render_stats_view(
        &self,
        stats: Option<CollectionStats>,
        stats_loading: bool,
        stats_error: Option<String>,
        session_key: Option<SessionKey>,
        cx: &App,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .overflow_y_scrollbar()
            .p(spacing::lg())
            .child(render_stats_row(
                stats,
                stats_loading,
                stats_error,
                session_key,
                self.state.clone(),
                cx,
            ))
            .into_any_element()
    }
}
