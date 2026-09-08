use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::base::{ElementExt as _, Tab, Tabs};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::actions::model::ActionStatus;
use crate::components::{
    Button, ConnectionIdentity, ConnectionManager as ConnectionManagerView,
    connection_identity_badge, request_unsaved_action,
};
use crate::keyboard::{self, FocusContent};
use crate::state::{ActiveTab, AppState, SessionKey, TabKey, UnsavedScope, View};
use crate::theme::{borders, colors, fonts, spacing};
use crate::views::{
    AgentActivityView, ChangelogView, CollectionView, DatabaseView, ForgeView, SettingsView,
    TransferView,
};

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

pub(crate) struct OpenTabsBar {
    state: Entity<AppState>,
    tabs_scroll_handle: ScrollHandle,
    last_selection: Option<(usize, TabKey, bool)>,
    last_tab_count: usize,
    reveal_pending: Rc<Cell<bool>>,
    last_viewport_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    _subscriptions: Vec<Subscription>,
}

pub(crate) struct TabsHost<'a> {
    pub(crate) current_view: View,
    pub(crate) has_collection: bool,
    pub(crate) collection_view: Option<&'a Entity<CollectionView>>,
    pub(crate) database_view: Option<&'a Entity<DatabaseView>>,
    pub(crate) transfer_view: Option<&'a Entity<TransferView>>,
    pub(crate) forge_view: Option<&'a Entity<ForgeView>>,
    pub(crate) agent_activity_view: Option<&'a Entity<AgentActivityView>>,
    pub(crate) connection_manager_view: Option<&'a Entity<ConnectionManagerView>>,
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
            .font_family(fonts::tabs())
            .text_sm()
            .text_color(cx.theme().tab_active_foreground)
            .child(self.label.clone())
    }
}

impl OpenTabsBar {
    pub(crate) fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        Self {
            state: state.clone(),
            tabs_scroll_handle: ScrollHandle::new(),
            last_selection: None,
            last_tab_count: 0,
            reveal_pending: Rc::new(Cell::new(false)),
            last_viewport_bounds: Rc::new(Cell::new(None)),
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
        }
    }
}

fn tab_connection_id(tab: &TabKey) -> Option<uuid::Uuid> {
    match tab {
        TabKey::Collection(key) => Some(key.connection_id),
        TabKey::Database(key) => Some(key.connection_id),
        TabKey::Transfer(key) => key.connection_id,
        TabKey::Forge(key) => Some(key.connection_id),
        _ => None,
    }
}

fn tab_shortcut(index: usize, window: &Window) -> Option<Kbd> {
    let actions: [&dyn Action; 9] = [
        &keyboard::SelectTab1,
        &keyboard::SelectTab2,
        &keyboard::SelectTab3,
        &keyboard::SelectTab4,
        &keyboard::SelectTab5,
        &keyboard::SelectTab6,
        &keyboard::SelectTab7,
        &keyboard::SelectTab8,
        &keyboard::SelectTab9,
    ];
    let bindings = window
        .bindings_for_action_in_context(*actions.get(index)?, KeyContext::parse("Workspace").ok()?);
    let binding = bindings
        .iter()
        .rev()
        .find(|binding| {
            binding.keystrokes().first().is_some_and(|key| {
                let modifiers = key.as_keystroke().modifiers;
                if cfg!(target_os = "macos") { modifiers.platform } else { modifiers.control }
            })
        })
        .or_else(|| bindings.last())?;
    Some(Kbd::new(binding.keystrokes().first()?.as_keystroke().clone()))
}

