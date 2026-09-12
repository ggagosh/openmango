use gpui_kit::ClipboardItem;
use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::component::list::ListDelegate as _;
use gpui_kit::{
    AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext, px, size,
};

use super::{ConnectionManager, ManagerTab, TestStatus};
use crate::models::{ActiveConnection, SavedConnection};
use crate::state::{AppState, ConfigManager};

fn setup(
    cx: &mut TestAppContext,
    config: ConfigManager,
    connections: Vec<SavedConnection>,
) -> (Entity<AppState>, Entity<ConnectionManager>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
    });
    let state = cx.new(|_| {
        let mut state =
            AppState::with_config(Arc::new(crate::connection::ConnectionManager::new()), config);
        state.connections = connections;
        state
    });
    let mut manager = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| ConnectionManager::new(state.clone(), None, window, cx));
        manager = Some(view.clone());
        Root::new(view, window, cx).bordered(false)
    });
    draw(cx);
    (state, manager.unwrap(), cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

fn paste_uri(manager: &Entity<ConnectionManager>, uri: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.focus(&manager.read(cx).draft.uri_state.read(cx).focus_handle(cx), cx);
        cx.write_to_clipboard(ClipboardItem::new_string(uri.to_string()));
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-a cmd-v" } else { "ctrl-a ctrl-v" });
    draw(cx);
}

#[gpui_kit::test]
fn connection_editor_preserves_credentials_and_tls_alias_while_editing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (_, manager, cx) = setup(cx, ConfigManager::with_config_dir(dir.path().into()), vec![]);
    paste_uri(&manager, "mongodb://user:secret@", cx);
    manager.read_with(cx, |view, cx| {
        assert!(view.parse_error.is_some());
        assert!(!view.draft.uri_state.read(cx).value().contains("secret"));
        assert_eq!(view.draft.password_state.read(cx).value().as_ref(), "secret");
    });
    paste_uri(
        &manager,
        "mongodb://?authMechanism=MONGODB-AWS&authMechanismProperties=AWS_SESSION_TOKEN:temporary-token",
        cx,
    );
    manager.read_with(cx, |view, cx| {
        assert!(view.parse_error.is_some());
        assert!(!view.draft.uri_state.read(cx).value().contains("temporary-token"));
        assert_eq!(view.draft.uri_secrets.aws_session_token.as_deref(), Some("temporary-token"));
    });
    paste_uri(&manager, "mongodb+srv://%20user%20:p%40ss@cluster.example/?ssl=false", cx);
    cx.update(|window, cx| {
        manager.update(cx, |view, cx| {
            assert!(!view.draft.tls);
            assert!(view.update_uri_from_fields(window, cx));
            let uri = view.real_uri(cx);
            assert!(uri.contains("%20user%20:p%40ss@"));
            assert!(uri.contains("ssl=false"));
            assert!(!uri.contains("tls="));
            view.draft.tls = true;
            assert!(view.update_uri_from_fields(window, cx));
            assert!(view.real_uri(cx).contains("ssl=true"));
        })
    });
}

