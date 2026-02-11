//! Reusable form field component for label + input patterns.

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::input::{Input, InputState};

use crate::theme::spacing;

/// A reusable form field component that renders a label above an input.
pub struct FormField {
    label: SharedString,
    input: Entity<InputState>,
    required: bool,
    description: Option<SharedString>,
    disabled: bool,
}

impl FormField {
    pub fn new(label: impl Into<SharedString>, input: &Entity<InputState>) -> Self {
        Self {
            label: label.into(),
            input: input.clone(),
            required: false,
            description: None,
            disabled: false,
        }
    }

    /// Mark this field as required (shows asterisk).
    #[allow(dead_code)]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Add a description/help text below the label.
    #[allow(dead_code)]
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Disable the input field.
    #[allow(dead_code)]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Render the form field into an element.
    pub fn render(self, cx: &App) -> impl IntoElement {
        let mut label_text = self.label.to_string();
        if self.required {
            label_text.push_str(" *");
        }

        let mut field = div()
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .child(div().text_sm().text_color(cx.theme().foreground).child(label_text));

        if let Some(description) = self.description {
            field = field.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(description.to_string()),
            );
        }

        field.child(Input::new(&self.input).disabled(self.disabled))
    }
}

/// Create a form field from a label and input state.
/// Convenience function for inline usage.
#[allow(dead_code)]
pub fn form_field(label: impl Into<SharedString>, input: &Entity<InputState>) -> FormField {
    FormField::new(label, input)
}
