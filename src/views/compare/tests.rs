use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};
use mongodb::bson::doc;

use super::detail::{DetailRow, detail_rows};
use crate::components::ContentArea;
use crate::state::compare::{CompareConfig, CompareDetail, CompareEndpoint, CompareTabState};
use crate::state::{AppState, ConfigManager};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn compare_sync_checkboxes_toggle_rows_and_category_mixed_state(cx: &mut TestAppContext) {
    use crate::connection::ops::compare::{CompareSummary, DiffKind, DiffRow, Side};
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let state = cx.new(|_| {
        AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(directory.path().into()),
        )
    });
    let id = state.update(cx, |state, cx| {
        state.open_compare_tab(None, cx);
        let id = state.active_compare_tab_id().unwrap();
        let tab = state.compare_tab_mut(id).unwrap();
        tab.compared = Some(tab.config.clone());
        tab.summary = Some(CompareSummary {
            counts: Default::default(),
            skipped: Some([0, 0]),
            truncated: false,
            cancelled: false,
            elapsed: Default::default(),
        });
        for key in 0..2 {
            tab.rows.push(DiffRow {
                key: key.into(),
                left_id: None,
                right_id: None,
                kind: DiffKind::OnlyLeft,
                changed: 0,
                paths: "".into(),
                left_hash: 1,
                right_hash: 0,
                left_count: 1,
                right_count: 0,
            });
            tab.segments[0].push(key as usize);
            tab.segments[1].push(key as usize);
        }
        tab.sync.set_target(Side::Right);
        id
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| ContentArea::new(state.clone(), cx));
        Root::new(view, window, cx).bordered(false)
    });
    cx.simulate_resize(size(px(1200.0), px(900.0)));
    draw(cx);
    draw(cx);
    let row = cx
        .update(|window, _| gpui_kit::base::test_support::snapshots(window))
        .into_iter()
        .find(|node| node.path().last() == Some(&gpui_kit::ElementId::from(("sync-row", 0usize))))
        .unwrap();
    cx.simulate_click(row.bounds().center(), Default::default());
    draw(cx);
    state.read_with(cx, |state, _| {
        assert_eq!(state.compare_tab(id).unwrap().sync.categories[0].count(2), 1)
    });
    let category = cx
        .update(|window, _| gpui_kit::base::test_support::snapshots(window))
        .into_iter()
        .find(|node| {
            node.path().last() == Some(&gpui_kit::ElementId::from(("sync-category", 0usize)))
        })
        .unwrap();
    cx.simulate_click(category.bounds().center(), Default::default());
    draw(cx);
    state.read_with(cx, |state, _| {
        assert_eq!(state.compare_tab(id).unwrap().sync.categories[0].count(2), 2)
    });
    for width in [700.0, 430.0] {
        cx.simulate_resize(size(px(width), px(1000.0)));
        draw(cx);
    }
}

