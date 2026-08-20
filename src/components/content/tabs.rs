use std::collections::HashMap;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::scroll::ScrollbarHandle as _;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};

use crate::actions::model::ActionStatus;
use crate::components::{ConnectionIdentity, connection_identity_badge, request_unsaved_action};
use crate::keyboard::FocusContent;
use crate::state::{
    ActiveTab, AppState, AppearanceSettings, IslandsTabStyle, SessionKey, TabKey, UnsavedScope,
    View,
};
use crate::theme::{borders, colors, islands, spacing};
use crate::views::{
    AgentActivityView, ChangelogView, CollectionView, DatabaseView, ForgeView, SettingsView,
    TransferView,
};

const OPEN_TAB_MAX_WIDTH: f32 = 260.0;
const OPEN_TAB_LABEL_MAX_WIDTH: f32 = 210.0;

fn request_close_tab(state: Entity<AppState>, tab: TabKey, window: &mut Window, cx: &mut App) {
    let state_for_close = state.clone();
    request_unsaved_action(
        state,
        UnsavedScope::Tab(tab.clone()),
        window,
        cx,
        move |_window, cx| {
            state_for_close.update(cx, |state, cx| {
                if let Some(index) =
                    state.open_tabs().iter().position(|candidate| candidate == &tab)
                {
                    state.close_tab(index, cx);
                }
            });
        },
    );
}

fn request_close_preview(
    state: Entity<AppState>,
    session_key: SessionKey,
    window: &mut Window,
    cx: &mut App,
) {
    let state_for_close = state.clone();
    request_unsaved_action(
        state,
        UnsavedScope::Preview(session_key.clone()),
        window,
        cx,
        move |_window, cx| {
            state_for_close.update(cx, |state, cx| {
                if state.preview_tab() == Some(&session_key) {
                    state.close_preview_tab(cx);
                }
            });
        },
    );
}

fn tab_strip_height(appearance: &AppearanceSettings) -> Pixels {
    match appearance.islands.tab_style {
        IslandsTabStyle::Islands => px(36.0),
        IslandsTabStyle::Segmented => px(32.0),
        IslandsTabStyle::Underline => px(38.0),
    }
}

pub(crate) struct OpenTabsBar {
    state: Entity<AppState>,
    tabs_scroll_handle: ScrollHandle,
    last_seen_open_tab_count: usize,
    pending_scroll_to_end_frames: u8,
    _subscriptions: Vec<Subscription>,
}

pub(crate) struct TabsHost<'a> {
    pub(crate) state: Entity<AppState>,
    pub(crate) tabs_bar: Entity<OpenTabsBar>,
    pub(crate) current_view: View,
    pub(crate) has_collection: bool,
    pub(crate) collection_view: Option<&'a Entity<CollectionView>>,
    pub(crate) database_view: Option<&'a Entity<DatabaseView>>,
    pub(crate) transfer_view: Option<&'a Entity<TransferView>>,
    pub(crate) forge_view: Option<&'a Entity<ForgeView>>,
    pub(crate) agent_activity_view: Option<&'a Entity<AgentActivityView>>,
    pub(crate) settings_view: Option<&'a Entity<SettingsView>>,
    pub(crate) changelog_view: Option<&'a Entity<ChangelogView>>,
}

#[derive(Clone)]
struct DraggedOpenTab {
    from_index: usize,
    label: SharedString,
}

impl Render for DraggedOpenTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(spacing::sm())
            .py(px(4.0))
            .rounded(borders::radius_sm())
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().tab_active)
            .text_sm()
            .text_color(cx.theme().tab_active_foreground)
            .child(self.label.clone())
    }
}

fn scroll_tabs_by(scroll_handle: &ScrollHandle, delta_x: Pixels) {
    let mut offset = scroll_handle.offset();
    let viewport = scroll_handle.bounds().size.width;
    let content = scroll_handle.content_size().width;
    let min_x = (viewport - content).min(px(0.0));

    offset.x += delta_x;
    if offset.x > px(0.0) {
        offset.x = px(0.0);
    }
    if offset.x < min_x {
        offset.x = min_x;
    }

    scroll_handle.set_offset(offset);
}

impl OpenTabsBar {
    pub(crate) fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let last_seen_open_tab_count = state.read(cx).open_tabs().len();
        Self {
            state: state.clone(),
            tabs_scroll_handle: ScrollHandle::new(),
            last_seen_open_tab_count,
            pending_scroll_to_end_frames: 0,
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
        }
    }
}

