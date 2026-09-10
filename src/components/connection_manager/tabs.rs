//! Tab rendering methods for ConnectionManager.
//!
//! Each tab (General, TLS, Network, Advanced) is rendered here.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{ButtonGroup, ButtonVariants as _};
use gpui_kit::component::collapsible::Collapsible;
use gpui_kit::component::form::{field, v_form};
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::Button;
use crate::models::{ConnectionColor, ConnectionEnvironment};
use crate::theme::{colors, spacing};

use super::ConnectionManager;

impl ConnectionManager {
    pub(super) fn render_general_tab(
        &mut self,
        parse_error: Option<String>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let selected_color = self.draft.color;
        let no_color_button = Button::new("connection-color-none")
            .xsmall()
            .label("None")
            .selected(selected_color.is_none())
            .on_click({
                let view = view.clone();
                move |_, _window, cx| {
                    view.update(cx, |this, cx| {
                        this.draft.color = None;
                        cx.notify();
                    });
                }
            });
        let color_buttons = ConnectionColor::ALL
            .into_iter()
            .map(|color| {
                let accent = colors::connection_accent(color, cx);
                let view = view.clone();
                Button::new(("connection-color", color as usize))
                    .selected(selected_color == Some(color))
                    .xsmall()
                    .child(div().size(px(12.0)).rounded_full().bg(accent))
                    .tooltip(color.label())
                    .on_click(move |_, _window, cx| {
                        view.update(cx, |this, cx| {
                            this.draft.color = Some(color);
                            cx.notify();
                        });
                    })
            })
            .collect::<Vec<_>>();

        div().flex().flex_col().gap(spacing::lg())
            .child(v_form()
                .child(field().label("Connection URI")
                    .description("Paste a MongoDB URI. Auth, TLS, and advanced options are filled automatically.")
                    .child(Input::new(&self.draft.uri_state).font_family(crate::theme::fonts::mono())))
                .child(field().label("Name")
                    .description("Optional. Defaults to the server name.")
                    .child(Input::new(&self.draft.name_state))))
            .when_some(parse_error, |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            // Connection color
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div().text_sm().text_color(cx.theme().foreground).child("Connection color"),
                    )
                    .child(
                        ButtonGroup::new("connection-colors").small().child(no_color_button)
                            .children(color_buttons),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Accents this connection in the sidebar and tabs."),
                    ),
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

            .into_any_element()
    }

    pub(super) fn render_authentication_tab(&self) -> AnyElement {
        v_form()
            .child(field().label("Username").child(Input::new(&self.draft.username_state)))
            .child(
                field()
                    .label("Password")
                    .child(Input::new(&self.draft.password_state).mask_toggle()),
            )
            .child(
                field()
                    .label("Authentication database")
                    .description("Leave empty to use the URI database or MongoDB default.")
                    .child(Input::new(&self.draft.auth_source_state)),
            )
            .child(
                field()
                    .label("Authentication mechanism")
                    .description("Optional. MongoDB negotiates the mechanism when empty.")
                    .child(Input::new(&self.draft.auth_mechanism_state)),
            )
            .child(
                field()
                    .label("Mechanism properties")
                    .child(Input::new(&self.draft.auth_mechanism_props_state)),
            )
            .into_any_element()
    }

    pub(super) fn render_access_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let selected_environment = self.draft.environment;
        let no_environment_button = Button::new("connection-environment-none")
            .selected(selected_environment.is_none())
            .xsmall()
            .label("Not set")
            .on_click({
                let view = view.clone();
                move |_, _window, cx| {
                    view.update(cx, |this, cx| {
                        this.draft.environment = None;
                        cx.notify();
                    });
                }
            });
        let environment_buttons = ConnectionEnvironment::ALL
            .into_iter()
            .map(|environment| {
                let view = view.clone();
                Button::new(("connection-environment", environment as usize))
                    .selected(selected_environment == Some(environment))
                    .xsmall()
                    .label(environment.label())
                    .on_click(move |_, _window, cx| {
                        view.update(cx, |this, cx| {
                            if environment == ConnectionEnvironment::Production
                                && this.draft.environment != Some(ConnectionEnvironment::Production)
                            {
                                this.draft.agent_shared = false;
                                this.draft.agent_writable = false;
                            }
                            this.draft.environment = Some(environment);
                            cx.notify();
                        });
                    })
            })
            .collect::<Vec<_>>();

