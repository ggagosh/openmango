//! Reusable form field component for label + input patterns.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;

use crate::theme::spacing;

/// A reusable form field component that renders a label above an input.
pub struct FormField {
    label: SharedString,
    input: Entity<InputState>,
}

impl FormField {
    pub fn new(label: impl Into<SharedString>, input: &Entity<InputState>) -> Self {
        Self { label: label.into(), input: input.clone() }
    }

    /// Render the form field into an element.
    pub fn render(self, cx: &App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .child(div().text_sm().text_color(cx.theme().foreground).child(self.label))
            .child(Input::new(&self.input))
    }
}
