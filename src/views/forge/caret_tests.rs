use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::component::input::{EditorState, InputEvent};
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};

use super::ForgeView;
use crate::state::{AppState, ConfigManager};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn opening_forge_starts_the_editor_caret(cx: &mut TestAppContext) {
    let config = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
    });
    // The kit starts the caret blinking where it emits `InputEvent::Focus`; the editor appears
    // during the same render that focuses it, so watch every editor from its creation.
    let focused = Rc::new(Cell::new(false));
    cx.update(|cx| {
        let focused = focused.clone();
        cx.observe_new::<EditorState>(move |_, _, cx| {
            let focused = focused.clone();
            cx.subscribe_self(move |_, event: &InputEvent, _| {
                focused.set(focused.get() || matches!(event, InputEvent::Focus));
            })
            .detach();
        })
        .detach();
    });
    let state = cx.new(|_| {
        AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(config.path().to_owned()),
        )
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        Root::new(cx.new(|cx| ForgeView::new(state.clone(), cx)), window, cx)
    });
    // An inactive window reports no focus changes at all.
    cx.update(|window, _| window.activate_window());
    draw(cx);

    cx.update(|_, cx| {
        state.update(cx, |state, cx| {
            state.open_forge_tab(uuid::Uuid::new_v4(), "db".into(), None, cx)
        })
    });
    draw(cx);
    assert!(focused.get());
}