        div().flex().flex_col().gap(spacing::lg())
            // Environment identity
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(cx.theme().foreground).child("Environment"))
                    .child(
                        ButtonGroup::new("connection-environments").small().child(no_environment_button)
                            .children(environment_buttons),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Selected explicitly; OpenMango never infers Production from a hostname."),
                    ),
            )
            .when(self.draft.environment == Some(ConnectionEnvironment::Production), |content| {
                content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::sm())
                        .child(
                            Switch::new("confirm-production-writes")
                                .checked(self.draft.confirm_production_writes)
                                .small()
                                .on_click({
                                    let view = view.clone();
                                    move |checked, _window, cx| {
                                        view.update(cx, |this, cx| {
                                            this.draft.confirm_production_writes = *checked;
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
                                        .child("Confirm Production writes and Forge"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().secondary_foreground)
                                        .child("Require an additional review before writes and Forge execution."),
                                ),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Switch::new("connection-history")
                            .checked(self.draft.history_enabled)
                            .small()
                            .disabled(!self.draft.history_enabled)
                            .on_click({
                                let view = view.clone();
                                move |checked, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.draft.history_enabled = *checked;
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
                                    .child("History"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().secondary_foreground)
                                    .child("Record encrypted update, replace, and delete events from all clients when the server is eligible."),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().warning)
                                    .child("Connect and use Settings to inspect eligibility, enable pre/post images, and configure retention."),
                            ),
                    ),
            )
            // Agent access
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(
                                Switch::new("connection-agent-shared")
                                    .checked(self.draft.agent_shared)
                                    .small()
                                    .on_click({
                                        let view = view.clone();
                                        move |checked, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.draft.agent_shared = *checked;
                                                if !*checked {
                                                    this.draft.agent_writable = false;
                                                }
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
                                            .child("Share with agents"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().secondary_foreground)
                                            .child("Allow authenticated MCP clients to see and use this connection. Credentials are never exposed."),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(
                                Switch::new("connection-protected")
                                    .checked(self.draft.protected)
                                    .small()
                                    .on_click({
                                        let view = view.clone();
                                        move |checked, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                if *checked {
                                                    this.draft.agent_shared = false;
                                                    this.draft.agent_writable = false;
                                                }
                                                this.draft.protected = *checked;
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
                                            .child("Protected connection"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().secondary_foreground)
                                            .child("Require Production-level safeguards for agent access."),
                                    ),
                            ),
                    )
                    .when(
                        self.draft.agent_shared
                            && (self.draft.protected
                                || self.draft.environment
                                    == Some(ConnectionEnvironment::Production)),
                        |content| {
                            content.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().warning)
                                    .child("This protected connection will be visible to agents. Direct writes remain disabled unless explicitly enabled in Settings."),
                            )
                        },
                    ),
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
            .child(v_form().child(
                field().label("TLS CA file").child(Input::new(&self.draft.tls_ca_file_state)),
            ))
            .child(
                v_form().child(
                    field()
                        .label("TLS certificate key file")
                        .child(Input::new(&self.draft.tls_cert_key_file_state)),
                ),
            )
            .child(
                v_form().child(
                    field()
                        .label("TLS certificate key password")
                        .child(Input::new(&self.draft.tls_cert_key_password_state).mask_toggle()),
                ),
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

        let ssh_auth_block =
            if self.draft.ssh_use_identity_file {
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(spacing::md())
                    .child(
                        v_form().child(
                            field()
                                .label("Identity file")
                                .child(Input::new(&self.draft.ssh_identity_file_state)),
                        ),
                    )
                    .child(v_form().child(field().label("Identity passphrase").child(
                        Input::new(&self.draft.ssh_identity_passphrase_state).mask_toggle(),
                    )))
                    .into_any_element()
            } else {
                v_form()
                    .child(
                        field()
                            .label("SSH password")
                            .child(Input::new(&self.draft.ssh_password_state).mask_toggle()),
                    )
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
                        .rounded(crate::theme::borders::radius_sm())
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
                            .child(v_form().child(
                                field().label("Host").child(Input::new(&self.draft.ssh_host_state)),
                            ))
                            .child(v_form().child(
                                field().label("Port").child(Input::new(&self.draft.ssh_port_state)),
                            ))
                            .child(
                                v_form().child(
                                    field()
                                        .label("Username")
                                        .child(Input::new(&self.draft.ssh_username_state)),
                                ),
                            )
                            .child(
                                v_form().child(
                                    field()
                                        .label("Local bind host")
                                        .child(Input::new(&self.draft.ssh_local_bind_host_state)),
                                ),
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
                                v_form().child(
                                    field()
                                        .label("Proxy host")
                                        .child(Input::new(&self.draft.proxy_host_state)),
                                ),
                            )
                            .child(
                                v_form().child(
                                    field()
                                        .label("Proxy port")
                                        .child(Input::new(&self.draft.proxy_port_state)),
                                ),
                            )
                            .child(
                                v_form().child(
                                    field()
                                        .label("Proxy username")
                                        .child(Input::new(&self.draft.proxy_username_state)),
                                ),
                            )
                            .child(v_form().child(field().label("Proxy password").child(
                                Input::new(&self.draft.proxy_password_state).mask_toggle(),
                            ))),
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
            .child(v_form().child(
                field().label("Application name").child(Input::new(&self.draft.app_name_state)),
            ))
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
                        v_form().child(
                            field()
                                .label("Read preference")
                                .child(Input::new(&self.draft.read_preference_state)),
                        ),
                    )
                    .child(
                        v_form().child(
                            field()
                                .label("Read concern level")
                                .child(Input::new(&self.draft.read_concern_state)),
                        ),
                    )
                    .child(
                        v_form().child(
                            field()
                                .label("Write concern (w)")
                                .child(Input::new(&self.draft.write_concern_state)),
                        ),
                    )
                    .child(v_form().child(
                        field().label("wTimeoutMS").child(Input::new(&self.draft.w_timeout_state)),
                    )),
            )
            // Pool & Timeouts collapsible
            .child(
                Collapsible::new()
                    .items_start()
                    .open(pool_expanded)
                    .child(
                        Button::new("pool-toggle")
                            .xsmall()
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
                                v_form().child(
                                    field()
                                        .label("Connect timeout (ms)")
                                        .child(Input::new(&self.draft.connect_timeout_state)),
                                ),
                            )
                            .child(
                                v_form().child(
                                    field().label("Server selection timeout (ms)").child(
                                        Input::new(&self.draft.server_selection_timeout_state),
                                    ),
                                ),
                            )
                            .child(
                                v_form().child(
                                    field()
                                        .label("Max pool size")
                                        .child(Input::new(&self.draft.max_pool_state)),
                                ),
                            )
                            .child(
                                v_form().child(
                                    field()
                                        .label("Min pool size")
                                        .child(Input::new(&self.draft.min_pool_state)),
                                ),
                            )
                            .child(
                                v_form().child(
                                    field()
                                        .label("Heartbeat frequency (ms)")
                                        .child(Input::new(&self.draft.heartbeat_frequency_state)),
                                ),
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
                            .xsmall()
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
                                v_form().child(
                                    field()
                                        .label("Compressors")
                                        .child(Input::new(&self.draft.compressors_state)),
                                ),
                            )
                            .child(
                                v_form().child(
                                    field()
                                        .label("Zlib compression level")
                                        .child(Input::new(&self.draft.zlib_level_state)),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }
}
