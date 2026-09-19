//! Busy state for label-only buttons.

use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Sizable as _, Size};
use gpui_kit::*;

use crate::components::Button;

/// A label-only button that shows a spinner in place of its label while `busy`. The kit's
/// `loading` only swaps an icon for the spinner, so a button without one just dimmed. The label
/// stays in layout, transparent, so the button keeps its size and nothing around it moves.
pub fn busy_label(
    button: Button,
    size: Size,
    label: impl Into<SharedString>,
    busy: bool,
) -> Button {
    let label = label.into();
    let button = button.with_size(size).loading(busy);
    if !busy {
        return button.label(label);
    }
    button.accessibility_label(label.clone()).child(
        div()
            .relative()
            .whitespace_nowrap()
            .line_height(relative(1.))
            .child(div().text_color(transparent_black()).child(label))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Spinner::new().with_size(size)),
            ),
    )
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::{Root, Size};
    use gpui_kit::{
        AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
        Styled as _, TestAppContext, Window, div,
    };

    use super::busy_label;
    use crate::components::Button;

    struct Buttons;

    impl Render for Buttons {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let button = |id: &'static str, size, busy| {
                div().debug_selector(move || id.into()).child(busy_label(
                    Button::new(id),
                    size,
                    "Load more",
                    busy,
                ))
            };
            div()
                .flex()
                .flex_col()
                .items_start()
                .child(button("xsmall-idle", Size::XSmall, false))
                .child(button("xsmall-busy", Size::XSmall, true))
                .child(button("medium-idle", Size::Medium, false))
                .child(button("medium-busy", Size::Medium, true))
        }
    }

    #[gpui_kit::test]
    fn a_busy_button_keeps_its_size(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply_design_tokens(cx);
        });
        let (_, cx) = cx.add_window_view(|window, cx| {
            let buttons = cx.new(|_| Buttons);
            Root::new(buttons, window, cx).bordered(false)
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let size =
            |bounds: Option<gpui_kit::Bounds<gpui_kit::Pixels>>| bounds.expect("button").size;
        assert_eq!(
            size(cx.debug_bounds("xsmall-busy")),
            size(cx.debug_bounds("xsmall-idle")),
            "xsmall"
        );
        assert_eq!(
            size(cx.debug_bounds("medium-busy")),
            size(cx.debug_bounds("medium-idle")),
            "medium"
        );
    }
}
