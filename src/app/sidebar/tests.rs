use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::{AppContext as _, Entity, ScrollStrategy, TestAppContext, VisualTestContext};
use gpui_kit::{px, size};

use super::Sidebar;
use crate::models::{ActiveConnection, SavedConnection, TreeNodeId};
use crate::state::{AppState, ConfigManager};

/// A sidebar over one open connection: database `a` with 60 collections, `b` with one.
/// Every database has its collections already, so expanding never reaches for the network.
fn setup(cx: &mut TestAppContext) -> (Entity<Sidebar>, uuid::Uuid, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
    });
    let dir = tempfile::tempdir().unwrap();
    let saved = SavedConnection::new("Local".into(), "mongodb://localhost:27017".into());
    let id = saved.id;
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = runtime.block_on(async {
        mongodb::Client::with_options(mongodb::options::ClientOptions::default()).unwrap()
    });
    let state = cx.new(|_| {
        let mut state = AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(dir.path().into()),
        );
        state.connections = vec![saved.clone()];
        state.insert_active_connection(
            id,
            ActiveConnection {
                config: saved,
                client,
                databases: vec!["a".into(), "b".into()],
                collections: HashMap::from([
                    ("a".to_string(), (0..60).map(|n| format!("col_{n:02}")).collect()),
                    ("b".to_string(), vec!["only".to_string()]),
                ]),
                runtime_meta: Default::default(),
            },
        );
        state
    });
    let mut sidebar = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Sidebar::new(state.clone(), window, cx));
        sidebar = Some(view.clone());
        Root::new(view, window, cx).bordered(false)
    });
    cx.simulate_resize(size(px(300.0), px(400.0)));
    draw(cx);
    (sidebar.unwrap(), id, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

fn expand(sidebar: &Entity<Sidebar>, node: &TreeNodeId, open: bool, cx: &mut VisualTestContext) {
    sidebar.update(cx, |sidebar, cx| sidebar.set_node_expanded(node, open, false, cx));
    draw(cx);
}

#[gpui_kit::test]
fn collapsed_database_stays_closed_when_a_sibling_opens(cx: &mut TestAppContext) {
    let (sidebar, id, cx) = setup(cx);
    let (conn, a, b) =
        (TreeNodeId::connection(id), TreeNodeId::database(id, "a"), TreeNodeId::database(id, "b"));
    expand(&sidebar, &conn, true, cx);
    expand(&sidebar, &a, true, cx);
    sidebar.update(cx, |sidebar, cx| {
        assert_eq!(sidebar.model.entries.len(), 63);
        sidebar.select_sidebar_node(TreeNodeId::collection(id, "a", "col_05"), false, cx);
    });

    expand(&sidebar, &a, false, cx);
    expand(&sidebar, &b, true, cx);

    sidebar.read_with(cx, |sidebar, _| {
        assert_eq!(sidebar.model.selected_tree_id, Some(a.clone()), "selection moves up");
        assert!(!sidebar.model.expanded_nodes.contains(&a), "the closed database reopened");
        assert_eq!(sidebar.model.entries.len(), 4);
    });
}

#[gpui_kit::test]
fn scrolled_rows_pin_their_connection_and_database(cx: &mut TestAppContext) {
    let (sidebar, id, cx) = setup(cx);
    expand(&sidebar, &TreeNodeId::connection(id), true, cx);
    expand(&sidebar, &TreeNodeId::database(id, "a"), true, cx);
    sidebar.read_with(cx, |sidebar, _| assert!(sidebar.sticky_rows().is_empty()));

    // Five rows down is far less than one viewport: arithmetic done in viewports instead of
    // rows would still see row 0 on top and pin nothing.
    sidebar.update(cx, |sidebar, cx| {
        sidebar.scroll_handle.scroll_to_item_strict(5, ScrollStrategy::Top);
        cx.notify();
    });
    draw(cx);

    sidebar.read_with(cx, |sidebar, _| {
        assert_eq!(sidebar.sticky_rows(), vec![0, 1]);
        let offset = sidebar.scroll_handle.0.borrow().base_handle.offset().y;
        assert_eq!(offset, -super::ROW_HEIGHT * 5.0, "rows are not ROW_HEIGHT tall");
        assert!(sidebar.page_size() > 5, "a page is {} rows", sidebar.page_size());
    });
}

#[gpui_kit::test]
fn arrow_keys_walk_the_tree(cx: &mut TestAppContext) {
    let (sidebar, id, cx) = setup(cx);
    let a = TreeNodeId::database(id, "a");
    expand(&sidebar, &TreeNodeId::connection(id), true, cx);
    let focus_handle = sidebar.read_with(cx, |sidebar, _| sidebar.focus_handle.clone());
    cx.update(|window, cx| window.focus(&focus_handle, cx));

    // Down from nothing lands on the first row, then the database; Right opens it, Right
    // again steps into it, Left steps back out, Left again closes it.
    cx.simulate_keystrokes("down down right");
    sidebar.read_with(cx, |sidebar, _| {
        assert_eq!(sidebar.model.selected_tree_id, Some(a.clone()));
        assert!(sidebar.model.expanded_nodes.contains(&a));
    });
    cx.simulate_keystrokes("right");
    sidebar.read_with(cx, |sidebar, _| {
        assert_eq!(sidebar.model.selected_tree_id, Some(TreeNodeId::collection(id, "a", "col_00")));
    });
    cx.simulate_keystrokes("left");
    sidebar.read_with(cx, |sidebar, _| assert_eq!(sidebar.model.selected_tree_id, Some(a.clone())));
    cx.simulate_keystrokes("left");
    sidebar.read_with(cx, |sidebar, _| assert!(!sidebar.model.expanded_nodes.contains(&a)));

    // End stops at the last row and a further Down stays there rather than wrapping.
    cx.simulate_keystrokes("end down");
    sidebar.read_with(cx, |sidebar, _| {
        assert_eq!(sidebar.model.selected_tree_id, Some(TreeNodeId::database(id, "b")));
    });
}
