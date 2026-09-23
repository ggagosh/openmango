use gpui_kit::base::{Checkbox, CheckboxState};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Native checkbox semantics, including the mixed state, with the app's compact styling.
pub fn tri_checkbox(
    id: impl Into<ElementId>,
    state: CheckboxState,
    label: impl Into<SharedString>,
    disabled: bool,
    cx: &App,
) -> Checkbox {
    let label = label.into();
    let accent = cx.theme().primary;
    Checkbox::new(id)
        .state(state)
        .disabled(disabled)
        .accessibility_label(label.clone())
        .flex()
        .items_center()
        .gap_2()
        .min_h(px(24.0))
        .px_1()
        .rounded_sm()
        .cursor_pointer()
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .focus_visible(move |style| style.border_color(accent))
        .when(disabled, |checkbox| checkbox.opacity(0.5))
        .child(
            div()
                .size(px(14.0))
                .flex_shrink_0()
                .rounded(px(3.0))
                .border_1()
                .border_color(if state == CheckboxState::Unchecked {
                    cx.theme().input
                } else {
                    accent
                })
                .bg(if state == CheckboxState::Unchecked { cx.theme().background } else { accent })
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .line_height(relative(1.0))
                .text_color(cx.theme().primary_foreground)
                .child(match state {
                    CheckboxState::Unchecked => "",
                    CheckboxState::Checked => "✓",
                    CheckboxState::Indeterminate => "−",
                }),
        )
        .when(!label.is_empty(), |checkbox| checkbox.child(div().text_xs().child(label)))
}