impl Render for OpenTabsBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (
            appearance,
            tabs,
            active_tab,
            preview_tab,
            dirty_tabs,
            current_view,
            connection_identities,
            pending_agent_actions,
        ) = {
            let state_ref = self.state.read(cx);
            (
                state_ref.settings.appearance.clone(),
                state_ref.open_tabs().to_vec(),
                state_ref.active_tab(),
                state_ref.preview_tab().cloned(),
                state_ref.dirty_tabs().clone(),
                state_ref.current_view,
                state_ref
                    .connections_snapshot()
                    .into_iter()
                    .map(|connection| (connection.id, ConnectionIdentity::from(&connection)))
                    .collect::<HashMap<_, _>>(),
                state_ref
                    .action_broker()
                    .list_all()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|action| action.status == ActionStatus::PendingApproval)
                    .count(),
            )
        };
        let islands_tab_variant = appearance.islands.tab_style == IslandsTabStyle::Islands;
        let selected_index = match active_tab {
            ActiveTab::Preview => tabs.len(),
            ActiveTab::Index(index) => index.min(tabs.len().saturating_sub(1)),
            ActiveTab::None => 0,
        };

        let tab_count = tabs.len();
        if tab_count > self.last_seen_open_tab_count {
            // Reveal the newest tab once; avoid extra render churn on regular tab switching.
            self.pending_scroll_to_end_frames = 1;
        }
        self.last_seen_open_tab_count = tab_count;

        let scroll_to_end_once = self.pending_scroll_to_end_frames > 0;
        if self.pending_scroll_to_end_frames > 0 {
            self.pending_scroll_to_end_frames -= 1;
        }

        let state = self.state.clone();
        let scroll_handle = self.tabs_scroll_handle.clone();
        if scroll_to_end_once {
            scroll_handle.scroll_to_item(selected_index);
        }

        let tab_bar = islands::tab_bar(TabBar::new("collection-tabs"), &appearance)
            .small()
            .min_w(px(0.0))
            .track_scroll(&scroll_handle)
            .selected_index(selected_index)
            .menu(false)
            .last_empty_space(
                div()
                    .id("collection-tabs-end-drop")
                    .h_full()
                    .min_w(px(24.0))
                    .flex_grow()
                    .can_drop(move |value, _window, _cx| {
                        value.downcast_ref::<DraggedOpenTab>().is_some()
                    })
                    .drag_over::<DraggedOpenTab>(|style, _drag, _window, cx| {
                        style.bg(cx.theme().drop_target)
                    })
                    .on_drop({
                        let state = self.state.clone();
                        move |drag: &DraggedOpenTab, _window, cx| {
                            let to = state.read(cx).open_tabs().len();
                            state.update(cx, |state, cx| {
                                state.move_open_tab(drag.from_index, to, cx);
                                state.set_tab_drag_over(None);
                                let final_index = state.open_tabs().len().saturating_sub(1);
                                state.select_tab(final_index, cx);
                            });
                        }
                    }),
            )
            .on_click(move |index, window, cx| {
                let index = *index;
                state.update(cx, |state, cx| {
                    if index < state.open_tabs().len() {
                        state.select_tab(index, cx);
                    } else {
                        state.select_preview_tab(cx);
                    }
                });
                window.dispatch_action(Box::new(FocusContent), cx);
            })
            .children(
                tabs.iter()
                    .enumerate()
                    .map(|(index, tab)| {
                        let (label, is_dirty) = match tab {
                            TabKey::Collection(tab) => (
                                format!("{}/{}", tab.database, tab.collection),
                                dirty_tabs.contains(tab),
                            ),
                            TabKey::Database(tab) => (tab.database.clone(), false),
                            TabKey::Transfer(tab) => {
                                (self.state.read(cx).transfer_tab_label(tab.id), false)
                            }
                            TabKey::Forge(tab) => {
                                (self.state.read(cx).forge_tab_label(tab.id), false)
                            }
                            TabKey::AgentActivity => ("Agent Activity".to_string(), false),
                            TabKey::Settings => ("Settings".to_string(), false),
                            TabKey::Changelog => ("What's New".to_string(), false),
                        };
                        let icon_name = match tab {
                            TabKey::Collection(_) => IconName::Braces,
                            TabKey::Database(_) => IconName::LayoutDashboard,
                            TabKey::Transfer(_) => IconName::Download,
                            TabKey::Forge(_) => IconName::SquareTerminal,
                            TabKey::AgentActivity => IconName::Bot,
                            TabKey::Settings => IconName::Settings,
                            TabKey::Changelog => IconName::BookOpen,
                        };
                        let connection_id = match tab {
                            TabKey::Collection(tab) => Some(tab.connection_id),
                            TabKey::Database(tab) => Some(tab.connection_id),
                            TabKey::Transfer(tab) => tab.connection_id,
                            TabKey::Forge(tab) => Some(tab.connection_id),
                            TabKey::AgentActivity | TabKey::Settings | TabKey::Changelog => None,
                        };
                        let connection_identity =
                            connection_id.and_then(|id| connection_identities.get(&id));
                        let connection_color =
                            connection_identity.and_then(|identity| identity.color);
                        let is_selected = selected_index == index;
                        let state = self.state.clone();
                        let tab_to_close = tab.clone();
                        let close_button = if islands_tab_variant {
                            div()
                                .id(("tab-close", index))
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(14.0))
                                .h(px(14.0))
                                .mr(px(6.0))
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .text_color(cx.theme().muted_foreground)
                                .hover(|s| {
                                    s.bg(cx.theme().secondary.opacity(0.45))
                                        .text_color(cx.theme().foreground)
                                })
                                .child(Icon::new(IconName::Close).xsmall())
                                .when(!is_selected, |s| {
                                    s.invisible().group_hover("tab-item", |s| s.visible())
                                })
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    cx.stop_propagation();
                                    request_close_tab(
                                        state.clone(),
                                        tab_to_close.clone(),
                                        window,
                                        cx,
                                    );
                                })
                        } else {
                            div()
                                .id(("tab-close", index))
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(16.0))
                                .h(px(16.0))
                                .mr(px(6.0))
                                .rounded(borders::radius_sm())
                                .cursor_pointer()
                                .hover(|s| s.bg(cx.theme().list_hover))
                                .child(
                                    Icon::new(IconName::Close)
                                        .xsmall()
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .when(!is_selected, |s| {
                                    s.invisible().group_hover("tab-item", |s| s.visible())
                                })
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    cx.stop_propagation();
                                    request_close_tab(
                                        state.clone(),
                                        tab_to_close.clone(),
                                        window,
                                        cx,
                                    );
                                })
                        };

                        let mut dirty_dot =
                            div().w(px(6.0)).h(px(6.0)).rounded_full().bg(cx.theme().primary);
                        if islands_tab_variant {
                            dirty_dot = dirty_dot.mr(px(2.0));
                        }

                        let icon_color = connection_color
                            .map(|color| colors::connection_accent(color, cx))
                            .unwrap_or_else(|| {
                                if is_selected {
                                    cx.theme().primary
                                } else {
                                    cx.theme().muted_foreground
                                }
                            });
                        let icon_el =
                            Icon::new(icon_name).with_size(px(14.0)).text_color(icon_color);
                        let prefix: AnyElement = if is_dirty {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .ml(px(6.0))
                                .child(dirty_dot)
                                .child(icon_el)
                                .into_any_element()
                        } else {
                            div()
                                .flex()
                                .items_center()
                                .ml(px(6.0))
                                .child(icon_el)
                                .into_any_element()
                        };

                        let drag_label: SharedString = label.clone().into();
                        let tab_view = Tab::new()
                            .max_w(px(OPEN_TAB_MAX_WIDTH))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .max_w(px(OPEN_TAB_LABEL_MAX_WIDTH))
                                    .child(div().min_w(px(0.0)).truncate().child(label))
                                    .when(
                                        matches!(tab, TabKey::AgentActivity)
                                            && pending_agent_actions > 0,
                                        |content| {
                                            content.child(
                                                div()
                                                    .min_w(px(14.0))
                                                    .h(px(14.0))
                                                    .px(px(3.0))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded_full()
                                                    .bg(cx.theme().danger)
                                                    .text_size(px(9.0))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(cx.theme().danger_foreground)
                                                    .child(if pending_agent_actions > 9 {
                                                        "9+".to_string()
                                                    } else {
                                                        pending_agent_actions.to_string()
                                                    }),
                                            )
                                        },
                                    )
                                    .when_some(connection_identity, |content, identity| {
                                        content.child(connection_identity_badge(identity, true, cx))
                                    }),
                            )
                            .prefix(prefix);

                        let drag_data = DraggedOpenTab { from_index: index, label: drag_label };
                        let drag_state = self.state.clone();

                        tab_view
                            .suffix(close_button)
                            .can_drop(move |value, _window, _cx| {
                                value
                                    .downcast_ref::<DraggedOpenTab>()
                                    .is_some_and(|drag| drag.from_index != index)
                            })
                            .drag_over::<DraggedOpenTab>({
                                let drag_state = drag_state.clone();
                                move |style, drag, _window, cx| {
                                    if drag.from_index == index {
                                        return style;
                                    }

                                    let insert_after = drag_state
                                        .read(cx)
                                        .tab_drag_over()
                                        .and_then(|(target, after)| {
                                            (target == index).then_some(after)
                                        })
                                        .unwrap_or(false);

                                    if insert_after {
                                        style
                                            .border_r_2()
                                            .border_l_0()
                                            .border_color(cx.theme().drag_border)
                                    } else {
                                        style
                                            .border_l_2()
                                            .border_r_0()
                                            .border_color(cx.theme().drag_border)
                                    }
                                }
                            })
                            .on_drag_move({
                                let drag_state = drag_state.clone();
                                move |event: &DragMoveEvent<DraggedOpenTab>, _window, cx| {
                                    let drag = event.drag(cx);
                                    if drag.from_index == index {
                                        return;
                                    }
                                    let insert_after =
                                        event.event.position.x > event.bounds.center().x;
                                    drag_state.update(cx, |state, cx| {
                                        let next = Some((index, insert_after));
                                        if state.tab_drag_over() != next {
                                            state.set_tab_drag_over(next);
                                            cx.notify();
                                        }
                                    });
                                }
                            })
                            .on_drop({
                                let drag_state = drag_state.clone();
                                move |drag: &DraggedOpenTab, _window, cx| {
                                    let to = drag_state
                                        .read(cx)
                                        .tab_drag_over()
                                        .and_then(|(target, after)| {
                                            (target == index).then_some(if after {
                                                index + 1
                                            } else {
                                                index
                                            })
                                        })
                                        .unwrap_or(index);

                                    drag_state.update(cx, |state, cx| {
                                        let from = drag.from_index;
                                        state.move_open_tab(from, to, cx);
                                        state.set_tab_drag_over(None);
                                        // Activate the dropped tab (Zed behavior)
                                        let final_index = if from == to || from + 1 == to {
                                            from
                                        } else if from < to {
                                            to - 1
                                        } else {
                                            to
                                        };
                                        state.select_tab(final_index, cx);
                                    });
                                }
                            })
                            .on_drag(drag_data, {
                                let drag_state = drag_state.clone();
                                move |drag, _position, _window, cx| {
                                    cx.stop_propagation();
                                    drag_state.update(cx, |state, cx| {
                                        if state.tab_drag_over().is_some() {
                                            state.set_tab_drag_over(None);
                                            cx.notify();
                                        }
                                    });
                                    cx.new(|_| drag.clone())
                                }
                            })
                    })
                    .chain(preview_tab.clone().map(|tab| {
                        let label = format!("{}/{}", tab.database, tab.collection);
                        let is_dirty = dirty_tabs.contains(&tab);
                        let is_preview_selected = matches!(active_tab, ActiveTab::Preview);
                        let state = self.state.clone();
                        let preview_to_close = tab.clone();
                        let close_button = if islands_tab_variant {
                            div()
                                .id("tab-close-preview")
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(14.0))
                                .h(px(14.0))
                                .mr(px(6.0))
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .text_color(cx.theme().muted_foreground)
                                .hover(|s| {
                                    s.bg(cx.theme().secondary.opacity(0.45))
                                        .text_color(cx.theme().foreground)
                                })
                                .child(Icon::new(IconName::Close).xsmall())
                                .when(!is_preview_selected, |s| {
                                    s.invisible().group_hover("tab-item", |s| s.visible())
                                })
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    cx.stop_propagation();
                                    request_close_preview(
                                        state.clone(),
                                        preview_to_close.clone(),
                                        window,
                                        cx,
                                    );
                                })
                        } else {
                            div()
                                .id("tab-close-preview")
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(16.0))
                                .h(px(16.0))
                                .mr(px(6.0))
                                .rounded(borders::radius_sm())
                                .cursor_pointer()
                                .hover(|s| s.bg(cx.theme().list_hover))
                                .child(
                                    Icon::new(IconName::Close)
                                        .xsmall()
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .when(!is_preview_selected, |s| {
                                    s.invisible().group_hover("tab-item", |s| s.visible())
                                })
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    cx.stop_propagation();
                                    request_close_preview(
                                        state.clone(),
                                        preview_to_close.clone(),
                                        window,
                                        cx,
                                    );
                                })
                        };

                        let mut dirty_dot =
                            div().w(px(6.0)).h(px(6.0)).rounded_full().bg(cx.theme().primary);
                        if islands_tab_variant {
                            dirty_dot = dirty_dot.mr(px(2.0));
                        }

                        let preview_identity = connection_identities.get(&tab.connection_id);
                        let icon_color = preview_identity
                            .and_then(|identity| identity.color)
                            .map(|color| colors::connection_accent(color, cx))
                            .unwrap_or_else(|| {
                                if is_preview_selected {
                                    cx.theme().primary
                                } else {
                                    cx.theme().muted_foreground
                                }
                            });
                        let icon_el =
                            Icon::new(IconName::Braces).with_size(px(14.0)).text_color(icon_color);
                        let prefix: AnyElement = if is_dirty {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .ml(px(6.0))
                                .child(dirty_dot)
                                .child(icon_el)
                                .into_any_element()
                        } else {
                            div()
                                .flex()
                                .items_center()
                                .ml(px(6.0))
                                .child(icon_el)
                                .into_any_element()
                        };

                        Tab::new()
                            .max_w(px(OPEN_TAB_MAX_WIDTH))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .max_w(px(OPEN_TAB_LABEL_MAX_WIDTH))
                                    .child(
                                        div()
                                            .min_w(px(0.0))
                                            .truncate()
                                            .italic()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(label),
                                    )
                                    .when_some(preview_identity, |content, identity| {
                                        content.child(connection_identity_badge(identity, true, cx))
                                    }),
                            )
                            .prefix(prefix)
                            .suffix(close_button)
                    })),
            );

        let strip_bg = islands::tool_bg(&appearance, cx).opacity(0.82);
        let no_line_mode = matches!(current_view, View::Documents | View::Forge);
        let strip_border = islands::panel_border(&appearance, cx);

        div()
            .id("collection-tabs-strip")
            .w_full()
            .h(tab_strip_height(&appearance))
            .min_w(px(0.0))
            .when(!no_line_mode, |s: Stateful<Div>| s.border_b_1().border_color(strip_border))
            .bg(strip_bg)
            .px(px(4.0))
            .py(px(4.0))
            .on_scroll_wheel({
                let scroll_handle = scroll_handle.clone();
                move |event, _window, _cx| {
                    let delta = event.delta.pixel_delta(px(1.0));
                    let axis = if delta.x.is_zero() { delta.y } else { delta.x };
                    if !axis.is_zero() {
                        scroll_tabs_by(&scroll_handle, axis);
                    }
                }
            })
            .child(div().min_w(px(0.0)).child(tab_bar))
    }
}

