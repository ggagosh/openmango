use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::Input;
use gpui_component::scroll::ScrollableElement;
use gpui_component::switch::Switch;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{Icon, IconName, Sizable as _, WindowExt as _};

use crate::components::Button;
use crate::helpers::extract_host_from_uri;
use crate::state::AppCommands;
use crate::theme::{borders, colors, sizing, spacing};

use super::{ConnectionManager, ManagerTab, TestStatus};
impl Render for ConnectionManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connections = self.state.read(cx).connections_snapshot();
        let selected_exists =
            self.selected_id.is_some_and(|id| connections.iter().any(|conn| conn.id == id));
        if !selected_exists && !connections.is_empty() && !self.creating_new {
            let next = connections.first().cloned();
            self.load_connection(next, window, cx);
        }

        let active_tab = self.active_tab;
        let status = self.status.clone();
        let is_testing = matches!(status, TestStatus::Testing);
        let status_text = match &status {
            TestStatus::Idle => "Test connection to verify settings".to_string(),
            TestStatus::Testing => "Testing connection...".to_string(),
            TestStatus::Success => "Connection OK".to_string(),
            TestStatus::Error(err) => format!("Connection failed: {err}"),
        };
        let status_color = match status {
            TestStatus::Success => colors::accent(),
            TestStatus::Error(_) => colors::status_error(),
            _ => colors::text_muted(),
        };

        let selected_id = self.selected_id;
        let is_active_selection =
            selected_id.is_some_and(|id| self.state.read(cx).is_connected(id));

        let list = div()
            .flex()
            .flex_col()
            .w(px(320.0))
            .min_w(px(320.0))
            .h_full()
            .border_r_1()
            .border_color(colors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(spacing::md())
                    .h(sizing::header_height())
                    .border_b_1()
                    .border_color(colors::border())
                    .child(
                        div().text_xs().text_color(colors::text_secondary()).child("Connections"),
                    )
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
                                        let view = cx.entity();
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
                                        let view = cx.entity();
                                        move |_, window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.remove_connection(window, cx);
                                            });
                                        }
                                    }),
                            ),
                    ),
            )
            .child(div().flex().flex_col().flex_1().overflow_y_scrollbar().child(
                if connections.is_empty() {
                    div()
                        .p(spacing::md())
                        .text_sm()
                        .text_color(colors::text_muted())
                        .child("No connections")
                        .into_any_element()
                } else {
                    let view = cx.entity();
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .p(spacing::xs())
                        .child(div().flex().flex_col().children(connections.into_iter().map(
                            move |conn| {
                                let is_selected = Some(conn.id) == selected_id;
                                let host = extract_host_from_uri(&conn.uri)
                                    .unwrap_or_else(|| "Unknown host".to_string());
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
                                        s.bg(colors::bg_hover())
                                            .border_1()
                                            .border_color(colors::border())
                                    })
                                    .hover(|s| s.bg(colors::bg_hover()))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(colors::text_primary())
                                                    .child(conn.name.clone()),
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
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(colors::text_secondary())
                                            .child(host),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(colors::text_muted())
                                            .child(format!("Last: {last_connected}")),
                                    )
                                    .on_mouse_down(MouseButton::Left, {
                                        let view = view.clone();
                                        let conn = conn.clone();
                                        move |_, window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.load_connection(
                                                    Some(conn.clone()),
                                                    window,
                                                    cx,
                                                );
                                                cx.notify();
                                            });
                                        }
                                    })
                            },
                        )))
                        .into_any_element()
                },
            ));

        let tab_bar = TabBar::new("connection-manager-tabs")
            .selected_index(active_tab.index())
            .on_click({
                let view = cx.entity();
                move |index, _window, cx| {
                    let index = *index;
                    view.update(cx, |this, cx| {
                        this.active_tab = ManagerTab::from_index(index);
                        cx.notify();
                    });
                }
            })
            .children(ManagerTab::all().into_iter().map(|tab| Tab::new().label(tab.label())));

        let parse_error = self.parse_error.clone();
        let actions = div()
            .flex()
            .items_center()
            .gap(spacing::sm())
            .flex_shrink_0()
            .child(
                Button::new("test-connection")
                    .compact()
                    .label(if is_testing { "Testing..." } else { "Test" })
                    .disabled(is_testing)
                    .on_click({
                        let view = cx.entity();
                        move |_, _window, cx| {
                            ConnectionManager::start_test(view.clone(), cx);
                        }
                    }),
            )
            .child(
                Button::new("save-connection")
                    .compact()
                    .primary()
                    .label(if is_active_selection { "Save & Reconnect" } else { "Save" })
                    .on_click({
                        let view = cx.entity();
                        let state = self.state.clone();
                        move |_, window, cx| {
                            let mut reconnect_id = None;
                            view.update(cx, |this, cx| {
                                let saved_id = this.save_connection(window, cx);
                                if let Some(saved_id) = saved_id
                                    && this.state.read(cx).is_connected(saved_id)
                                {
                                    reconnect_id = Some(saved_id);
                                }
                            });
                            if let Some(connection_id) = reconnect_id {
                                AppCommands::connect(state.clone(), connection_id, cx);
                            }
                        }
                    }),
            )
            .child(Button::new("close-manager").compact().label("Close").on_click(
                |_, window, cx| {
                    window.close_dialog(cx);
                },
            ));
        let editor = div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w(px(0.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(spacing::md())
                    .h(sizing::header_height())
                    .border_b_1()
                    .border_color(colors::border())
                    .child(tab_bar),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .overflow_y_scrollbar()
                            .p(spacing::md())
                            .child(match active_tab {
                                ManagerTab::General => {
                                    self.render_general_tab(parse_error, window, cx)
                                }
                                ManagerTab::Auth => self.render_auth_tab(window, cx),
                                ManagerTab::Options => self.render_options_tab(window, cx),
                                ManagerTab::Tls => self.render_tls_tab(window, cx),
                                ManagerTab::Pool => self.render_pool_tab(window, cx),
                                ManagerTab::Advanced => self.render_advanced_tab(window, cx),
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(spacing::sm())
                            .px(spacing::md())
                            .h(sizing::header_height())
                            .border_t_1()
                            .border_color(colors::border())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .text_color(status_color)
                                    .truncate()
                                    .child(status_text),
                            )
                            .child(actions),
                    ),
            );

        div().flex().flex_row().flex_1().h_full().w_full().child(list).child(editor)
    }
}