#[gpui_kit::test]
fn compare_detail_columns_align_with_headers_for_one_sided_documents(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let state = cx.new(|_| {
        AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(directory.path().into()),
        )
    });
    state.update(cx, |state, cx| {
        state.open_compare_tab(None, cx);
        let id = state.active_compare_tab_id().unwrap();
        let tab = state.compare_tab_mut(id).unwrap();
        tab.config.fields = vec!["logId".into()];
        tab.compared = Some(tab.config.clone());
        tab.rows.push(crate::connection::ops::compare::DiffRow { key: "record-1".into(), left_id: Some(1.into()), right_id: None,
            kind: crate::connection::ops::compare::DiffKind::OnlyLeft, changed: 0, paths: "".into(), left_hash: 1, right_hash: 0, left_count: 1, right_count: 0 });
        tab.segments[0].push(0); tab.segments[1].push(0);
        tab.selected = Some(0); tab.detail_row = Some(0);
        let mut deep = doc! {"value": 42};
        for key in ["e", "d", "c", "b", "a"] { deep = doc! {key: deep}; }
        tab.detail = Some(Arc::new(CompareDetail {
            documents: [vec![doc! {"_id":1,"logId":"record-1", "message":"long value ".repeat(120), "nested":{"value":123}, "deep":deep}], Vec::new()],
            changed_since_scan: false,
        }));
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| ContentArea::new(state.clone(), cx));
        Root::new(view, window, cx).bordered(false)
    });
    for width in [1750.0, 1000.0, 700.0, 430.0] {
        cx.simulate_resize(size(px(width), px(1200.0)));
        draw(cx);
        draw(cx);
        for (cell, heading) in [
            ("compare-value-0", "compare-heading-0"),
            ("compare-value-1", "compare-heading-1"),
            ("compare-value-2", "compare-heading-0"),
            ("compare-value-3", "compare-heading-1"),
            ("compare-value-4", "compare-heading-0"),
            ("compare-value-5", "compare-heading-1"),
            ("compare-value-8", "compare-heading-0"),
            ("compare-value-9", "compare-heading-1"),
            ("compare-value-20", "compare-heading-0"),
            ("compare-value-21", "compare-heading-1"),
        ] {
            let value = cx.debug_bounds(cell).unwrap_or_else(|| panic!("missing {cell}"));
            let title = cx.debug_bounds(heading).unwrap();
            assert!(
                (f32::from(value.left() - title.left())).abs() <= 1.0
                    && (f32::from(value.right() - title.right())).abs() <= 1.0,
                "{cell} is not under {heading} at {width}px: value={value:?}, heading={title:?}"
            );
            assert!(value.size.width > px(40.0), "missing-side cells must retain their column");
        }
        let field = cx.debug_bounds("compare-field-10").expect("deep nested field");
        let button = cx
            .update(|window, _| gpui_kit::base::test_support::snapshots(window))
            .into_iter()
            .find(|node| {
                node.path().last() == Some(&gpui_kit::ElementId::from(("compare-branch", 10usize)))
            })
            .unwrap()
            .bounds();
        assert!(
            button.left() >= field.left()
                && button.right() <= field.right()
                && button.size.width >= px(24.0),
            "deep disclosure escapes its field at {width}px: {button:?}, {field:?}"
        );
    }
}

