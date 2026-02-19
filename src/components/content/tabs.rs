use std::collections::HashSet;

use gpui::*;
use gpui_component::scroll::ScrollbarHandle as _;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};

use crate::state::{ActiveTab, AppState, SessionKey, TabKey, View};
use crate::theme::{borders, spacing};
use crate::views::{
    ChangelogView, CollectionView, DatabaseView, ForgeView, JsonEditorView, SettingsView,
    TransferView,
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
    pub(crate) json_editor_view: Option<&'a Entity<JsonEditorView>>,
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

    let tab_bar = TabBar::new("collection-tabs")
        .underline()
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
                        TabKey::JsonEditor(tab) => {
                            (host.state.read(cx).json_editor_tab_label(tab.id), false)
                        }
                        TabKey::Transfer(tab) => {
                            (host.state.read(cx).transfer_tab_label(tab.id), false)
                        }
                        TabKey::Forge(tab) => (host.state.read(cx).forge_tab_label(tab.id), false),
                        TabKey::Settings => ("Settings".to_string(), false),
                        TabKey::Changelog => ("What's New".to_string(), false),
                    };
                    let state = host.state.clone();
                    let close_button = div()
                        .id(("tab-close", index))
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(16.0))
                        .h(px(16.0))
                        .rounded(borders::radius_sm())
                        .cursor_pointer()
                        .hover(|s| s.bg(cx.theme().list_hover))
                        .child(
                            Icon::new(IconName::Close)
                                .xsmall()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                            cx.stop_propagation();
                            state.update(cx, |state, cx| {
                                state.close_tab(index, cx);
                            });
                        });

                    let dirty_dot =
                        div().w(px(6.0)).h(px(6.0)).rounded_full().bg(cx.theme().primary);

                    let drag_label: SharedString = label.clone().into();
                    let mut tab_view = Tab::new()
                        .max_w(px(OPEN_TAB_MAX_WIDTH))
                        .child(div().max_w(px(OPEN_TAB_LABEL_MAX_WIDTH)).truncate().child(label));
                    if is_dirty {
                        tab_view = tab_view.prefix(dirty_dot);
                    }

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
                    let state = host.state.clone();
                    let close_button = div()
                        .id("tab-close-preview")
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(16.0))
                        .h(px(16.0))
                        .rounded(borders::radius_sm())
                        .cursor_pointer()
                        .hover(|s| s.bg(cx.theme().list_hover))
                        .child(
                            Icon::new(IconName::Close)
                                .xsmall()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                            cx.stop_propagation();
                            state.update(cx, |state, cx| {
                                state.close_preview_tab(cx);
                            });
                        });

                    let dirty_dot =
                        div().w(px(6.0)).h(px(6.0)).rounded_full().bg(cx.theme().primary);

                    let mut tab_view = Tab::new().max_w(px(OPEN_TAB_MAX_WIDTH)).child(
                        div()
                            .max_w(px(OPEN_TAB_LABEL_MAX_WIDTH))
                            .truncate()
                            .italic()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    );
                    if is_dirty {
                        tab_view = tab_view.prefix(dirty_dot);
                    }

                    tab_view.suffix(close_button)
                })),
        );

    let content = match host.current_view {
        View::Database => host
            .database_view
            .map(|view| view.clone().into_any_element())
            .unwrap_or_else(|| div().into_any_element()),
        View::JsonEditor => host
            .json_editor_view
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
                .border_b_1()
                .border_color(cx.theme().border.opacity(0.65))
                .bg(cx.theme().tab_bar.opacity(0.4))
                .on_scroll_wheel({
                    let scroll_handle = scroll_handle.clone();
                    let state = host.state.clone();
                    move |event, _window, cx| {
                        let delta = event.delta.pixel_delta(px(1.0));
                        let axis = if delta.x.is_zero() { delta.y } else { delta.x };
                        if !axis.is_zero() {
                            scroll_tabs_by(&scroll_handle, axis);
                            state.update(cx, |_state, cx| cx.notify());
                        }
                    }
                })
                .child(div().min_w(px(0.0)).child(tab_bar)),
        )
        .child(content)
        .into_any_element()
}