impl Render for OpenTabsBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (tabs, preview_tab, active_tab, dirty_tabs, connections, pending_actions) = {
            let state = self.state.read(cx);
            (
                state.open_tabs().to_vec(),
                state.preview_tab().cloned(),
                state.active_tab(),
                state.dirty_tabs().clone(),
                state
                    .connections_snapshot()
                    .into_iter()
                    .map(|connection| (connection.id, ConnectionIdentity::from(&connection)))
                    .collect::<HashMap<_, _>>(),
                state
                    .action_broker()
                    .list_all()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|action| action.status == ActionStatus::PendingApproval)
                    .count(),
            )
        };
        let selected_index = match active_tab {
            ActiveTab::Index(index) if index < tabs.len() => Some(index),
            ActiveTab::Preview if preview_tab.is_some() => Some(tabs.len()),
            _ => None,
        };
        let entries: Vec<_> = tabs
            .into_iter()
            .map(|tab| (tab, false))
            .chain(preview_tab.map(|key| (TabKey::Collection(key), true)))
            .collect();
        let selection = selected_index.and_then(|index| {
            entries.get(index).map(|(tab, preview)| (index, tab.clone(), *preview))
        });
        if self.last_selection != selection || self.last_tab_count != entries.len() {
            // A preview can replace another tab at the same index. Keep the
            // request pending across render passes until layout consumes it.
            self.reveal_pending.set(true);
        }
        self.last_selection = selection;
        self.last_tab_count = entries.len();

        let show_connection_names = entries
            .iter()
            .filter_map(|(tab, _)| tab_connection_id(tab))
            .collect::<HashSet<_>>()
            .len()
            > 1;
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let track = foreground.opacity(0.07);
        let selected_bg =
            if cx.theme().is_dark() { foreground.opacity(0.17) } else { cx.theme().background };
        let selected_border = foreground.opacity(0.18);
        let hover_bg = foreground.opacity(0.06);
        let tab_count = entries.len();
        let scroll_handle = self.tabs_scroll_handle.clone();
        let last_viewport_bounds = self.last_viewport_bounds.clone();
        let reveal_pending = self.reveal_pending.clone();
        let reveal_state = self.state.clone();

        let tab_items = entries
            .into_iter()
            .enumerate()
            .map(|(index, (tab, is_preview))| {
                let (label, icon, is_dirty): (String, Icon, bool) = match &tab {
                    TabKey::Collection(key) => (
                        format!("{}/{}", key.database, key.collection),
                        crate::assets::AppIcon::Braces.into(),
                        dirty_tabs.contains(key),
                    ),
                    TabKey::Database(key) => {
                        (key.database.clone(), IconName::LayoutDashboard.into(), false)
                    }
                    TabKey::Transfer(key) => (
                        self.state.read(cx).transfer_tab_label(key.id),
                        crate::assets::AppIcon::Download.into(),
                        false,
                    ),
                    TabKey::Forge(key) => {
                        (key.database.clone(), IconName::SquareTerminal.into(), false)
                    }
                    TabKey::AgentActivity => ("Agent Activity".into(), IconName::Bot.into(), false),
                    TabKey::Connections => {
                        ("Connections".into(), IconName::Settings2.into(), false)
                    }
                    TabKey::Settings => ("Settings".into(), IconName::Settings.into(), false),
                    TabKey::Changelog => ("What's New".into(), IconName::BookOpen.into(), false),
                };
                let identity = tab_connection_id(&tab).and_then(|id| connections.get(&id));
                let is_selected = selected_index == Some(index);
                let title = if matches!(tab, TabKey::Forge(_)) {
                    format!("Forge: {label}")
                } else {
                    label.clone()
                };
                let tooltip = identity
                    .map(|identity| format!("{title} · {}", identity.display_name()))
                    .unwrap_or(title);
                let icon_color = identity
                    .and_then(|identity| identity.color)
                    .map(|color| colors::connection_accent(color, cx))
                    .unwrap_or(muted);
                let close_state = self.state.clone();
                let close_tab = tab.clone();
                let close_button = Button::new(("workspace-tab-close", index))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip("Close tab")
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        if is_preview {
                            if let TabKey::Collection(key) = &close_tab {
                                request_close_preview(close_state.clone(), key.clone(), window, cx);
                            }
                        } else {
                            request_close_tab(close_state.clone(), close_tab.clone(), window, cx);
                        }
                    });
                let shortcut = (!is_preview).then(|| tab_shortcut(index, window)).flatten();
                let state = self.state.clone();
                let drag_label: SharedString = label.clone().into();
                let tab_view = Tab::new(("workspace-tab", index))
                    .group("workspace-tab")
                    .accessibility_label(tooltip.clone())
                    .set_position(index + 1, tab_count)
                    .selected(is_selected)
                    .flex_1()
                    .min_w(px(180.0))
                    .h_full()
                    .gap(spacing::sm())
                    .px(spacing::sm())
                    .rounded_full()
                    .border_1()
                    .border_color(colors::transparent())
                    .bg(colors::transparent())
                    .text_color(muted)
                    .styles(|styles| {
                        styles.selected(|style| {
                            style
                                .bg(selected_bg)
                                .border_color(selected_border)
                                .text_color(foreground)
                                .font_weight(FontWeight::MEDIUM)
                        })
                    })
                    .hover(move |style| {
                        if is_selected { style } else { style.bg(hover_bg).text_color(foreground) }
                    })
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        state.update(cx, |state, cx| {
                            if is_preview {
                                state.select_preview_tab(cx);
                            } else {
                                state.select_tab(index, cx);
                            }
                        });
                        window.dispatch_action(Box::new(FocusContent), cx);
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .flex_shrink_0()
                            .child(icon.with_size(px(13.0)).text_color(icon_color))
                            .when(is_dirty, |row| {
                                row.child(div().size(px(5.0)).rounded_full().bg(cx.theme().primary))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(spacing::sm())
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .min_w(px(0.0))
                                    .truncate()
                                    .when(is_preview, |label| label.italic())
                                    .child(label),
                            )
                            .when_some(
                                identity.filter(|identity| {
                                    show_connection_names
                                        || identity.environment.is_some()
                                        || identity.read_only
                                }),
                                |row, identity| {
                                    row.child(connection_identity_badge(
                                        identity,
                                        show_connection_names,
                                        cx,
                                    ))
                                },
                            )
                            .when(
                                matches!(tab, TabKey::AgentActivity) && pending_actions > 0,
                                |row| {
                                    row.child(
                                        div()
                                            .flex_shrink_0()
                                            .px(spacing::xs())
                                            .rounded_full()
                                            .bg(cx.theme().danger)
                                            .text_color(cx.theme().danger_foreground)
                                            .text_xs()
                                            .child(if pending_actions > 9 {
                                                "9+".into()
                                            } else {
                                                pending_actions.to_string()
                                            }),
                                    )
                                },
                            ),
                    )
                    .when_some(shortcut, |tab, shortcut| {
                        tab.child(
                            div()
                                .flex_shrink_0()
                                .child(shortcut.appearance(false).text_xs().text_color(muted)),
                        )
                    })
                    .child(
                        div()
                            .size(px(20.0))
                            .flex_shrink_0()
                            .when(!is_selected, |close| {
                                close
                                    .invisible()
                                    .group_hover("workspace-tab", |close| close.visible())
                            })
                            .child(close_button),
                    );

                // Preview tabs remain transient; only pinned workspace tabs can be reordered.
                let drag_state = self.state.clone();
                tab_view.when(!is_preview, |tab_view| {
                    tab_view
                        .can_drop(move |value, _, _| {
                            value
                                .downcast_ref::<DraggedOpenTab>()
                                .is_some_and(|drag| drag.from_index != index)
                        })
                        .drag_over::<DraggedOpenTab>({
                            let state = drag_state.clone();
                            move |style, drag, _, cx| {
                                if drag.from_index == index {
                                    return style;
                                }
                                let after = state
                                    .read(cx)
                                    .tab_drag_over()
                                    .is_some_and(|(target, after)| target == index && after);
                                if after {
                                    style.border_r_2().border_color(cx.theme().drag_border)
                                } else {
                                    style.border_l_2().border_color(cx.theme().drag_border)
                                }
                            }
                        })
                        .on_drag_move({
                            let state = drag_state.clone();
                            move |event: &DragMoveEvent<DraggedOpenTab>, _, cx| {
                                if event.drag(cx).from_index == index {
                                    return;
                                }
                                let after = event.event.position.x > event.bounds.center().x;
                                state.update(cx, |state, cx| {
                                    let next = Some((index, after));
                                    if state.tab_drag_over() != next {
                                        state.set_tab_drag_over(next);
                                        cx.notify();
                                    }
                                });
                            }
                        })
                        .on_drop({
                            let state = drag_state.clone();
                            move |drag: &DraggedOpenTab, _, cx| {
                                cx.stop_propagation();
                                state.update(cx, |state, cx| {
                                    let to = state
                                        .tab_drag_over()
                                        .and_then(|(target, after)| {
                                            (target == index).then_some(index + usize::from(after))
                                        })
                                        .unwrap_or(index);
                                    let from = drag.from_index;
                                    state.move_open_tab(from, to, cx);
                                    state.set_tab_drag_over(None);
                                    let selected = if from == to || from + 1 == to {
                                        from
                                    } else if from < to {
                                        to - 1
                                    } else {
                                        to
                                    };
                                    state.select_tab(selected, cx);
                                });
                            }
                        })
                        .on_drag(DraggedOpenTab { from_index: index, label: drag_label }, {
                            let state = drag_state.clone();
                            move |drag, _, _, cx| {
                                cx.stop_propagation();
                                state.update(cx, |state, cx| {
                                    if state.tab_drag_over().is_some() {
                                        state.set_tab_drag_over(None);
                                        cx.notify();
                                    }
                                });
                                cx.new(|_| drag.clone())
                            }
                        })
                })
            })
            .collect::<Vec<_>>();

        let state = self.state.clone();
        div()
            .id("workspace-title-tabs")
            .flex()
            .items_center()
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .font_family(fonts::tabs())
            .text_size(px(13.0))
            .font_weight(FontWeight::NORMAL)
            .when(tab_count == 0, |bar| {
                bar.child(div().flex_1().text_center().text_color(muted).child("OpenMango"))
            })
            .when(tab_count > 0, |bar| {
                bar.child(
                    Tabs::new("workspace-tabs")
                        // Tabs are client controls; only the surrounding gutters
                        // should participate in the native titlebar hit test.
                        .occlude()
                        .flex()
                        .items_center()
                        .flex_1()
                        .min_w(px(0.0))
                        .h(px(28.0))
                        .rounded_full()
                        .bg(track)
                        .gap(px(2.0))
                        .overflow_x_scroll()
                        .overflow_y_hidden()
                        .track_scroll(&scroll_handle)
                        .children(tab_items),
                )
            })
            .child(
                div()
                    .id("workspace-tabs-end-drop")
                    .w(px(12.0))
                    .h_full()
                    .flex_shrink_0()
                    .can_drop(|value, _, _| value.downcast_ref::<DraggedOpenTab>().is_some())
                    .drag_over::<DraggedOpenTab>(|style, _, _, cx| style.bg(cx.theme().drop_target))
                    .on_drop(move |drag: &DraggedOpenTab, _, cx| {
                        cx.stop_propagation();
                        state.update(cx, |state, cx| {
                            let to = state.open_tabs().len();
                            state.move_open_tab(drag.from_index, to, cx);
                            state.set_tab_drag_over(None);
                            state.select_tab(to.saturating_sub(1), cx);
                        });
                    }),
            )
            .when(tab_count > 0, |bar| {
                // Observe after the tab list: its native scroll handle now has
                // current bounds. Keeping this outside Tabs preserves child indices.
                bar.on_prepaint(move |_, window, _| {
                    let bounds = scroll_handle.bounds();
                    let resized = last_viewport_bounds.replace(Some(bounds)) != Some(bounds);
                    if resized {
                        reveal_pending.set(true);
                    }
                    if reveal_pending.replace(false) {
                        window.on_next_frame(move |window, cx| {
                            let state = reveal_state.read(cx);
                            let index = match state.active_tab() {
                                ActiveTab::Index(index) if index < state.open_tabs().len() => {
                                    Some(index)
                                }
                                ActiveTab::Preview if state.preview_tab().is_some() => {
                                    Some(state.open_tabs().len())
                                }
                                _ => None,
                            };
                            if let Some(index) = index {
                                scroll_handle.scroll_to_item(index);
                                window.refresh();
                            }
                        });
                    }
                })
            })
    }
}

pub(crate) fn render_tabs_host(host: TabsHost<'_>, cx: &App) -> AnyElement {
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
        View::Connections => host
            .connection_manager_view
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
        .child(div().flex_1().min_h(px(0.0)).min_w(px(0.0)).overflow_hidden().child(content))
        .into_any_element()
}