pub(crate) fn render_tabs_host(host: TabsHost<'_>, cx: &App) -> AnyElement {
    let appearance = host.state.read(cx).settings.appearance.clone();
    let tabs_bar = AnyView::from(host.tabs_bar.clone())
        .cached(StyleRefinement::default().w_full().h(tab_strip_height(&appearance)));

    let content = match host.current_view {
        View::Database => host
            .database_view
            .map(|view| view.clone().into_any_element())
            .unwrap_or_else(|| div().into_any_element()),
        View::Transfer => host
            .transfer_view
            .map(|view| view.clone().into_any_element())
            .unwrap_or_else(|| div().into_any_element()),
        View::Forge => host
            .forge_view
            .map(|view| view.clone().into_any_element())
            .unwrap_or_else(|| div().into_any_element()),
        View::AgentActivity => host
            .agent_activity_view
            .map(|view| view.clone().into_any_element())
            .unwrap_or_else(|| div().into_any_element()),
        View::Settings => host
            .settings_view
            .map(|view| view.clone().into_any_element())
            .unwrap_or_else(|| div().into_any_element()),
        View::Changelog => host
            .changelog_view
            .map(|view| view.clone().into_any_element())
            .unwrap_or_else(|| div().into_any_element()),
        _ => {
            if host.has_collection {
                host.collection_view
                    .map(|view| view.clone().into_any_element())
                    .unwrap_or_else(|| div().into_any_element())
            } else {
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Select a tab or open a collection")
                    .into_any_element()
            }
        }
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .h_full()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .child(tabs_bar)
        .child(div().flex_1().min_h(px(0.0)).min_w(px(0.0)).overflow_hidden().child(content))
        .into_any_element()
}
