//! Tab rendering methods for ConnectionManager.
//!
//! Each tab (General, Auth, Options, TLS, Pool, Advanced) is rendered here.

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::input::Input;
use gpui_component::switch::Switch;

use crate::components::Button;
use crate::theme::spacing;

use super::ConnectionManager;

impl ConnectionManager {
    pub(super) fn render_general_tab(
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
                    .child(div().text_sm().text_color(cx.theme().foreground).child("Name"))
                    .child(Input::new(&self.draft.name_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(cx.theme().foreground).child("URI"))
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
                            .text_color(cx.theme().muted_foreground)
                            .child("Keep URI as the source of truth; fields below sync on update."),
                    )
                    .child(if let Some(err) = parse_error {
                        div()
                            .text_xs()
                            .text_color(cx.theme().danger_foreground)
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
                                    .text_color(cx.theme().foreground)
                                    .child("Read-only (safe mode)"),
                            )
                            .child(
                                div().text_xs().text_color(cx.theme().secondary_foreground).child(
                                    "Block inserts, updates, deletes, drops, and index changes",
                                ),
                            ),
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
                                div().text_sm().text_color(cx.theme().foreground).child("Username"),
                            )
                            .child(Input::new(&self.draft.username_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div().text_sm().text_color(cx.theme().foreground).child("Password"),
                            )
                            .child(Input::new(&self.draft.password_state).mask_toggle()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(cx.theme().foreground).child("App name"))
                    .child(Input::new(&self.draft.app_name_state)),
            )
            .into_any_element()
    }

    pub(super) fn render_auth_tab(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(cx.theme().foreground).child("Auth source"))
                    .child(Input::new(&self.draft.auth_source_state)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div().text_sm().text_color(cx.theme().foreground).child("Auth mechanism"),
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
                            .text_color(cx.theme().foreground)
                            .child("Auth mechanism properties"),
                    )
                    .child(Input::new(&self.draft.auth_mechanism_props_state)),
            )
            .into_any_element()
    }

    pub(super) fn render_options_tab(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                                let view = cx.entity();
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
                            .text_color(cx.theme().foreground)
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
                                    .text_color(cx.theme().foreground)
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
                                    .text_color(cx.theme().foreground)
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
                                    .text_color(cx.theme().foreground)
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
                                    .text_color(cx.theme().foreground)
                                    .child("wTimeoutMS"),
                            )
                            .child(Input::new(&self.draft.w_timeout_state)),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_tls_tab(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                        let view = cx.entity();
                        move |checked, _window, cx| {
                            view.update(cx, |this, cx| {
                                this.draft.tls = *checked;
                                cx.notify();
                            });
                        }
                    }))
                    .child(div().text_sm().text_color(cx.theme().foreground).child("TLS enabled")),
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
                                let view = cx.entity();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.draft.tls_insecure = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(div().text_sm().text_color(cx.theme().foreground).child("TLS insecure")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(cx.theme().foreground).child("TLS CA file"))
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
                            .text_color(cx.theme().foreground)
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
                            .text_color(cx.theme().foreground)
                            .child("TLS certificate key password"),
                    )
                    .child(Input::new(&self.draft.tls_cert_key_password_state).mask_toggle()),
            )
            .into_any_element()
    }

    pub(super) fn render_pool_tab(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                                    .text_color(cx.theme().foreground)
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
                                    .text_color(cx.theme().foreground)
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
                                    .text_color(cx.theme().foreground)
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
                                    .text_color(cx.theme().foreground)
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
                                    .text_color(cx.theme().foreground)
                                    .child("Heartbeat frequency (ms)"),
                            )
                            .child(Input::new(&self.draft.heartbeat_frequency_state)),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_advanced_tab(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(cx.theme().foreground).child("Compressors"))
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
                            .text_color(cx.theme().foreground)
                            .child("Zlib compression level"),
                    )
                    .child(Input::new(&self.draft.zlib_level_state)),
            )
            .into_any_element()
    }
}
