use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonGroup};
use gpui_kit::component::{Selectable as _, Sizable as _};
use gpui_kit::*;

use super::types::Combinator;

pub(super) fn match_mode(
    id: impl Into<ElementId>,
    current: Combinator,
    on_change: impl Fn(Combinator, &mut Window, &mut App) + 'static,
) -> ButtonGroup {
    let on_change = Rc::new(on_change);
    // Child callbacks also receive keyboard activation in the published ButtonGroup.
    ButtonGroup::new(id).small().children(
        [
            (Combinator::And, "All", "Match every condition · $and"),
            (Combinator::Or, "Any", "Match at least one condition · $or"),
        ]
        .into_iter()
        .map(|(mode, label, tooltip)| {
            let on_change = on_change.clone();
            Button::new(label)
                .label(label)
                .selected(current == mode)
                .tooltip(tooltip)
                .on_click(move |_, window, cx| on_change(mode, window, cx))
        }),
    )
}

pub(super) fn toolbar() -> Div {
    div().flex().flex_wrap().items_center().gap(crate::theme::spacing::sm()).min_w(px(0.))
}

#[cfg(test)]
#[path = "controls_tests.rs"]
mod tests;
