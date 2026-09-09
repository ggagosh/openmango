use gpui_kit::component::{Root, Theme};
use gpui_kit::{
    AppContext as _, Context, InteractiveElement as _, IntoElement, KeyDownEvent, KeyUpEvent,
    Keystroke, Modifiers, ParentElement as _, Render, Styled as _, TestAppContext, Window, div,
    point, px,
};

use super::match_mode;
use crate::components::filter_builder::types::Combinator;

struct Modes {
    first: Combinator,
    second: Combinator,
}

impl Render for Modes {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let first = cx.entity();
        let second = cx.entity();
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap_2()
            .child(div().debug_selector(|| "first-mode".into()).child(match_mode(
                "first",
                self.first,
                move |mode, _, cx| {
                    first.update(cx, |this, cx| {
                        this.first = mode;
                        cx.notify();
                    });
                },
            )))
            .child(div().debug_selector(|| "second-mode".into()).child(match_mode(
                "second",
                self.second,
                move |mode, _, cx| {
                    second.update(cx, |this, cx| {
                        this.second = mode;
                        cx.notify();
                    });
                },
            )))
    }
}

#[gpui_kit::test]
fn group_modes_support_keyboard_and_independent_pointer_selection(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::global_mut(cx).font_family = crate::theme::fonts::ui().into();
        Theme::global_mut(cx).font_size = px(16.);
    });
    let mut modes = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|_| Modes { first: Combinator::Or, second: Combinator::And });
        modes = Some(view.clone());
        Root::new(view, window, cx).bordered(false)
    });
    let modes = modes.unwrap();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.blur(cx);
        window.focus_next(cx);
    });
    let key = Keystroke::parse("enter").unwrap();
    cx.simulate_event(KeyDownEvent {
        keystroke: key.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent { keystroke: key });
    cx.run_until_parked();
    assert_eq!(modes.read_with(cx, |view, _| view.first), Combinator::And);
    let second = cx.debug_bounds("second-mode").unwrap();
    cx.simulate_click(
        point(second.left() + second.size.width * 0.75, second.center().y),
        Modifiers::default(),
    );
    assert_eq!(modes.read_with(cx, |view, _| view.second), Combinator::Or);
    assert_eq!(modes.read_with(cx, |view, _| view.first), Combinator::And);
}
