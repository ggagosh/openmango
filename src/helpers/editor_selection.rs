use gpui_kit::base::actions::{SelectLeft, SelectRight};
use gpui_kit::base::input::{InputBaseState, InputModeKind};
use gpui_kit::*;

/// Selects from `anchor` to `head`, leaving the caret at `head` even when it comes first.
///
/// ponytail: gpui-base 0.7.3's `set_selected_range` turns a backwards range into a caret
/// (`normalize_token_range` tests `Range::is_empty`, which is true for `end < start`), and no other
/// setter keeps a direction. The editor's own Select Left/Right do, one character per action.
/// Go back to `set_selected_range(anchor..head)` once upstream accepts backwards ranges.
pub fn select_range<M: InputModeKind>(
    input: &mut InputBaseState<M>,
    anchor: usize,
    head: usize,
    window: &mut Window,
    cx: &mut Context<InputBaseState<M>>,
) {
    // Actions reach whatever is focused, so an unfocused input keeps the text selected, forwards.
    if head >= anchor || !input.focus_handle(cx).is_focused(window) {
        input.set_selected_range(anchor.min(head)..anchor.max(head), cx);
        return;
    }
    let selected = input.selected_range();
    let current_anchor =
        if input.cursor() == selected.start { selected.end } else { selected.start };
    // Growing a selection a word at a time moves the caret from where it is, not from the anchor.
    if current_anchor != anchor {
        input.set_selected_range(anchor..anchor, cx);
    }
    let from = input.cursor();
    let (range, action): (_, Box<dyn Action>) = if head < from {
        (head..from, Box::new(SelectLeft))
    } else {
        (from..head, Box::new(SelectRight))
    };
    let skipped = input.text().slice(range).to_string();
    // The caret steps over a CRLF at once.
    for _ in 0..skipped.chars().count() - skipped.matches("\r\n").count() {
        window.dispatch_action(action.boxed_clone(), cx);
    }
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
            cx.update(|window, cx| {
                editor.update(cx, |input, cx| super::select_range(input, anchor, head, window, cx))
            });
            cx.run_until_parked();
            let (range, cursor) =
                editor.read_with(cx, |input, _| (input.selected_range(), input.cursor()));
            assert_eq!((range, cursor), (anchor.min(head)..anchor.max(head), head));
        }
    }
}
