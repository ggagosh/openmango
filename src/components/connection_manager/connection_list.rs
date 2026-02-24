//! Connection list panel rendering.
//!
//! Renders the left-side panel showing all saved connections.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Icon, IconName, Sizable as _};
use uuid::Uuid;

use crate::components::Button;
use crate::helpers::extract_host_from_uri;
use crate::models::SavedConnection;
use crate::theme::{borders, sizing, spacing};

use super::export_dialog::open_export_dialog;
use super::import::open_import_flow;
use super::{ConnectionManager, ManagerTab};

impl ConnectionManager {
    /// Renders the connection list panel (left side of the manager).
    pub(super) fn render_connection_list(
        &mut self,
        connections: Vec<SavedConnection>,
        selected_id: Option<Uuid>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let header = self.render_list_header(cx);

        div()
            .flex()
            .flex_col()
            .w(px(260.0))
            .min_w(px(260.0))
            .h_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(div().flex_shrink_0().child(header))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .child(Self::render_list_content(view, connections, selected_id, cx)),
            )
            .into_any_element()
    }

    /// Renders the header with title and action buttons.
    fn render_list_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let selected_id = self.selected_id;
        let state = self.state.clone();

        div()
            .flex()
            .items_center()
            .justify_between()
            .px(spacing::md())
            .h(sizing::header_height())
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().text_xs().text_color(cx.theme().secondary_foreground).child("Connections"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(
                        Button::new("import-connections")
                            .compact()
                            .tooltip("Import Connections")
                            .icon(Icon::new(IconName::Download).xsmall())
                            .on_click({
                                let state = state.clone();
                                move |_, window, cx| {
                                    open_import_flow(state.clone(), window, cx);
                                }
                            }),
                    )
                    .child(
                        Button::new("export-connections")
                            .compact()
                            .tooltip("Export Connections")
                            .icon(Icon::new(IconName::Upload).xsmall())
                            .on_click({
                                let state = state.clone();
                                move |_, window, cx| {
                                    open_export_dialog(state.clone(), window, cx);
                                }
                            }),
                    )
                    .child(
                        Button::new("new-connection")
                            .compact()
                            .icon(Icon::new(IconName::Plus).xsmall())
                            .on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.load_connection(None, window, cx);
                                        this.active_tab = ManagerTab::General;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("remove-connection")
                            .compact()
                            .icon(Icon::new(IconName::Delete).xsmall())
                            .disabled(selected_id.is_none())
                            .on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.remove_connection(window, cx);
                                    });
                                }
                            }),
                    ),
            )
            .into_any_element()
    }

    /// Renders the list of connections or an empty state.
    fn render_list_content(
        view: Entity<Self>,
        connections: Vec<SavedConnection>,
        selected_id: Option<Uuid>,
        cx: &App,
    ) -> AnyElement {
        if connections.is_empty() {
            div()
                .p(spacing::md())
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No connections")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .p(spacing::xs())
                .child(div().flex().flex_col().gap(px(2.0)).children(connections.into_iter().map(
                    move |conn| Self::render_connection_item(view.clone(), conn, selected_id, cx),
                )))
                .into_any_element()
        }
    }

    /// Renders a single connection item in the list.
    fn render_connection_item(
        view: Entity<Self>,
        conn: SavedConnection,
        selected_id: Option<Uuid>,
        cx: &App,
    ) -> AnyElement {
        let is_selected = Some(conn.id) == selected_id;
        let host = extract_host_from_uri(&conn.uri).unwrap_or_else(|| "Unknown host".to_string());
        let last_connected = conn
            .last_connected
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "Never".to_string());
        let read_only = conn.read_only;

        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(spacing::sm())
            .py(spacing::xs())
            .cursor_pointer()
            .rounded(borders::radius_sm())
            .border_1()
            .when(is_selected, |s| s.bg(cx.theme().list_hover).border_color(cx.theme().border))
            .when(!is_selected, |s| s.border_color(gpui::transparent_black()))
            .hover(|s| s.bg(cx.theme().list_hover))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div().text_sm().text_color(cx.theme().foreground).child(conn.name.clone()),
                    )
                    .when(read_only, |s| {
                        s.child(
                            div()
                                .px(spacing::xs())
                                .py(px(1.0))
                                .rounded(borders::radius_sm())
                                .bg(cx.theme().warning)
                                .text_xs()
                                .text_color(cx.theme().tab_bar)
                                .child("RO"),
                        )
                    }),
            )
            .child(div().text_xs().text_color(cx.theme().secondary_foreground).child(host))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Last: {last_connected}")),
            )
            .on_mouse_down(MouseButton::Left, {
                move |_, window, cx| {
                    view.update(cx, |this, cx| {
                        this.load_connection(Some(conn.clone()), window, cx);
                        cx.notify();
                    });
                }
            })
            .into_any_element()
    }
}
