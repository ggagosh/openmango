use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::prelude::*;
use gpui_kit::{TestAppContext, VisualTestContext, div, px, size};
use mongodb::bson::doc;

use super::CollectionView;
use crate::bson::DocumentKey;
use crate::models::SavedConnection;
use crate::state::{
    AppState, ConfigManager, DocumentViewMode, ExplainOpenMode, SessionDocument, SessionKey,
};

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

/// The documents view on its own, pointed at a session of a saved connection.
fn mount<'a>(
    cx: &'a mut TestAppContext,
    config: &std::path::Path,
    prepare: impl FnOnce(&mut AppState, &SessionKey),
) -> (gpui_kit::Entity<AppState>, SessionKey, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
        crate::keyboard::bind_keymap(cx, &Default::default());
    });
    let saved = SavedConnection::new("Local".into(), "mongodb://localhost".into());
    let session = SessionKey::new(saved.id, "shop", "orders");
    // No live connection: a real client would start loading documents on a tokio thread, which
    // the deterministic test scheduler rejects. The view only needs a current session.
    let state = cx.new(|_| {
        let mut state = AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(config.into()),
        );
        state.connections = vec![saved];
        state.ensure_session(session.clone());
        prepare(&mut state, &session);
        state
    });

    // Mounting it through the content area needs an open connection, which would start real
    // loads.
    let mut view = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let documents = cx.new(|cx| CollectionView::new(state.clone(), cx));
        view = Some(documents.clone());
        let host = cx.new(|_| Host(documents));
        Root::new(host, window, cx)
    });
    view.unwrap().update(cx, |view, cx| {
        let state = view.state.clone();
        view.view_model.set_current_session(Some(session.clone()), &state, cx);
    });
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    draw(cx);
    draw(cx);
    (state, session, cx)
}

/// Escape is bound to `CloseSearch` across the whole Documents context, so the Explain modal's
/// own key handler never sees it. The action has to close the modal itself.
#[gpui_kit::test]
fn escape_closes_the_explain_modal(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (state, session, cx) = mount(cx, dir.path(), |state, session| {
        state.set_explain_open_mode(session, ExplainOpenMode::Modal);
    });

    let mode = |cx: &mut VisualTestContext| {
        state.read_with(cx, |state, _| state.session(&session).unwrap().data.explain.open_mode)
    };
    assert!(matches!(mode(cx), ExplainOpenMode::Modal), "the modal should start open");

    cx.simulate_keystrokes("escape");
    draw(cx);

    assert!(matches!(mode(cx), ExplainOpenMode::Closed), "Escape left the Explain modal open");
}

/// Escape leaves the JSON editor for the view it was opened from, unless it has edits to save.
#[gpui_kit::test]
fn escape_closes_the_json_editor_only_when_saved(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (state, session, cx) = mount(cx, dir.path(), |state, session| {
        let document = doc! { "_id": 1, "status": "open" };
        let data = &mut state.session_mut(session).unwrap().data;
        let key = DocumentKey::from_document(&document, 0);
        data.index_by_key.insert(key.clone(), 0);
        data.items = vec![SessionDocument { key, doc: document }];
        data.total = 1;
        data.loaded = true;
        state.set_view_mode(session, DocumentViewMode::Table);
        state.set_view_mode(session, DocumentViewMode::Json);
    });
    let mode = |cx: &mut VisualTestContext| {
        state.read_with(cx, |state, _| state.session_view_mode(&session))
    };
    let sessions = state.read_with(cx, |state, _| state.editor_sessions());
    let id = sessions.find_any_document_session().expect("the JSON editor should be open");
    let saved = sessions.snapshot(id).unwrap().content;
    // The editor takes focus as it opens, so Escape goes through it first.
    assert!(cx.update(|window, cx| window.focused(cx).is_some()));

    sessions.update_content(id, "{}".into());
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(mode(cx), DocumentViewMode::Json, "Escape dropped unsaved JSON edits");

    sessions.update_content(id, saved);
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(mode(cx), DocumentViewMode::Table, "Escape should return to the table");
}
