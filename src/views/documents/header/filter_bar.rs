//! Filter bar and query options rendering for collection header.

use gpui::*;
use gpui_component::input::{Input, InputState};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::components::Button;
use crate::state::{AppState, SessionKey};
use crate::theme::{borders, colors, spacing};
use crate::views::documents::CollectionView;

/// Render the filter row with filter input and buttons.
#[allow(clippy::too_many_arguments)]
pub fn render_filter_row(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    filter_state: Option<Entity<InputState>>,
    filter_active: bool,
    sort_active: bool,
    projection_active: bool,
    query_options_open: bool,
) -> Div {
    let state_for_filter = state.clone();
    let state_for_clear = state.clone();
    let state_for_toggle = state.clone();

    let options_label =
        if sort_active || projection_active { "Options \u{2022}" } else { "Options" };
    let options_icon =
        Icon::new(if query_options_open { IconName::ChevronDown } else { IconName::ChevronRight })
            .xsmall();

    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .child(if let Some(filter_state) = filter_state.clone() {
            div()
                .flex_1()
                .min_w(px(240.0))
                .on_mouse_down(MouseButton::Left, |_, _window, cx| {
                    cx.stop_propagation();
                })
                .child(
                    Input::new(&filter_state)
                        .font_family(crate::theme::fonts::mono())
                        .w_full()
                        .disabled(session_key.is_none()),
                )
                .into_any_element()
        } else {
            div().flex_1().into_any_element()
        })
        .child(
            Button::new("apply-filter")
                .compact()
                .label("Filter")
                .disabled(session_key.is_none())
                .on_click({
                    let session_key = session_key.clone();
                    let filter_state = filter_state.clone();
                    let state_for_filter = state_for_filter.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        let Some(filter_state) = filter_state.clone() else {
                            return;
                        };
                        CollectionView::apply_filter(
                            state_for_filter.clone(),
                            session_key,
                            filter_state,
                            window,
                            cx,
                        );
                    }
                }),
        )
        .child(
            Button::new("clear-filter")
                .compact()
                .label("Clear")
                .disabled(session_key.is_none() || !filter_active)
                .on_click({
                    let session_key = session_key.clone();
                    let filter_state = filter_state.clone();
                    let state_for_clear = state_for_clear.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        let Some(filter_state) = filter_state.clone() else {
                            return;
                        };
                        filter_state.update(cx, |state, cx| {
                            state.set_value("{}".to_string(), window, cx);
                        });
                        CollectionView::apply_filter(
                            state_for_clear.clone(),
                            session_key,
                            filter_state,
                            window,
                            cx,
                        );
                    }
                }),
        )
        .child(
            Button::new("toggle-options")
                .ghost()
                .compact()
                .label(options_label)
                .icon(options_icon)
                .icon_right()
                .disabled(session_key.is_none())
                .on_click({
                    let session_key = session_key.clone();
                    let state_for_toggle = state_for_toggle.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        state_for_toggle.update(cx, |state, cx| {
                            let session = state.ensure_session(session_key.clone());
                            session.view.query_options_open = !session.view.query_options_open;
                            cx.notify();
                        });
                    }
                }),
        )
}

/// Render the query options panel (sort/projection).
pub fn render_query_options(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    sort_state: Option<Entity<InputState>>,
    projection_state: Option<Entity<InputState>>,
    sort_active: bool,
    projection_active: bool,
) -> Div {
    let state_for_query = state.clone();
    let state_for_clear = state.clone();

    div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .px(spacing::md())
        .py(spacing::sm())
        .bg(colors::bg_sidebar())
        .border_1()
        .border_color(colors::border_subtle())
        .rounded(borders::radius_sm())
        .child(render_query_option_row("Sort", sort_state.clone(), session_key.is_none()))
        .child(render_query_option_row("Project", projection_state.clone(), session_key.is_none()))
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(spacing::sm())
                .child(
                    Button::new("apply-query")
                        .compact()
                        .label("Apply")
                        .disabled(session_key.is_none())
                        .on_click({
                            let session_key = session_key.clone();
                            let sort_state = sort_state.clone();
                            let projection_state = projection_state.clone();
                            let state_for_query = state_for_query.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let Some(session_key) = session_key.clone() else {
                                    return;
                                };
                                let Some(sort_state) = sort_state.clone() else {
                                    return;
                                };
                                let Some(projection_state) = projection_state.clone() else {
                                    return;
                                };
                                CollectionView::apply_query_options(
                                    state_for_query.clone(),
                                    session_key,
                                    sort_state,
                                    projection_state,
                                    window,
                                    cx,
                                );
                            }
                        }),
                )
                .child(
                    Button::new("clear-query")
                        .compact()
                        .label("Clear")
                        .disabled(session_key.is_none() || (!sort_active && !projection_active))
                        .on_click({
                            let session_key = session_key.clone();
                            let sort_state = sort_state.clone();
                            let projection_state = projection_state.clone();
                            let state_for_clear = state_for_clear.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let Some(session_key) = session_key.clone() else {
                                    return;
                                };
                                let Some(sort_state) = sort_state.clone() else {
                                    return;
                                };
                                let Some(projection_state) = projection_state.clone() else {
                                    return;
                                };
                                sort_state.update(cx, |state, cx| {
                                    state.set_value("{}".to_string(), window, cx);
                                });
                                projection_state.update(cx, |state, cx| {
                                    state.set_value("{}".to_string(), window, cx);
                                });
                                CollectionView::apply_query_options(
                                    state_for_clear.clone(),
                                    session_key,
                                    sort_state,
                                    projection_state,
                                    window,
                                    cx,
                                );
                            }
                        }),
                ),
        )
}

/// Render a single query option row (label + input).
pub fn render_query_option_row(
    label: &str,
    state: Option<Entity<InputState>>,
    disabled: bool,
) -> AnyElement {
    let Some(state) = state else {
        return div().into_any_element();
    };

    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .child(
            div().w(px(72.0)).text_sm().text_color(colors::text_muted()).child(label.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .on_mouse_down(MouseButton::Left, |_, _window, cx| {
                    cx.stop_propagation();
                })
                .child(
                    Input::new(&state)
                        .font_family(crate::theme::fonts::mono())
                        .w_full()
                        .disabled(disabled),
                ),
        )
        .into_any_element()
}