#[gpui_kit::test]
fn compare_dropdowns_have_control_sized_hitboxes_inside_their_rows(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let state = cx.new(|_| {
        AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(directory.path().into()),
        )
    });
    state.update(cx, |state, cx| {
        state.open_compare_tab(
            Some(CompareEndpoint {
                database: "tenantdevshipmanager".into(),
                collection: "auditlogs".into(),
                ..Default::default()
            }),
            cx,
        );
        let id = state.active_compare_tab_id().unwrap();
        let tab = state.compare_tab_mut(id).unwrap();
        tab.metadata[0] = Some(crate::state::compare::CompareMetadata {
            endpoint: tab.config.sides[0].clone(),
            count: Some(323),
            bytes: Some(158 * 1024),
            indexes: vec![mongodb::IndexModel::builder().keys(doc! {"_id": 1}).build()],
            ..Default::default()
        });
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| ContentArea::new(state.clone(), cx));
        Root::new(view, window, cx).bordered(false)
    });
    for width in [1750.0, 1200.0, 900.0, 430.0] {
        cx.simulate_resize(size(px(width), px(900.0)));
        draw(cx);
        draw(cx);
        let setup = cx.debug_bounds("compare-setup").unwrap();
        let actions = cx.debug_bounds("compare-actions").unwrap();
        let controls = cx.update(|window, _| gpui_kit::base::test_support::snapshots(window));
        let mut picker_bounds = Vec::new();
        for name in [
            "Left connection",
            "Left database",
            "Left collection",
            "Right connection",
            "Right database",
            "Right collection",
        ] {
            let control = controls
                .iter()
                .find(|node| node.label() == Some(name))
                .unwrap_or_else(|| panic!("missing {name}"));
            let bounds = control.bounds();
            assert!(
                bounds.size.height <= px(32.0),
                "{name} steals its parent's height at {width}px: {bounds:?}"
            );
            assert!(
                bounds.bottom() <= actions.top(),
                "{name} overlaps actions at {width}px: {bounds:?}, {actions:?}"
            );
            assert!(
                bounds.left() >= setup.left() && bounds.right() <= setup.right(),
                "{name} overflows setup at {width}px"
            );
            picker_bounds.push(bounds);
        }
        for (i, a) in picker_bounds.iter().enumerate() {
            assert!(
                picker_bounds[i + 1..].iter().all(|b| !a.intersects(b)),
                "dropdown hitboxes overlap at {width}px: {picker_bounds:?}"
            );
        }
        let statistics = cx.debug_bounds("compare-size-0").expect("collection statistics");
        assert!(
            picker_bounds[..3].iter().all(|bounds| bounds.bottom() <= statistics.top()),
            "statistics overlap dropdowns at {width}px"
        );
        assert!(statistics.bottom() <= actions.top(), "statistics overlap actions at {width}px");
        assert!(
            !controls.iter().any(|node| node.label() == Some("Suggested match keys")),
            "key editing belongs in settings, not the main toolbar"
        );
        let compare = controls
            .iter()
            .find(|node| node.path().last() == Some(&"compare-run".into()))
            .unwrap()
            .bounds();
        let settings = controls
            .iter()
            .find(|node| node.path().last() == Some(&"compare-options".into()))
            .unwrap()
            .bounds();
        assert!(compare.top() >= actions.top() && compare.bottom() <= actions.bottom());
        assert!(!compare.intersects(&settings));
        if width >= 1200.0 {
            let left: Vec<_> = ["Left connection", "Left database", "Left collection"]
                .map(|name| controls.iter().find(|n| n.label() == Some(name)).unwrap().bounds())
                .into();
            assert!(
                left.iter().all(|b| b.top() == left[0].top()),
                "wide pickers must be one row: {left:?}"
            );
            assert!(
                setup.size.height < px(240.0),
                "wide setup has unexplained empty space: {setup:?}"
            );
        }
    }

    cx.simulate_resize(size(px(1750.0), px(900.0)));
    draw(cx);
    let before = cx.debug_bounds("compare-setup").unwrap();
    let options = cx
        .update(|window, _| gpui_kit::base::test_support::snapshots(window))
        .into_iter()
        .find(|node| node.path().last() == Some(&"compare-options".into()))
        .unwrap();
    cx.simulate_click(options.bounds().center(), Default::default());
    draw(cx);
    draw(cx);
    assert_eq!(
        cx.debug_bounds("compare-setup").unwrap(),
        before,
        "opening settings must not displace results"
    );
    assert!(cx.debug_bounds("compare-settings-panel").is_some());
    let suggested = cx
        .update(|window, _| gpui_kit::base::test_support::snapshots(window))
        .into_iter()
        .find(|node| node.label() == Some("Suggested match keys"))
        .unwrap();
    assert!(suggested.bounds().size.height <= px(32.0));
    cx.simulate_click(suggested.bounds().center(), Default::default());
    draw(cx);
    assert!(
        cx.update(|window, _| gpui_kit::base::test_support::snapshots(window))
            .iter()
            .any(|node| node.label() == Some("Suggested match keys")
                && node.expanded() == Some(true))
    );
    cx.simulate_keystrokes("escape");
    draw(cx);
    cx.simulate_click(suggested.bounds().center(), Default::default());
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert!(
        cx.debug_bounds("compare-settings-panel").is_some(),
        "choosing an indexed key keeps settings open"
    );
    assert!(
        cx.update(|window, _| gpui_kit::base::test_support::snapshots(window))
            .iter()
            .any(|node| node.label() == Some("Suggested match keys")
                && node.expanded() == Some(false)),
        "choosing an indexed key must close its dropdown"
    );
    let input = cx
        .update(|window, _| gpui_kit::base::test_support::snapshots(window))
        .into_iter()
        .find(|node| node.label() == Some("Custom match field"))
        .expect("named match input");
    cx.simulate_click(input.bounds().center(), Default::default());
    cx.simulate_input("logId");
    draw(cx);
    assert!(cx.update(|window, _| gpui_kit::base::test_support::snapshots(window)).iter()
        .any(|node| node.label() == Some("Custom match field") && node.value() == Some("logId")),
        "the custom key input must receive typing");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(
        state.read_with(cx, |app, _| app
            .compare_tab(app.active_compare_tab_id().unwrap())
            .unwrap()
            .config
            .fields
            .clone()),
        ["logId"]
    );
    assert!(
        cx.debug_bounds("compare-settings-panel").is_some(),
        "adding a key must keep settings open"
    );

    for width in [1750.0, 900.0, 430.0] {
        cx.simulate_resize(size(px(width), px(700.0)));
        draw(cx);
        draw(cx);
        if cx.debug_bounds("compare-settings").is_none() {
            let options = cx
                .update(|window, _| gpui_kit::base::test_support::snapshots(window))
                .into_iter()
                .find(|node| node.path().last() == Some(&"compare-options".into()))
                .unwrap();
            cx.simulate_click(options.bounds().center(), Default::default());
            draw(cx);
            draw(cx);
        }
        let panel = cx.debug_bounds("compare-settings").expect("settings popover");
        assert!(
            panel.left() >= px(0.0) && panel.right() <= px(width),
            "settings overflow at {width}px: {panel:?}"
        );
        assert!(
            panel.top() >= px(0.0) && panel.bottom() <= px(700.0),
            "settings are clipped at {width}px: {panel:?}"
        );
        let controls = cx.update(|window, _| gpui_kit::base::test_support::snapshots(window));
        for label in ["Custom match field", "Filter both collections", "Field to ignore"] {
            let bounds = controls.iter().find(|node| node.label() == Some(label)).unwrap().bounds();
            assert!(
                bounds.left() >= panel.left() && bounds.right() <= panel.right(),
                "{label} escapes settings"
            );
        }
    }
    let done = cx
        .update(|window, _| gpui_kit::base::test_support::snapshots(window))
        .into_iter()
        .find(|node| node.path().last() == Some(&"compare-settings-done".into()))
        .unwrap();
    let input = cx
        .update(|window, _| gpui_kit::base::test_support::snapshots(window))
        .into_iter()
        .find(|node| node.label() == Some("Custom match field"))
        .unwrap();
    cx.simulate_click(input.bounds().center(), Default::default());
    cx.simulate_input("tenantId");
    cx.simulate_click(done.bounds().center(), Default::default());
    draw(cx);
    assert!(cx.debug_bounds("compare-settings-panel").is_none(), "Done closes settings");
    assert_eq!(
        state.read_with(cx, |app, _| app
            .compare_tab(app.active_compare_tab_id().unwrap())
            .unwrap()
            .config
            .fields
            .clone()),
        ["logId", "tenantId"]
    );
}