impl ConnectionManager {
    fn render_general_tab(
        &mut self,
        parse_error: Option<String>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("Name"))
                    .child(Input::new(&self.draft.name_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("URI"))
                    .child(
                        Input::new(&self.draft.uri_state)
                            .font_family(crate::theme::fonts::mono())
                            .w_full(),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(
                                Button::new("import-uri")
                                    .compact()
                                    .label("Import from URI")
                                    .on_click({
                                        let view = view.clone();
                                        move |_, window, cx| {
                                            ConnectionManager::import_uri_from_clipboard_or_dialog(
                                                view.clone(),
                                                window,
                                                cx,
                                            );
                                        }
                                    }),
                            )
                            .child(
                                Button::new("apply-uri").compact().label("Update URI").on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        view.update(cx, |this, cx| {
                                            this.update_uri_from_fields(window, cx);
                                        });
                                    }
                                }),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors::text_muted())
                            .child("Keep URI as the source of truth; fields below sync on update."),
                    )
                    .child(if let Some(err) = parse_error {
                        div()
                            .text_xs()
                            .text_color(colors::text_error())
                            .child(err)
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Switch::new("connection-read-only")
                            .checked(self.draft.read_only)
                            .small()
                            .on_click({
                                let view = view.clone();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.draft.read_only = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Read-only (safe mode)"),
                            )
                            .child(div().text_xs().text_color(colors::text_secondary()).child(
                                "Block inserts, updates, deletes, drops, and index changes",
                            )),
                    ),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(spacing::md())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Username"),
                            )
                            .child(Input::new(&self.draft.username_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Password"),
                            )
                            .child(Input::new(&self.draft.password_state).mask_toggle()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("App name"))
                    .child(Input::new(&self.draft.app_name_state)),
            )
            .into_any_element()
    }

    fn render_auth_tab(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("Auth source"))
                    .child(Input::new(&self.draft.auth_source_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div().text_sm().text_color(colors::text_primary()).child("Auth mechanism"),
                    )
                    .child(Input::new(&self.draft.auth_mechanism_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors::text_primary())
                            .child("Auth mechanism properties"),
                    )
                    .child(Input::new(&self.draft.auth_mechanism_props_state)),
            )
            .into_any_element()
    }

    fn render_options_tab(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Switch::new("direct-connection")
                            .checked(self.draft.direct_connection)
                            .small()
                            .on_click({
                                let view = _cx.entity();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.draft.direct_connection = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors::text_primary())
                            .child("Direct connection"),
                    ),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(spacing::md())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Read preference"),
                            )
                            .child(Input::new(&self.draft.read_preference_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Read concern level"),
                            )
                            .child(Input::new(&self.draft.read_concern_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Write concern (w)"),
                            )
                            .child(Input::new(&self.draft.write_concern_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("wTimeoutMS"),
                            )
                            .child(Input::new(&self.draft.w_timeout_state)),
                    ),
            )
            .into_any_element()
    }

    fn render_tls_tab(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(Switch::new("tls-enabled").checked(self.draft.tls).small().on_click({
                        let view = _cx.entity();
                        move |checked, _window, cx| {
                            view.update(cx, |this, cx| {
                                this.draft.tls = *checked;
                                cx.notify();
                            });
                        }
                    }))
                    .child(div().text_sm().text_color(colors::text_primary()).child("TLS enabled")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Switch::new("tls-insecure")
                            .checked(self.draft.tls_insecure)
                            .small()
                            .on_click({
                                let view = _cx.entity();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.draft.tls_insecure = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        div().text_sm().text_color(colors::text_primary()).child("TLS insecure"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("TLS CA file"))
                    .child(Input::new(&self.draft.tls_ca_file_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors::text_primary())
                            .child("TLS certificate key file"),
                    )
                    .child(Input::new(&self.draft.tls_cert_key_file_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors::text_primary())
                            .child("TLS certificate key password"),
                    )
                    .child(Input::new(&self.draft.tls_cert_key_password_state).mask_toggle()),
            )
            .into_any_element()
    }

    fn render_pool_tab(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(spacing::md())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Connect timeout (ms)"),
                            )
                            .child(Input::new(&self.draft.connect_timeout_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Server selection timeout (ms)"),
                            )
                            .child(Input::new(&self.draft.server_selection_timeout_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Max pool size"),
                            )
                            .child(Input::new(&self.draft.max_pool_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Min pool size"),
                            )
                            .child(Input::new(&self.draft.min_pool_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Heartbeat frequency (ms)"),
                            )
                            .child(Input::new(&self.draft.heartbeat_frequency_state)),
                    ),
            )
            .into_any_element()
    }

    fn render_advanced_tab(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child("Compressors"))
                    .child(Input::new(&self.draft.compressors_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors::text_primary())
                            .child("Zlib compression level"),
                    )
                    .child(Input::new(&self.draft.zlib_level_state)),
            )
            .into_any_element()
    }
}
