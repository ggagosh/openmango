//! Shared utilities for document dialogs.

use gpui::*;
use gpui_component::button::{Button as MenuButton, ButtonCustomVariant, ButtonVariants};
use gpui_component::{ActiveTheme as _, Sizable as _, Size, StyledExt as _, WindowExt as _};

use crate::theme::{borders, spacing};

/// Creates a standardized dropdown button variant used across dialogs.
pub fn dropdown_variant(cx: &mut App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .color(cx.theme().secondary)
        .foreground(cx.theme().foreground)
        .border(cx.theme().sidebar_border)
        .hover(cx.theme().secondary_hover)
        .active(cx.theme().secondary_hover)
        .shadow(false)
}

/// Creates the standard style refinement for dropdown buttons.
pub fn dropdown_style() -> StyleRefinement {
    StyleRefinement::default()
        .font_family(crate::theme::fonts::ui())
        .font_weight(FontWeight::NORMAL)
        .text_size(crate::theme::typography::text_xs())
        .h(px(22.0))
        .px(spacing::sm())
        .py(px(2.0))
}

/// Creates a standardized dropdown button with consistent styling.
pub fn styled_dropdown_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &mut App,
) -> MenuButton {
    let variant = dropdown_variant(cx);
    MenuButton::new(id)
        .compact()
        .label(label)
        .dropdown_caret(true)
        .custom(variant)
        .rounded(borders::radius_sm())
        .with_size(Size::XSmall)
        .refine_style(&dropdown_style())
}

/// Creates an escape key subscription that closes the dialog.
pub fn escape_key_subscription<V: 'static>(cx: &mut Context<V>) -> Subscription {
    cx.intercept_keystrokes(move |event, window, cx| {
        let key = event.keystroke.key.to_ascii_lowercase();
        if key == "escape" {
            window.close_dialog(cx);
            cx.stop_propagation();
        }
    })
}

/// Returns the status text and color for dialog status display.
pub fn status_text(
    error_message: Option<&String>,
    updating: bool,
    updating_label: &str,
    default_label: &str,
    cx: &App,
) -> (String, Hsla) {
    if let Some(error) = error_message {
        (error.clone(), cx.theme().danger_foreground)
    } else if updating {
        (updating_label.to_string(), cx.theme().muted_foreground)
    } else {
        (default_label.to_string(), cx.theme().muted_foreground)
    }
}