#[gpui_kit::test]
fn compare_setup_wraps_and_tab_switches_preserve_setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
    });
    let config_dir = tempfile::tempdir().unwrap();
    let state = cx.new(|_| {
        AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(config_dir.path().into()),
        )
    });
    state.update(cx, |state, cx| state.open_compare_tab(None, cx));
    let first = state.read_with(cx, |state, _| state.active_compare_tab_id().unwrap());
    state.update(cx, |state, cx| {
        state.update_compare_config(first, |config| config.fields = vec!["sku".into()], cx)
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        let content = cx.new(|cx| ContentArea::new(state.clone(), cx));
        Root::new(content, window, cx).bordered(false)
    });
    for width in [430.0, 900.0, 1200.0] {
        cx.simulate_resize(size(px(width), px(900.0)));
        draw(cx);
        draw(cx);
        let setup = cx.debug_bounds("compare-setup").expect("setup is visible");
        let left = cx.debug_bounds("compare-side-0").expect("left picker");
        let right = cx.debug_bounds("compare-side-1").expect("right picker");
        let actions = cx.debug_bounds("compare-actions").expect("actions");
        assert!(!left.intersects(&right), "pickers overlap at {width}: {left:?}, {right:?}");
        assert!(actions.right() <= setup.right(), "actions overflow at {width}");
        assert!(left.right() <= setup.right() && right.right() <= setup.right());
        assert!(setup.size.height > px(100.0));
    }
    state.update(cx, |state, cx| state.open_compare_tab(None, cx));
    draw(cx);
    state.update(cx, |state, cx| state.select_tab(0, cx));
    draw(cx);
    assert_eq!(
        state.read_with(cx, |state, _| state.compare_tab(first).unwrap().config.fields.clone()),
        ["sku"]
    );
    state.update(cx, |state, cx| {
        let tab = state.compare_tab_mut(first).unwrap();
        tab.compared = Some(tab.config.clone());
        tab.rows.push(crate::connection::ops::compare::DiffRow {
            key: mongodb::bson::Bson::String("A".into()),
            left_id: Some(1.into()),
            right_id: Some(2.into()),
            kind: crate::connection::ops::compare::DiffKind::Different,
            changed: 1,
            paths: "price".into(),
            left_hash: 1,
            right_hash: 2,
            left_count: 1,
            right_count: 1,
        });
        tab.segments[0].push(0);
        tab.segments[3].push(0);
        tab.selected = Some(0);
        tab.detail_row = Some(0);
        tab.detail = Some(Arc::new(CompareDetail {
            documents: [
                vec![doc! {"_id":1, "sku":"A", "price":1, "same":true}],
                vec![doc! {"_id":2, "sku":"A", "price":2, "same":true}],
            ],
            changed_since_scan: true,
        }));
        cx.notify();
    });
    draw(cx);
    draw(cx);
    assert!(cx.debug_bounds("compare-view").is_some());
}

