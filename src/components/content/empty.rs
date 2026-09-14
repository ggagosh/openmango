use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use uuid::Uuid;

use crate::components::{ConnectionIdentity, ConnectionManager, connection_identity_tags};
use crate::keyboard::OpenConnectionSwitcher;
use crate::state::{AppCommands, AppState};
use crate::theme::{colors, sizing, spacing};

const WELCOME_LIST_WIDTH: f32 = 360.0;

pub(crate) fn render_empty_state(hint: String, cx: &App) -> AnyElement {
    centered(brand(cx).child(
        div().mt(spacing::lg()).text_sm().text_color(cx.theme().muted_foreground).child(hint),
    ))
}

/// Shown while nothing is connected: the most recently used connections, one click each.
pub(crate) fn render_welcome(
    state: Entity<AppState>,
    recent: Vec<ConnectionIdentity>,
    connecting: Option<Uuid>,
    window: &Window,
    cx: &App,
) -> AnyElement {
    if recent.is_empty() {
        return centered(
            brand(cx)
                .child(
                    div()
                        .mt(spacing::lg())
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Add a MongoDB connection to get started."),
                )
                .child(
                    Button::new("welcome-new-connection")
                        .primary()
                        .icon(Icon::new(IconName::Plus).xsmall())
                        .label("New connection")
                        .on_click(move |_, window, cx| {
                            ConnectionManager::open_new(state.clone(), window, cx);
                        }),
                ),
        );
    }

    let switcher_shortcut =
        window.highest_precedence_binding_for_action(&OpenConnectionSwitcher).map(|binding| {
            binding.keystrokes().iter().map(ToString::to_string).collect::<Vec<_>>().join(" ")
        });

    let mut list = div().flex().flex_col().gap(px(2.0)).w(px(WELCOME_LIST_WIDTH)).child(
        div()
            .px(spacing::md())
            .pb(spacing::xs())
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child("Recent connections"),
    );
    for identity in recent {
        list = list.child(recent_row(state.clone(), identity, connecting, cx));
    }

    let footer = div()
        .flex()
        .items_center()
        .justify_between()
        .mt(spacing::sm())
        .child(
            Button::new("welcome-all-connections")
                .ghost()
                .small()
                .label("All connections")
                .when_some(switcher_shortcut, |button, shortcut| {
                    button.child(
                        div()
                            .ml(spacing::xs())
                            .text_color(cx.theme().muted_foreground)
                            .child(shortcut),
                    )
                })
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(OpenConnectionSwitcher), cx);
                }),
        )
        .child(
            Button::new("welcome-new-connection")
                .ghost()
                .small()
                .icon(Icon::new(IconName::Plus).xsmall())
                .label("New connection")
                .on_click({
                    let state = state.clone();
                    move |_, window, cx| ConnectionManager::open_new(state.clone(), window, cx)
                }),
        );

    centered(brand(cx).child(div().mt(spacing::lg()).child(list.child(footer))))
}

fn recent_row(
    state: Entity<AppState>,
    identity: ConnectionIdentity,
    connecting: Option<Uuid>,
    cx: &App,
) -> AnyElement {
    let id = identity.id;
    let is_connecting = connecting == Some(id);
    let icon_color = identity
        .color
        .map(|color| colors::connection_accent(color, cx))
        .unwrap_or(cx.theme().muted_foreground);

    Button::new(SharedString::from(format!("welcome-connect-{id}")))
        .ghost()
        .w_full()
        .h(px(34.0))
        .accessibility_label(format!("Connect to {}", identity.name))
        // One connection opens at a time; the others wait until it settles.
        .disabled(connecting.is_some() && !is_connecting)
        .child(
            div()
                .flex()
                .w_full()
                .min_w(px(0.0))
                .items_center()
                .gap(spacing::sm())
                .child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .size(sizing::icon_md())
                        .child(if is_connecting {
                            Spinner::new()
                                .with_size(sizing::icon_sm())
                                .color(cx.theme().muted_foreground)
                                .into_any_element()
                        } else {
                            Icon::new(IconName::Globe)
                                .size(sizing::icon_md())
                                .text_color(icon_color)
                                .into_any_element()
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .truncate()
                        .child(identity.name.clone()),
                )
                .child(connection_identity_tags(&identity, cx)),
        )
        .on_click(move |_, _, cx| {
            if !is_connecting {
                AppCommands::connect(state.clone(), id, cx);
            }
        })
        .into_any_element()
}

fn brand(cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(spacing::lg())
        .items_center()
        .child(img("logo/openmango.png").w(px(120.0)).h(px(120.0)))
        .child(
            div()
                .text_2xl()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().primary)
                .font_family(crate::theme::fonts::heading())
                .child("OpenMango"),
        )
        .child(
            div()
                .text_base()
                .text_color(cx.theme().secondary_foreground)
                .child("MongoDB GUI Client"),
        )
}

fn centered(content: Div) -> AnyElement {
    div().flex().flex_1().items_center().justify_center().child(content).into_any_element()
}
