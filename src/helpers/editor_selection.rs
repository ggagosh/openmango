use gpui_kit::Context;
use gpui_kit::base::input::{InputBaseState, InputModeKind};

/// Selects from `anchor` to `head`, leaving the caret at `head` even when it comes first.
pub fn select_range<M: InputModeKind>(
    input: &mut InputBaseState<M>,
    anchor: usize,
    head: usize,
    cx: &mut Context<InputBaseState<M>>,
) {
    input.set_selected_range_with_direction(anchor.min(head)..anchor.max(head), head < anchor, cx);
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::Root;
    use gpui_kit::component::input::{Editor, EditorState};
    use gpui_kit::{
        AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _, Render,
        Styled as _, TestAppContext, Window, div,
    };

    struct Host(Entity<EditorState>);

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(Editor::new(&self.0))
        }
    }

    #[gpui_kit::test]
    fn backwards_selections_keep_the_caret_at_their_start(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut editor = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let state = cx.new(|cx| EditorState::new(window, cx).default_value("db.orders.find()"));
            editor = Some(state.clone());
            Root::new(cx.new(|_| Host(state)), window, cx)
        });
        let editor = editor.unwrap();
        cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
        cx.run_until_parked();

        for (anchor, head) in [(16, 10), (16, 3), (16, 12), (3, 9)] {
            cx.update(|_, cx| {
                editor.update(cx, |input, cx| super::select_range(input, anchor, head, cx))
            });
            cx.run_until_parked();
            let (range, cursor) =
                editor.read_with(cx, |input, _| (input.selected_range(), input.cursor()));
            assert_eq!((range, cursor), (anchor.min(head)..anchor.max(head), head));
        }
    }
}