#[test]
fn details_keep_ignored_ids_visible_and_fold_unchanged_fields() {
    let pair = CompareDetail {
        documents: [
            vec![doc! {"_id": 1, "sku": "x", "nested": {"price": 1}, "same": true}],
            vec![doc! {"_id": 2, "sku": "x", "nested": {"price": 2}, "same": true}],
        ],
        changed_since_scan: false,
    };
    let config = CompareConfig { fields: vec!["sku".into()], ..Default::default() };
    let mut expansion = super::detail_tree::Expansion::default();
    let rows = detail_rows(&pair, &config, &expansion).unwrap();
    assert!(matches!(rows[0], DetailRow::Field { informational: true, .. }));
    assert!(rows.iter().any(|row| matches!(row, DetailRow::Unchanged { count: 2, .. })));
    expansion.unchanged.insert(Vec::new());
    assert!(detail_rows(&pair, &config, &expansion).unwrap().len() > rows.len());
}

#[test]
fn compare_config_roundtrips_without_results_and_drop_cancels_work() {
    let config = CompareConfig {
        sides: [
            CompareEndpoint {
                connection_id: Some(uuid::Uuid::new_v4()),
                database: "shop".into(),
                collection: "orders".into(),
            },
            CompareEndpoint::default(),
        ],
        fields: vec!["sku".into()],
        ..Default::default()
    };
    let mut tab = CompareTabState::new(config.clone());
    let token = tab.begin();
    let encoded = serde_json::to_string(&config).unwrap();
    assert!(!encoded.contains("rows"));
    assert_eq!(serde_json::from_str::<CompareConfig>(&encoded).unwrap(), config);
    drop(tab);
    assert!(token.is_cancelled());
}

#[test]
fn first_custom_key_replaces_default_id_but_explicit_compound_keys_are_preserved() {
    let mut config = CompareConfig::default();
    config.add_match_fields("logId");
    assert_eq!(config.fields, ["logId"]);
    config.add_match_fields("tenantId, logId");
    assert_eq!(config.fields, ["logId", "tenantId"]);
    config.add_match_fields("_id");
    assert_eq!(config.fields, ["logId", "tenantId", "_id"]);
    let mut explicit = CompareConfig::default();
    explicit.add_match_fields("logId, _id");
    assert_eq!(explicit.fields, ["logId", "_id"]);
}

