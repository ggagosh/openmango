use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use uuid::Uuid;

use crate::models::{
    ConnectionColor, ConnectionEnvironment, ConnectionWriteIdentity, SavedConnection,
};
use crate::state::AppState;
use crate::theme::{borders, colors, spacing};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionIdentity {
    pub id: Uuid,
    pub name: String,
    pub color: Option<ConnectionColor>,
    pub environment: Option<ConnectionEnvironment>,
    pub read_only: bool,
    pub confirm_production_writes: bool,
}

impl From<&ConnectionWriteIdentity> for ConnectionIdentity {
    fn from(connection: &ConnectionWriteIdentity) -> Self {
        Self {
            id: connection.id,
            name: connection.name.clone(),
            color: connection.color,
            environment: connection.environment,
            read_only: connection.read_only,
            confirm_production_writes: connection.confirm_production_writes,
        }
    }
}

impl From<&SavedConnection> for ConnectionIdentity {
    fn from(connection: &SavedConnection) -> Self {
        Self {
            id: connection.id,
            name: connection.name.clone(),
            color: connection.color,
            environment: connection.environment,
            read_only: connection.read_only,
            confirm_production_writes: connection.confirm_production_writes,
        }
    }
}

impl ConnectionIdentity {
    pub fn requires_production_write_confirmation(&self) -> bool {
        self.environment == Some(ConnectionEnvironment::Production)
            && self.confirm_production_writes
    }

    pub fn environment_label(&self) -> Option<&'static str> {
        self.environment.map(ConnectionEnvironment::label)
    }

    pub fn display_name(&self) -> String {
        match (self.environment_label(), self.read_only) {
            (Some(environment), true) => format!("{} · {environment} · Read-only", self.name),
            (Some(environment), false) => format!("{} · {environment}", self.name),
            (None, true) => format!("{} · Read-only", self.name),
            (None, false) => self.name.clone(),
        }
    }
}

pub fn connection_identity_for(
    state: &Entity<AppState>,
    connection_id: Uuid,
    include_name: bool,
    cx: &App,
) -> AnyElement {
    state
        .read(cx)
        .connection_by_id(connection_id)
        .map(ConnectionIdentity::from)
        .map(|identity| connection_identity_badge(&identity, include_name, cx))
        .unwrap_or_else(|| div().into_any_element())
}

pub fn connection_identity_badge(
    identity: &ConnectionIdentity,
    include_name: bool,
    cx: &App,
) -> AnyElement {
    let environment = identity.environment_label();
    let mut row = div().flex().items_center().gap(spacing::xs()).min_w(px(0.0));
    if let Some(color) = identity.color {
        row = row.child(
            div()
                .size(px(8.0))
                .flex_shrink_0()
                .rounded_full()
                .bg(colors::connection_accent(color, cx)),
        );
    }
    if include_name {
        row = row.child(
            div()
                .min_w(px(0.0))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_sm()
                .text_color(cx.theme().foreground)
                .child(identity.name.clone()),
        );
    }
    if let Some(environment) = environment {
        row = row.child(
            div()
                .px(spacing::xs())
                .py(px(1.0))
                .rounded(borders::radius_sm())
                .flex_shrink_0()
                .bg(if identity.environment == Some(ConnectionEnvironment::Production) {
                    cx.theme().danger.opacity(0.14)
                } else {
                    cx.theme().secondary
                })
                .text_xs()
                .text_color(if identity.environment == Some(ConnectionEnvironment::Production) {
                    cx.theme().danger
                } else {
                    cx.theme().secondary_foreground
                })
                .child(environment),
        );
    }
    if identity.read_only {
        row = row.child(
            div()
                .px(spacing::xs())
                .py(px(1.0))
                .rounded(borders::radius_sm())
                .flex_shrink_0()
                .bg(cx.theme().warning.opacity(0.16))
                .text_xs()
                .text_color(cx.theme().warning)
                .child("RO"),
        );
    }
    row.into_any_element()
}
