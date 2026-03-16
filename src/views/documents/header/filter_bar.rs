//! Filter bar and query options rendering for collection header.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::RopeExt as _;
use gpui_component::input::{Input, InputState};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::components::Button;
use crate::state::{AppCommands, AppState, SessionKey};
use crate::theme::{borders, spacing};
use crate::views::documents::CollectionView;

fn set_query_object_default(
    input: &mut InputState,
    window: &mut Window,
    cx: &mut Context<InputState>,
) {
    input.set_value("{}".to_string(), window, cx);
    let position = input.text().offset_to_position(1);
    input.set_cursor_position(position, window, cx);
}

/// Render the filter row with filter input and buttons.
#[allow(clippy::too_many_arguments)]
pub fn render_filter_row(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    filter_state: Option<Entity<InputState>>,
    sort_state: Option<Entity<InputState>>,
    filter_valid: bool,
    sort_valid: bool,
    filter_active: bool,
    sort_active: bool,
    projection_active: bool,
    query_options_open: bool,
    filter_builder_open: bool,
    explain_loading: bool,
    filter_error_message: Option<&str>,
    filter_dirty: bool,
    cx: &App,
) -> Div {
    let state_for_filter = state.clone();
    let state_for_clear = state.clone();
    let state_for_toggle = state.clone();
    let disabled = session_key.is_none();
    let segmented_border = if filter_active {
        cx.theme().primary.opacity(0.45)
    } else {
        cx.theme().sidebar_border.opacity(0.5)
    };

    let mut col = div().flex().flex_col().gap(px(4.0));

    col = col.child(
        div()
            .flex()
            .items_center()
            .gap(spacing::xs())
            .child({
                let mut bar = div()
                    .flex()
                    .items_center()
                    .flex_1()
                    .min_w(px(360.0))
                    .rounded(borders::radius_md())
                    .border_1()
                    .border_color(segmented_border)
                    .bg(cx.theme().secondary.opacity(0.14));

                if filter_active {
                    bar = bar.border_l_2().border_color(cx.theme().primary.opacity(0.65));
                }

                bar.child(render_query_segment(
                    "query-segment-find",
                    IconName::Search,
                    filter_state.clone(),
                    "find {}",
                    filter_valid,
                    filter_active,
                    disabled,
                    cx,
                ))
                .child(div().w(px(1.0)).h(px(16.0)).bg(cx.theme().sidebar_border.opacity(0.32)))
                .child(render_query_segment(
                    "query-segment-sort",
                    IconName::SortAscending,
                    sort_state.clone(),
                    "sort",
                    sort_valid,
                    false,
                    disabled,
                    cx,
                ))
            })
            .child({
                let mut run_btn = filter_action_button(
                    Button::new("apply-filter").compact(),
                    IconName::Search,
                    "Run",
                )
                .disabled(session_key.is_none() || !filter_valid);

                if filter_dirty && filter_valid {
                    run_btn = run_btn.active_style(cx.theme().primary.opacity(0.55));
                }

                run_btn.on_click({
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
                })
            })
            .child(
                filter_action_button(
                    Button::new("run-explain").compact(),
                    IconName::Info,
                    "Explain",
                )
                .disabled(session_key.is_none() || explain_loading)
                .on_click({
                    let session_key = session_key.clone();
                    let state = state.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        AppCommands::run_explain_for_session(state.clone(), session_key, cx);
                    }
                }),
            )
            .child(
                filter_action_button(
                    Button::new("clear-filter").compact(),
                    IconName::Close,
                    "Clear Find",
                )
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
                            state.set_value(String::new(), window, cx);
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
            .child({
                let mut options_button = Button::new("toggle-options")
                    .ghost()
                    .compact()
                    .icon(Icon::new(IconName::Settings).xsmall())
                    .tooltip("Projection options")
                    .disabled(session_key.is_none())
                    .on_click({
                        let session_key = session_key.clone();
                        let state_for_toggle = state_for_toggle.clone();
                        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            state_for_toggle.update(cx, |state, cx| {
                                state.toggle_query_options_open(&session_key);
                                cx.notify();
                            });
                        }
                    });
                if query_options_open || sort_active || projection_active {
                    options_button = options_button.active_style(cx.theme().secondary);
                }
                options_button
            })
            .child({
                let mut builder_button = Button::new("toggle-filter-builder")
                    .ghost()
                    .compact()
                    .icon(Icon::new(IconName::Braces).xsmall())
                    .tooltip("Filter Builder")
                    .disabled(session_key.is_none())
                    .on_click({
                        let session_key = session_key.clone();
                        let state = state.clone();
                        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            state.update(cx, |state, cx| {
                                state.toggle_filter_builder_open(&session_key);
                                cx.notify();
                            });
                        }
                    });
                if filter_builder_open {
                    builder_button = builder_button.active_style(cx.theme().primary.opacity(0.55));
                }
                builder_button
            }),
    );

    if let Some(err) = filter_error_message {
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .px(spacing::sm())
                .child(Icon::new(IconName::CircleX).size(px(12.0)).text_color(cx.theme().danger))
                .child(
                    div().text_xs().text_color(cx.theme().danger).truncate().child(err.to_string()),
                ),
        );
    }

    col
}