#[gpui_kit::test]
fn compare_pickers_take_arrow_keys_and_enter(cx: &mut TestAppContext) {
    use std::collections::HashMap;

    use crate::models::{ActiveConnection, SavedConnection};
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let saved = SavedConnection::new("Local".into(), "mongodb://localhost:27017".into());
    let connection = saved.id;
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = runtime.block_on(async {
        mongodb::Client::with_options(mongodb::options::ClientOptions::default()).unwrap()
    });
    let state = cx.new(|_| {
        let mut state = AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(directory.path().into()),
        );
        state.connections = vec![saved.clone()];
        state.insert_active_connection(
            connection,
            ActiveConnection {
                config: saved,
                client,
                databases: vec!["shop".into(), "store".into(), "warehouse".into()],
                // Every database has its collections, so no picker reaches for the runtime.
                collections: HashMap::from([
                    ("shop".to_string(), vec!["alpha".to_string()]),
                    ("store".to_string(), vec!["alpha".to_string()]),
                    ("warehouse".to_string(), vec!["alpha".to_string()]),
                ]),
                collection_details: Default::default(),
                runtime_meta: Default::default(),
            },
        );
        state
    });
    // The collection stays empty throughout: a complete endpoint would load metadata.
    let id = state.update(cx, |state, cx| {
        state.open_compare_tab(
            Some(CompareEndpoint {
                connection_id: Some(connection),
                database: String::new(),
                collection: String::new(),
            }),
            cx,
        );
        state.active_compare_tab_id().unwrap()
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| ContentArea::new(state.clone(), cx));
        Root::new(view, window, cx).bordered(false)
    });
    cx.simulate_resize(size(px(1400.0), px(900.0)));
    draw(cx);
    draw(cx);
    let database = |cx: &mut VisualTestContext| {
        state.update(cx, |state, _| state.compare_tab(id).unwrap().config.sides[0].database.clone())
    };
    let open_picker = |cx: &mut VisualTestContext| {
        let trigger = cx
            .update(|window, _| gpui_kit::base::test_support::snapshots(window))
            .iter()
            .find(|node| node.label() == Some("Left database"))
            .expect("left database picker")
            .bounds();
        cx.simulate_click(trigger.center(), gpui_kit::Modifiers::default());
        draw(cx);
    };
    // Nothing chosen yet: two arrows down land on the second entry.
    open_picker(cx);
    for key in ["down", "down", "enter"] {
        cx.simulate_keystrokes(key);
        draw(cx);
    }
    assert_eq!(database(cx), "store", "arrows and enter must pick from an empty picker");
    // Something chosen: one arrow down moves off it, and re-rendering must not snap it back.
    open_picker(cx);
    for key in ["down", "enter"] {
        cx.simulate_keystrokes(key);
        draw(cx);
    }
    assert_eq!(database(cx), "warehouse", "arrows and enter must move off the current value");
}

#[test]
fn a_new_run_keeps_the_previous_results_until_it_reports() {
    use crate::connection::ops::compare::{CompareMessage, CompareSummary, DiffKind, DiffRow};
    let row = DiffRow {
        key: 1.into(),
        left_id: None,
        right_id: None,
        kind: DiffKind::OnlyLeft,
        changed: 0,
        paths: "".into(),
        left_hash: 1,
        right_hash: 0,
        left_count: 1,
        right_count: 0,
    };
    let done = || {
        CompareMessage::Done(CompareSummary {
            counts: Default::default(),
            skipped: Some([0, 0]),
            truncated: false,
            cancelled: false,
            elapsed: Default::default(),
        })
    };
    let mut tab = CompareTabState::new(CompareConfig::default());
    tab.begin();
    tab.receive(CompareMessage::Progress {
        counts: Default::default(),
        new_rows: vec![row],
        left_started: true,
        right_started: true,
    });
    tab.receive(done());
    tab.selected = Some(0);
    assert_eq!(tab.rows.len(), 1);

    tab.begin();
    assert!(tab.running && !tab.busy(), "a fresh run is not shown as busy yet");
    tab.slow = true;
    assert!(tab.busy());
    assert_eq!((tab.rows.len(), tab.selected), (1, Some(0)), "old results stay while scanning");
    tab.receive(done());
    assert!(tab.rows.is_empty() && tab.selected.is_none() && !tab.running);
}