#[gpui_kit::test]
fn connection_editor_pastes_options_and_saves_without_connecting(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (state, manager, cx) = setup(cx, ConfigManager::with_config_dir(dir.path().into()), vec![]);
    cx.update(|window, cx| {
        window.focus(&manager.read(cx).draft.uri_state.read(cx).focus_handle(cx), cx)
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" });
    cx.simulate_input("mongodb+srv://user:p%40ss@cluster.example/app?authSource=admin&appName=Imported%20App&retryWrites=false&tls=false");
    draw(cx);
    manager.read_with(cx, |view, cx| {
        assert_eq!(view.draft.password_state.read(cx).value().as_ref(), "p@ss");
        assert_eq!(
            view.draft.app_name_state.read(cx).value().as_ref(),
            "Imported App",
            "URI after paste: {}",
            view.draft.uri_state.read(cx).value()
        );
        assert_eq!(view.draft.auth_source_state.read(cx).value().as_ref(), "admin");
        assert!(!view.draft.tls);
        assert!(!view.draft.uri_state.read(cx).value().contains("p%40ss"));
    });
    cx.update(|window, cx| {
        manager.update(cx, |view, cx| {
            view.draft
                .password_state
                .update(cx, |input, cx| input.set_value(" p@ss:/?& ", window, cx));
            assert!(view.save_connection(false, window, cx).is_some());
            assert!(view.pending_save.is_some());
            assert!(view.has_unsaved_changes(cx));
        })
    });
    draw(cx);
    state.read_with(cx, |state, _| {
        assert_eq!(state.connections.len(), 1);
        let saved = &state.connections[0];
        assert!(!state.is_connected(saved.id));
        assert!(saved.uri.contains("retryWrites=false"));
        assert!(saved.uri.contains("appName=Imported%20App"));
        assert!(saved.uri.contains("tls=false"));
        assert!(saved.uri.contains("%20p%40ss%3A%2F%3F%26%20"));
    });
    manager.read_with(cx, |view, cx| {
        assert!(view.pending_save.is_none());
        assert!(!view.has_unsaved_changes(cx));
        assert!(!view.creating_new);
    });
    let saved = std::fs::read_to_string(dir.path().join("connections.json")).unwrap();
    assert!(!saved.contains("p%40ss"));
}

#[gpui_kit::test]
fn connection_editor_failed_save_keeps_draft_and_active_session(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut saved = SavedConnection::new("Existing".into(), "mongodb://localhost:27017".into());
    saved.secret_id = Some(uuid::Uuid::new_v4());
    let (state, manager, cx) =
        setup(cx, ConfigManager::with_config_dir(dir.path().into()), vec![saved.clone()]);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = runtime.block_on(async {
        mongodb::Client::with_options(mongodb::options::ClientOptions::default()).unwrap()
    });
    state.update(cx, |state, _| {
        state.insert_active_connection(
            saved.id,
            ActiveConnection {
                config: saved.clone(),
                client,
                databases: vec!["keep_this_session".into()],
                collections: Default::default(),
                runtime_meta: Default::default(),
            },
        )
    });
    // Make the final atomic rename fail without changing the test process permissions.
    std::fs::create_dir(dir.path().join("connections.json")).unwrap();
    cx.update(|window, cx| {
        manager.update(cx, |view, cx| {
            view.draft
                .uri_state
                .update(cx, |input, cx| input.set_value("mongodb://different.example", window, cx));
        })
    });
    draw(cx);
    cx.update(|window, cx| {
        manager.update(cx, |view, cx| {
            assert!(view.save_connection(true, window, cx).is_some());
        })
    });
    draw(cx);
    manager.read_with(cx, |view, cx| {
        assert!(view.pending_save.is_none());
        assert!(view.has_unsaved_changes(cx));
        assert!(view.draft.uri_state.read(cx).value().contains("different.example"));
        assert!(matches!(view.status, TestStatus::Error(_)));
    });
    state.read_with(cx, |state, _| {
        assert_eq!(state.connections[0].uri, saved.uri);
        let active = state.active_connection_by_id(saved.id).unwrap();
        assert_eq!(active.config.uri, saved.uri);
        assert_eq!(active.databases, ["keep_this_session"]);
    });
    std::fs::remove_dir(dir.path().join("connections.json")).unwrap();
    cx.update(|window, cx| {
        manager.update(cx, |view, cx| {
            assert!(view.save_connection(false, window, cx).is_some());
        })
    });
    draw(cx);
    state.read_with(cx, |state, _| {
        assert!(state.connections[0].uri.contains("different.example"));
        assert_eq!(state.active_connection_by_id(saved.id).unwrap().config.uri, saved.uri);
        assert!(state.connection_needs_reconnect(saved.id));
    });
}

#[gpui_kit::test]
fn connection_editor_discards_stale_test_results_and_renders_every_tab(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (_, manager, cx) = setup(cx, ConfigManager::with_config_dir(dir.path().into()), vec![]);
    cx.update(|window, cx| {
        manager.update(cx, |view, cx| {
            view.test_generation = 4;
            view.status = TestStatus::Testing;
            view.pending_test_fingerprint = Some(view.draft.fingerprint(cx));
            view.draft
                .ssh_password_state
                .update(cx, |input, cx| input.set_value("changed", window, cx));
            view.finish_test(4, Ok(()), cx);
            assert!(matches!(view.status, TestStatus::Idle));
            view.pending_test_fingerprint = Some(view.draft.fingerprint(cx));
            view.status = TestStatus::Testing;
            view.finish_test(3, Ok(()), cx);
            assert!(matches!(view.status, TestStatus::Testing));
            view.finish_test(4, Ok(()), cx);
            assert!(matches!(view.status, TestStatus::Success));
        })
    });
    for tab in ManagerTab::all() {
        manager.update(cx, |view, cx| {
            view.active_tab = tab;
            cx.notify();
        });
        draw(cx);
    }
    cx.simulate_resize(size(px(640.), px(480.)));
    for tab in ManagerTab::all() {
        manager.update(cx, |view, cx| {
            view.active_tab = tab;
            cx.notify();
        });
        draw(cx);
    }
}

#[gpui_kit::test]
fn connection_editor_native_search_confirms_the_matching_connection(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let first = SavedConnection::new("Alpha".into(), "mongodb://alpha.example".into());
    let second = SavedConnection::new("Beta".into(), "mongodb://beta.example".into());
    let second_id = second.id;
    let (_, manager, cx) =
        setup(cx, ConfigManager::with_config_dir(dir.path().into()), vec![first, second]);
    let list = manager.read_with(cx, |view, _| view.connection_list.clone());
    cx.update(|window, cx| {
        list.update(cx, |list, cx| {
            list.set_query("beta", window, cx);
            assert_eq!(list.delegate().items_count(0, cx), 1);
        });
        window.focus(&list.read(cx).focus_handle(cx), cx);
    });
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(manager.read_with(cx, |view, _| view.selected_id), Some(second_id));
}
