use gpui_kit::*;

use crate::state::AppState;
use crate::theme::islands;

pub(crate) fn render_shell(
    state: Entity<AppState>,
    content: impl IntoElement,
    with_background: bool,
    cx: &App,
) -> AnyElement {
    let mut root = div().flex().flex_col().flex_1().h_full().min_h(px(0.0)).min_w(px(0.0));
    if with_background {
        let appearance = state.read(cx).settings.appearance.clone();
        root = root.bg(islands::content_bg(&appearance, cx));
    }
    root.child(content).into_any_element()
}
