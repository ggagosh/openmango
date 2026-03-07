use std::collections::HashSet;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::scroll::ScrollbarHandle as _;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};

use crate::state::{ActiveTab, AppState, IslandsTabStyle, SessionKey, TabKey, View};
use crate::theme::{borders, islands, spacing};
use crate::views::{
    ChangelogView, CollectionView, DatabaseView, ForgeView, SettingsView, TransferView,
};

const OPEN_TAB_MAX_WIDTH: f32 = 260.0;
const OPEN_TAB_LABEL_MAX_WIDTH: f32 = 210.0;

pub(crate) struct TabsHost<'a> {
    pub(crate) state: Entity<AppState>,
    pub(crate) tabs_scroll_handle: &'a ScrollHandle,
    pub(crate) scroll_to_end_once: bool,
    pub(crate) tabs: &'a [TabKey],
    pub(crate) active_tab: ActiveTab,
    pub(crate) preview_tab: Option<SessionKey>,
    pub(crate) dirty_tabs: &'a HashSet<SessionKey>,
    pub(crate) current_view: View,
    pub(crate) has_collection: bool,
    pub(crate) collection_view: Option<&'a Entity<CollectionView>>,
    pub(crate) database_view: Option<&'a Entity<DatabaseView>>,
    pub(crate) transfer_view: Option<&'a Entity<TransferView>>,
    pub(crate) forge_view: Option<&'a Entity<ForgeView>>,
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

pub(crate) fn render_tabs_host(host: TabsHost<'_>, cx: &App) -> AnyElement {
    let appearance = host.state.read(cx).settings.appearance.clone();
    let islands_tab_variant = appearance.islands.tab_style == IslandsTabStyle::Islands;
    let selected_index = match host.active_tab {
        ActiveTab::Preview => host.tabs.len(),
        ActiveTab::Index(index) => index.min(host.tabs.len().saturating_sub(1)),
        ActiveTab::None => 0,
    };

    let tabs = host.tabs;
    let dirty_tabs = host.dirty_tabs;
    let state = host.state.clone();
    let scroll_handle = host.tabs_scroll_handle.clone();
    if host.scroll_to_end_once {
        // This mirrors Zed-like behavior: reveal the selected tab minimally, not by forcing max-end.
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
                    let state = host.state.clone();
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
        .on_click(move |index, _window, cx| {
            let index = *index;
            state.update(cx, |state, cx| {
                if index < state.open_tabs().len() {
                    state.select_tab(index, cx);
                } else {
                    state.select_preview_tab(cx);
                }
            });
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
                            (host.state.read(cx).transfer_tab_label(tab.id), false)
                        }
                        TabKey::Forge(tab) => (host.state.read(cx).forge_tab_label(tab.id), false),
                        TabKey::Settings => ("Settings".to_string(), false),
                        TabKey::Changelog => ("What's New".to_string(), false),
                    };
                    let icon_name = match tab {
                        TabKey::Collection(_) => IconName::Braces,
                        TabKey::Database(_) => IconName::LayoutDashboard,
                        TabKey::Transfer(_) => IconName::Download,
                        TabKey::Forge(_) => IconName::SquareTerminal,
                        TabKey::Settings => IconName::Settings,
                        TabKey::Changelog => IconName::BookOpen,
                    };
                    let is_selected = selected_index == index;
                    let state = host.state.clone();
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
                            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                cx.stop_propagation();
                                state.update(cx, |state, cx| {
                                    state.close_tab(index, cx);
                                });
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
                            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                cx.stop_propagation();
                                state.update(cx, |state, cx| {
                                    state.close_tab(index, cx);
                                });
                            })
                    };

                    let mut dirty_dot =
                        div().w(px(6.0)).h(px(6.0)).rounded_full().bg(cx.theme().primary);
                    if islands_tab_variant {
                        dirty_dot = dirty_dot.mr(px(2.0));
                    }

                    let icon_color =
                        if is_selected { cx.theme().primary } else { cx.theme().muted_foreground };
                    let icon_el = Icon::new(icon_name).with_size(px(14.0)).text_color(icon_color);
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
                        div().flex().items_center().ml(px(6.0)).child(icon_el).into_any_element()
                    };

                    let drag_label: SharedString = label.clone().into();
                    let tab_view = Tab::new()
                        .max_w(px(OPEN_TAB_MAX_WIDTH))
                        .child(div().max_w(px(OPEN_TAB_LABEL_MAX_WIDTH)).truncate().child(label))
                        .prefix(prefix);

                    let drag_data = DraggedOpenTab { from_index: index, label: drag_label };
                    let drag_state = host.state.clone();

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
                                    .and_then(|(target, after)| (target == index).then_some(after))
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
                                let insert_after = event.event.position.x > event.bounds.center().x;
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
                .chain(host.preview_tab.clone().map(|tab| {
                    let label = format!("{}/{}", tab.database, tab.collection);
                    let is_dirty = dirty_tabs.contains(&tab);
                    let is_preview_selected = matches!(host.active_tab, ActiveTab::Preview);
                    let state = host.state.clone();
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
                            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                cx.stop_propagation();
                                state.update(cx, |state, cx| {
                                    state.close_preview_tab(cx);
                                });
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
                            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                cx.stop_propagation();
                                state.update(cx, |state, cx| {
                                    state.close_preview_tab(cx);
                                });
                            })
                    };

                    let mut dirty_dot =
                        div().w(px(6.0)).h(px(6.0)).rounded_full().bg(cx.theme().primary);
                    if islands_tab_variant {
                        dirty_dot = dirty_dot.mr(px(2.0));
                    }

                    let icon_color = if is_preview_selected {
                        cx.theme().primary
                    } else {
                        cx.theme().muted_foreground
                    };
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
                        div().flex().items_center().ml(px(6.0)).child(icon_el).into_any_element()
                    };

                    Tab::new()
                        .max_w(px(OPEN_TAB_MAX_WIDTH))
                        .child(
                            div()
                                .max_w(px(OPEN_TAB_LABEL_MAX_WIDTH))
                                .truncate()
                                .italic()
                                .text_color(cx.theme().muted_foreground)
                                .child(label),
                        )
                        .prefix(prefix)
                        .suffix(close_button)
                })),
        );

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

    let main_content: AnyElement = content;

    let strip_bg = islands::tool_bg(&appearance, cx).opacity(0.82);
    let no_line_mode = matches!(host.current_view, View::Documents | View::Forge);
    let strip_border = islands::panel_border(&appearance, cx);

    div()
        .flex()
        .flex_col()
        .flex_1()
        .h_full()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .child(
            div()
                .id("collection-tabs-strip")
                .w_full()
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
                .child(div().min_w(px(0.0)).child(tab_bar)),
        )
        .child(div().flex_1().min_h(px(0.0)).min_w(px(0.0)).overflow_hidden().child(main_content))
        .into_any_element()
}
