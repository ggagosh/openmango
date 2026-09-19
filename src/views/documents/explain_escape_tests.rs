use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::prelude::*;
use gpui_kit::{TestAppContext, VisualTestContext, div, px, size};

use super::CollectionView;
use crate::models::SavedConnection;
use crate::state::{AppState, ConfigManager, ExplainOpenMode, SessionKey};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

struct Host(gpui_kit::Entity<CollectionView>);
impl gpui_kit::Render for Host {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl IntoElement {
        div().flex().size_full().child(self.0.clone())
    }
}

/// Escape is bound to `CloseSearch` across the whole Documents context, so the Explain modal's
/// own key handler never sees it. The action has to close the modal itself.
#[gpui_kit::test]
fn escape_closes_the_explain_modal(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
        crate::keyboard::bind_keymap(cx, &Default::default());
    });
    let dir = tempfile::tempdir().unwrap();
    let saved = SavedConnection::new("Local".into(), "mongodb://localhost".into());
    let id = saved.id;
    // No live connection: a real client would start loading documents on a tokio thread, which
    // the deterministic test scheduler rejects. The view only needs a current session.
    let state = cx.new(|_| {
        let mut state = AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(dir.path().into()),
        );
        state.connections = vec![saved.clone()];
        state
    });
    let session = SessionKey::new(id, "shop", "orders");
    state.update(cx, |state, _| {
        state.ensure_session(session.clone());
        state.set_explain_open_mode(&session, ExplainOpenMode::Modal);
    });

    // The documents view on its own, pointed at the session: mounting it through the content
    // area needs an open connection, which would start real loads.
    let mut view = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let documents = cx.new(|cx| CollectionView::new(state.clone(), cx));
        view = Some(documents.clone());
        let host = cx.new(|_| Host(documents));
        Root::new(host, window, cx).bordered(false)
    });
    let view = view.unwrap();
    view.update(cx, |view, cx| {
        let state = view.state.clone();
        view.view_model.set_current_session(Some(session.clone()), &state, cx);
    });
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    draw(cx);
    draw(cx);

    let mode = |cx: &mut VisualTestContext| {
        state.read_with(cx, |state, _| state.session(&session).unwrap().data.explain.open_mode)
    };
    assert!(matches!(mode(cx), ExplainOpenMode::Modal), "the modal should start open");

    cx.simulate_keystrokes("escape");
    draw(cx);

    assert!(matches!(mode(cx), ExplainOpenMode::Closed), "Escape left the Explain modal open");
}
