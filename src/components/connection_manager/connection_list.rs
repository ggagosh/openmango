use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IndexPath, Sizable as _};
use gpui_kit::*;

use crate::components::{ConnectionIdentity, connection_identity_badge};
use crate::helpers::extract_host_from_uri;
use crate::models::SavedConnection;
use crate::state::AppState;
use crate::theme::{sizing, spacing};

use super::ConnectionManager;
use super::export_dialog::open_export_dialog;
use super::import::open_import_flow;

pub(super) struct ConnectionList {
    pub(super) manager: WeakEntity<ConnectionManager>,
    pub(super) state: Entity<AppState>,
    pub(super) query: String,
    pub(super) connections: Vec<SavedConnection>,
    pub(super) selected: Option<IndexPath>,
}

impl ConnectionList {
    fn refresh(&mut self, cx: &App) {
        self.connections = self
            .state
            .read(cx)
            .connections
            .iter()
            .filter(|connection| {
                connection.name.to_lowercase().contains(&self.query)
                    || extract_host_from_uri(&connection.uri)
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(&self.query)
            })
            .cloned()
            .collect();
    }
}

impl ListDelegate for ConnectionList {
    type Item = ListItem;
    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.connections.len()
    }
    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let connection = self.connections.get(ix.row)?;
        let identity = ConnectionIdentity::from(connection);
        let host = extract_host_from_uri(&connection.uri).unwrap_or_default();
        let connected = self.state.read(cx).is_connected(connection.id);
        Some(
            ListItem::new(connection.id.to_string()).child(
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(0.))
                    .gap(px(2.))
                    .child(connection_identity_badge(&identity, true, cx))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(host),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if connected { "Connected" } else { "Disconnected" }),
                    ),
            ),
        )
    }
    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.selected = ix;
    }
    fn confirm(
        &mut self,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        let Some(connection) = self.selected.and_then(|ix| self.connections.get(ix.row)).cloned()
        else {
            return;
        };
        let manager = self.manager.clone();
        let handle = window.window_handle();
        // End the list lease before the manager updates the confirmed selection.
        cx.defer(move |cx| {
            let Some(manager) = manager.upgrade() else {
                return;
            };
            let _ = handle.update(cx, |_, window, cx| {
                ConnectionManager::request_load_connection(
                    manager.clone(),
                    Some(connection),
                    window,
                    cx,
                );
                manager.update(cx, |manager, cx| manager.sync_connection_list(window, cx));
            });
        });
    }
    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.query = query.to_lowercase();
        self.refresh(cx);
        Task::ready(())
    }
    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div().p(spacing::md()).text_sm().text_color(cx.theme().muted_foreground).child(
            if self.query.is_empty() {
                "No saved connections. Choose New to add one."
            } else {
                "No matching connections."
            },
        )
    }
}

impl ConnectionManager {
    pub(super) fn sync_connection_list(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.connection_list.update(cx, |list, cx| {
            list.delegate_mut().refresh(cx);
            let selected = list
                .delegate()
                .connections
                .iter()
                .position(|connection| Some(connection.id) == self.selected_id)
                .map(IndexPath::new);
            list.set_selected_index(selected, window, cx);
            cx.notify();
        });
    }
    pub(super) fn render_connection_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.clone();
        let view = cx.entity();
        div()
            .flex()
            .flex_col()
            .w(px(224.))
            .min_w(px(180.))
            .max_w(px(224.))
            .h_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(spacing::sm())
                    .h(sizing::header_height())
                    .child(div().text_sm().child("Saved connections"))
                    .child(
                        Button::new("new-connection")
                            .small()
                            .ghost()
                            .label("New")
                            .disabled(self.pending_save.is_some())
                            .on_click(move |_, window, cx| {
                                Self::request_load_connection(view.clone(), None, window, cx)
                            }),
                    ),
            )
            .child(List::new(&self.connection_list).small().flex_1().min_h(px(0.)))
            .child(
                div().p(spacing::sm()).child(
                    Button::new("connection-list-more")
                        .small()
                        .ghost()
                        .label("Import / Export")
                        .dropdown_menu(move |menu, _, _| {
                            menu.item(PopupMenuItem::new("Import connections…").on_click({
                                let state = state.clone();
                                move |_, window, cx| open_import_flow(state.clone(), window, cx)
                            }))
                            .item(
                                PopupMenuItem::new("Export connections…").on_click({
                                    let state = state.clone();
                                    move |_, window, cx| {
                                        open_export_dialog(state.clone(), window, cx)
                                    }
                                }),
                            )
                        }),
                ),
            )
            .into_any_element()
    }
}
