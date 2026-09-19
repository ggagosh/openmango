use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::prelude::*;
use gpui_kit::{TestAppContext, VisualTestContext, div, px, size};

use crate::components::ContentArea;
use crate::state::{AppState, ConfigManager};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

struct Host(gpui_kit::Entity<ContentArea>);
impl gpui_kit::Render for Host {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl IntoElement {
        // The way the app shell mounts the content area (see `AppRoot::render`).
        div().flex().flex_row().size_full().child(
            div()
                .flex()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .overflow_hidden()
                .child(self.0.clone()),
        )
    }
}

/// The form sits in a `flex_1` + `min_h(0)` scroll region. If any ancestor stops passing a
/// definite height down, that region collapses to nothing and the page looks empty.
#[gpui_kit::test]
fn the_transfer_form_is_visible(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
    });
    let dir = tempfile::tempdir().unwrap();
    let state = cx.new(|_| {
        AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(dir.path().into()),
        )
    });
    state.update(cx, |state, cx| state.open_transfer_tab(cx));
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| ContentArea::new(state.clone(), cx));
        let host = cx.new(|_| Host(view));
        Root::new(host, window, cx).bordered(false)
    });
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    draw(cx);
    draw(cx);

    // The card keeps its natural height even when the scroll region around it has collapsed and
    // clips it, so the card's own size proves nothing. Where the footer lands does: below the
    // form when the region has height, on top of it when it has none.
    for (w, h) in [(1200.0, 800.0), (2000.0, 1300.0), (900.0, 500.0)] {
        cx.simulate_resize(size(px(w), px(h)));
        draw(cx);
        let form = cx.debug_bounds("transfer-form").expect("the form was not drawn at all");
        let footer = cx.debug_bounds("transfer-footer").expect("no footer");
        eprintln!(
            "WINDOW {w}x{h}: form {:?}..{:?}  footer top {:?}",
            form.top(),
            form.bottom(),
            footer.top()
        );
        assert!(footer.top() >= form.bottom(), "the form's scroll region collapsed at {w}x{h}");
    }
}
