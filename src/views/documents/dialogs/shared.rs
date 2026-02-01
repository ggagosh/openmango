//! Shared utilities for document dialogs.

use gpui::*;
use gpui_component::button::{Button as MenuButton, ButtonCustomVariant, ButtonVariants};
use gpui_component::{Sizable as _, Size, StyledExt as _, WindowExt as _};

use crate::theme::{borders, colors, spacing};

/// Creates a standardized dropdown button variant used across dialogs.
pub fn dropdown_variant(cx: &mut App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .color(colors::bg_button_secondary().into())
        .foreground(colors::text_primary().into())
        .border(colors::border_subtle().into())
        .hover(colors::bg_button_secondary_hover().into())
        .active(colors::bg_button_secondary_hover().into())
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
) -> (String, Hsla) {
    if let Some(error) = error_message {
        (error.clone(), colors::text_error().into())
    } else if updating {
        (updating_label.to_string(), colors::text_muted().into())
    } else {
        (default_label.to_string(), colors::text_muted().into())
    }
}
