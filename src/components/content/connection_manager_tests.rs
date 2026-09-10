use std::sync::Arc;

use gpui_kit::component::input::AnyInputState;
use gpui_kit::component::{Root, WindowExt as _};
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};

use super::ContentArea;
use crate::components::ConnectionManager;
use crate::models::SavedConnection;
use crate::state::{AppState, ConfigManager};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn connection_open_requests_create_reuse_and_protect_the_manager(cx: &mut TestAppContext) {
    let config = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
    });
    let saved = SavedConnection::new("Saved connection".into(), "mongodb://localhost:27017".into());
    let saved_id = saved.id;
    let state = cx.new(|_| {
        let mut state = AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(config.path().to_owned()),
        );
        state.connections.push(saved);
        state
    });
    let mut content = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| ContentArea::new(state.clone(), cx));
        content = Some(view.clone());
        // This is the shared action invoked by the sidebar '+' and New Connection command.
        ConnectionManager::open_new(state.clone(), window, cx);
        Root::new(view, window, cx).bordered(false)
    });
    draw(cx);
    let content = content.unwrap();
    let manager =
        content.read_with(cx, |content, _| content.connection_manager_view.clone().unwrap());
    assert!(!manager.read_with(cx, |manager, cx| manager.has_unsaved_changes(cx)));

    // Exercise the existing-manager branch as well as construction.
    cx.update(|window, cx| ConnectionManager::open_selected(state.clone(), saved_id, window, cx));
    draw(cx);
    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
    cx.update(|window, cx| ConnectionManager::open_new(state.clone(), window, cx));
    draw(cx);
    assert_eq!(
        content.read_with(cx, |content, _| content
            .connection_manager_view
            .as_ref()
            .unwrap()
            .entity_id()),
        manager.entity_id()
    );
    let name = cx.update(|window, cx| {
        match window.focused_input(cx).expect("new connection name is focused") {
            AnyInputState::Input(input) => input,
            _ => panic!("expected connection name input"),
        }
    });
    cx.update(|window, cx| {
        name.update(cx, |input, cx| input.set_value("Unsaved draft", window, cx))
    });
    draw(cx);
    cx.update(|window, cx| ConnectionManager::open_new(state.clone(), window, cx));
    draw(cx);
    assert!(
        cx.update(|window, cx| window.has_active_dialog(cx)),
        "opening a new connection must protect the current draft"
    );
    cx.update(|window, cx| window.close_dialog(cx));
    draw(cx);
    assert_eq!(name.read_with(cx, |input, _| input.value().to_string()), "Unsaved draft");
    assert!(manager.read_with(cx, |manager, cx| manager.has_unsaved_changes(cx)));
}
