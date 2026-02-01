//! Connection list panel rendering.
//!
//! Renders the left-side panel showing all saved connections.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Icon, IconName, Sizable as _};
use uuid::Uuid;

use crate::components::Button;
use crate::helpers::extract_host_from_uri;
use crate::models::SavedConnection;
use crate::theme::{borders, colors, sizing, spacing};

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
            .w(px(320.0))
            .min_w(px(320.0))
            .h_full()
            .border_r_1()
            .border_color(colors::border())
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .child(Self::render_list_content(view, connections, selected_id)),
            )
            .into_any_element()
    }

    /// Renders the header with title and action buttons.
    fn render_list_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let selected_id = self.selected_id;

        div()
            .flex()
            .items_center()
            .justify_between()
            .px(spacing::md())
            .h(sizing::header_height())
            .border_b_1()
            .border_color(colors::border())
            .child(div().text_xs().text_color(colors::text_secondary()).child("Connections"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
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
    ) -> AnyElement {
        if connections.is_empty() {
            div()
                .p(spacing::md())
                .text_sm()
                .text_color(colors::text_muted())
                .child("No connections")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .p(spacing::xs())
                .child(div().flex().flex_col().children(connections.into_iter().map(move |conn| {
                    Self::render_connection_item(view.clone(), conn, selected_id)
                })))
                .into_any_element()
        }
    }

    /// Renders a single connection item in the list.
    fn render_connection_item(
        view: Entity<Self>,
        conn: SavedConnection,
        selected_id: Option<Uuid>,
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
            .gap(px(4.0))
            .px(spacing::md())
            .py(spacing::sm())
            .cursor_pointer()
            .rounded(borders::radius_sm())
            .when(is_selected, |s| {
                s.bg(colors::bg_hover()).border_1().border_color(colors::border())
            })
            .hover(|s| s.bg(colors::bg_hover()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div().text_sm().text_color(colors::text_primary()).child(conn.name.clone()),
                    )
                    .when(read_only, |s| {
                        s.child(
                            div()
                                .px(spacing::xs())
                                .py(px(1.0))
                                .rounded(borders::radius_sm())
                                .bg(colors::status_warning())
                                .text_xs()
                                .text_color(colors::bg_header())
                                .child("RO"),
                        )
                    }),
            )
            .child(div().text_xs().text_color(colors::text_secondary()).child(host))
            .child(
                div()
                    .text_xs()
                    .text_color(colors::text_muted())
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
