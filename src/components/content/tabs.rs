use std::collections::HashSet;

use gpui::*;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::state::{ActiveTab, AppState, SessionKey, TabKey, View};
use crate::theme::{borders, colors, spacing};
use crate::views::{CollectionView, DatabaseView, ForgeView, SettingsView, TransferView};

pub(crate) struct TabsHost<'a> {
    pub(crate) state: Entity<AppState>,
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
    let tab_bar = TabBar::new("collection-tabs")
        .underline()
        .small()
        .selected_index(selected_index)
        .menu(true)
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
                        .hover(|s| s.bg(colors::bg_hover()))
                        .child(Icon::new(IconName::Close).xsmall().text_color(colors::text_muted()))
                        .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                            cx.stop_propagation();
                            state.update(cx, |state, cx| {
                                state.close_tab(index, cx);
                            });
                        });

                    let dirty_dot = div().w(px(6.0)).h(px(6.0)).rounded_full().bg(colors::accent());

                    let mut tab_view = Tab::new().label(label);
                    if is_dirty {
                        tab_view = tab_view.prefix(dirty_dot);
                    }

                    tab_view.suffix(close_button)
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
                        .hover(|s| s.bg(colors::bg_hover()))
                        .child(Icon::new(IconName::Close).xsmall().text_color(colors::text_muted()))
                        .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                            cx.stop_propagation();
                            state.update(cx, |state, cx| {
                                state.close_preview_tab(cx);
                            });
                        });

                    let dirty_dot = div().w(px(6.0)).h(px(6.0)).rounded_full().bg(colors::accent());

                    let mut tab_view = Tab::new()
                        .child(div().italic().text_color(colors::text_muted()).child(label));
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
                    .text_color(colors::text_muted())
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
        .child(div().pl(spacing::sm()).child(tab_bar))
        .child(content)
        .into_any_element()
}