#[allow(clippy::too_many_arguments)]
fn render_query_segment(
    id: impl Into<ElementId>,
    icon: IconName,
    state: Option<Entity<InputState>>,
    placeholder: &'static str,
    valid: bool,
    active: bool,
    disabled: bool,
    cx: &App,
) -> impl IntoElement {
    let icon_color = if !valid {
        cx.theme().danger
    } else if active {
        cx.theme().primary
    } else {
        cx.theme().muted_foreground
    };

    let mut row = div()
        .id(id)
        .flex()
        .items_center()
        .gap(spacing::xs())
        .flex_1()
        .min_w(px(0.0))
        .px(spacing::sm())
        .py(px(2.0))
        .text_color(if valid { cx.theme().muted_foreground } else { cx.theme().danger })
        .when(!valid, |s| s.bg(cx.theme().danger.opacity(0.08)))
        .child(Icon::new(icon).xsmall().text_color(icon_color))
        .on_mouse_down(MouseButton::Left, |_, _window, cx| {
            cx.stop_propagation();
        });

    if let Some(state) = state {
        row = row.child(
            Input::new(&state)
                .small()
                .font_family(crate::theme::fonts::mono())
                .appearance(false)
                .w_full()
                .disabled(disabled),
        );
    } else {
        row = row.child(div().text_xs().child(placeholder));
    }

    row
}

/// Render the query options panel (sort/projection).
#[allow(clippy::too_many_arguments)]
pub fn render_query_options(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    sort_state: Option<Entity<InputState>>,
    projection_state: Option<Entity<InputState>>,
    sort_valid: bool,
    projection_valid: bool,
    sort_active: bool,
    projection_active: bool,
    cx: &App,
) -> Div {
    let state_for_query = state.clone();
    let state_for_clear = state.clone();

    let apply_disabled = session_key.is_none() || !sort_valid || !projection_valid;
    let disabled = session_key.is_none();
    let segmented_border = cx.theme().sidebar_border.opacity(0.5);

    div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .child(
            div()
                .flex()
                .items_center()
                .flex_1()
                .min_w(px(240.0))
                .rounded(borders::radius_md())
                .border_1()
                .border_color(segmented_border)
                .bg(cx.theme().secondary.opacity(0.14))
                .child(render_query_segment(
                    "query-segment-project",
                    IconName::Braces,
                    projection_state.clone(),
                    "project {}",
                    projection_valid,
                    projection_active,
                    disabled,
                    cx,
                )),
        )
        .child(
            filter_action_button(
                Button::new("apply-query").compact(),
                IconName::Check,
                "Apply options",
            )
            .disabled(apply_disabled)
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
            filter_action_button(
                Button::new("clear-query").compact(),
                IconName::Close,
                "Clear options",
            )
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
                        set_query_object_default(state, window, cx);
                    });
                    projection_state.update(cx, |state, cx| {
                        set_query_object_default(state, window, cx);
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
        )
}

fn filter_action_button(button: Button, icon: IconName, label: &'static str) -> Button {
    button.ghost().icon(Icon::new(icon).xsmall()).tooltip(label)
}
