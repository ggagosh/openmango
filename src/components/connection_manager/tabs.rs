//! Tab rendering methods for ConnectionManager.
//!
//! Each tab (General, TLS, Network, Advanced) is rendered here.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::collapsible::Collapsible;
use gpui_component::input::Input;
use gpui_component::switch::Switch;

use crate::components::Button;
use crate::theme::spacing;

use super::ConnectionManager;

impl ConnectionManager {
    /// General tab: Name, URI, Read-only, Username/Password, App name, Auth fields.
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
            // Name
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(cx.theme().foreground).child("Name"))
                    .child(Input::new(&self.draft.name_state)),
            )
            // URI
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
            // Read-only switch
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
            // Username / Password
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
            // App name
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(cx.theme().foreground).child("App name"))
                    .child(Input::new(&self.draft.app_name_state)),
            )
            // Auth fields
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

    /// TLS tab: unchanged.
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

    /// Network tab: SSH Tunnel + SOCKS5 Proxy with spacing-only section labels.
    pub(super) fn render_network_tab(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();

        let ssh_auth_block = if self.draft.ssh_use_identity_file {
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
                                .child("Identity file"),
                        )
                        .child(Input::new(&self.draft.ssh_identity_file_state)),
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
                                .child("Identity passphrase"),
                        )
                        .child(Input::new(&self.draft.ssh_identity_passphrase_state).mask_toggle()),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .child(div().text_sm().text_color(cx.theme().foreground).child("SSH password"))
                .child(Input::new(&self.draft.ssh_password_state).mask_toggle())
                .into_any_element()
        };

        let both_enabled = self.draft.ssh_enabled && self.draft.proxy_enabled;

        div()
            .flex()
            .flex_col()
            // Mutual-exclusion warning
            .when(both_enabled, |this| {
                this.child(
                    div()
                        .mb(spacing::md())
                        .rounded_md()
                        .bg(cx.theme().danger.opacity(0.08))
                        .px(spacing::sm())
                        .py(spacing::xs())
                        .text_xs()
                        .text_color(cx.theme().danger_foreground)
                        .child("SSH tunnel and SOCKS5 proxy cannot be enabled together."),
                )
            })
            // SSH Tunnel section
            .child(
                div()
                    .mt(px(28.0))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("SSH TUNNEL"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::lg())
                    .mt(spacing::md())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(
                                Switch::new("ssh-enabled")
                                    .checked(self.draft.ssh_enabled)
                                    .small()
                                    .on_click({
                                        let view = view.clone();
                                        move |checked, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.draft.ssh_enabled = *checked;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().foreground)
                                    .child("Enable SSH tunnel"),
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
                                            .child("Host"),
                                    )
                                    .child(Input::new(&self.draft.ssh_host_state)),
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
                                            .child("Port"),
                                    )
                                    .child(Input::new(&self.draft.ssh_port_state)),
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
                                            .child("Username"),
                                    )
                                    .child(Input::new(&self.draft.ssh_username_state)),
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
                                            .child("Local bind host"),
                                    )
                                    .child(Input::new(&self.draft.ssh_local_bind_host_state)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(
                                Switch::new("ssh-use-identity")
                                    .checked(self.draft.ssh_use_identity_file)
                                    .small()
                                    .on_click({
                                        let view = view.clone();
                                        move |checked, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.draft.ssh_use_identity_file = *checked;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().foreground)
                                    .child("Use identity file auth"),
                            ),
                    )
                    .child(ssh_auth_block)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(
                                Switch::new("ssh-strict-host-key")
                                    .checked(self.draft.ssh_strict_host_key_checking)
                                    .small()
                                    .on_click({
                                        let view = view.clone();
                                        move |checked, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.draft.ssh_strict_host_key_checking = *checked;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().foreground)
                                    .child("Strict host key checking"),
                            ),
                    )
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                        "Use SSH tunnel when MongoDB is only reachable \
                                 through a bastion host.",
                    )),
            )
            // SOCKS5 Proxy section
            .child(
                div()
                    .mt(px(28.0))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("SOCKS5 PROXY"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::lg())
                    .mt(spacing::md())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(
                                Switch::new("proxy-enabled")
                                    .checked(self.draft.proxy_enabled)
                                    .small()
                                    .on_click({
                                        let view = view.clone();
                                        move |checked, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.draft.proxy_enabled = *checked;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().foreground)
                                    .child("Enable SOCKS5 proxy"),
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
                                            .child("Proxy host"),
                                    )
                                    .child(Input::new(&self.draft.proxy_host_state)),
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
                                            .child("Proxy port"),
                                    )
                                    .child(Input::new(&self.draft.proxy_port_state)),
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
                                            .child("Proxy username"),
                                    )
                                    .child(Input::new(&self.draft.proxy_username_state)),
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
                                            .child("Proxy password"),
                                    )
                                    .child(
                                        Input::new(&self.draft.proxy_password_state).mask_toggle(),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Only SOCKS5 proxy is supported. Credentials are optional."),
                    ),
            )
            .into_any_element()
    }

    /// Advanced tab: Options + collapsible Pool & Timeouts + collapsible Compression.
    pub(super) fn render_advanced_tab(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let pool_expanded = self.draft.pool_expanded;
        let compression_expanded = self.draft.compression_expanded;

        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            // Direct connection switch
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
                                let view = view.clone();
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
            // Read/Write concern grid
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
            // Pool & Timeouts collapsible
            .child(
                Collapsible::new()
                    .items_start()
                    .open(pool_expanded)
                    .child(
                        Button::new("pool-toggle")
                            .compact()
                            .ghost()
                            .label(if pool_expanded {
                                "▼ Pool & Timeouts"
                            } else {
                                "▶ Pool & Timeouts"
                            })
                            .on_click({
                                let view = view.clone();
                                move |_, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.draft.pool_expanded = !this.draft.pool_expanded;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .content(
                        div()
                            .grid()
                            .grid_cols(2)
                            .gap(spacing::md())
                            .mt(spacing::sm())
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
                    ),
            )
            // Compression collapsible
            .child(
                Collapsible::new()
                    .items_start()
                    .open(compression_expanded)
                    .child(
                        Button::new("compression-toggle")
                            .compact()
                            .ghost()
                            .label(if compression_expanded {
                                "▼ Compression"
                            } else {
                                "▶ Compression"
                            })
                            .on_click({
                                let view = view.clone();
                                move |_, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.draft.compression_expanded =
                                            !this.draft.compression_expanded;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .content(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::lg())
                            .mt(spacing::sm())
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(spacing::xs())
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().foreground)
                                            .child("Compressors"),
                                    )
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
                            ),
                    ),
            )
            .into_any_element()
    }
}
