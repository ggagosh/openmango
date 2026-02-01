//! Dialog helper utilities to reduce boilerplate in dialog creation.

use gpui::*;
use gpui_component::WindowExt as _;

use crate::components::Button;

/// Creates a standard Cancel button that closes the dialog.
///
/// # Example
/// ```ignore
/// vec![
///     cancel_button("cancel-db"),
///     Button::new("create-db").primary().label("Create").on_click(...).into_any_element(),
/// ]
/// ```
pub fn cancel_button(id: impl Into<ElementId>) -> AnyElement {
    Button::new(id)
        .label("Cancel")
        .on_click(|_, window, cx| {
            window.close_dialog(cx);
        })
        .into_any_element()
}

/// Creates a standard primary action button for dialogs.
///
/// # Example
/// ```ignore
/// vec![
///     cancel_button("cancel-db"),
///     primary_button("create-db", "Create", move |window, cx| {
///         // Handle submit
///         window.close_dialog(cx);
///     }),
/// ]
/// ```
pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    Button::new(id)
        .primary()
        .label(label)
        .on_click(move |_, window, cx| {
            on_click(window, cx);
        })
        .into_any_element()
}

/// Creates a standard secondary button for dialogs.
#[allow(dead_code)]
pub fn secondary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    Button::new(id)
        .label(label)
        .on_click(move |_, window, cx| {
            on_click(window, cx);
        })
        .into_any_element()
}
