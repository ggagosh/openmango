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

#[gpui_kit::test]
fn two_picked_documents_open_side_by_side_with_id_as_information(cx: &mut TestAppContext) {
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
    let documents = [
        doc! {"_id": 1, "name": "left", "n": 1, "same": true},
        doc! {"_id": 2, "name": "right", "n": 1, "same": true},
    ];
    // Picked documents are never paired by _id: it is information, not a difference.
    let pair = CompareDetail {
        documents: documents.clone().map(|document| vec![document]),
        changed_since_scan: false,
    };
    let config = CompareConfig { fields: Vec::new(), ..Default::default() };
    let rows = detail_rows(&pair, &config, &Default::default()).unwrap();
    assert!(matches!(&rows[0], DetailRow::Field { informational: true, .. }));
    assert!(matches!(&rows[1], DetailRow::Field { kind: Some(_), informational: false, .. }));
    assert!(matches!(rows[2], DetailRow::Unchanged { count: 2, .. }));
    assert_eq!(rows.len(), 3);

    // The app's root view draws the dialog layer; this host stands in for it.
    struct DialogHost;
    impl gpui_kit::Render for DialogHost {
        fn render(
            &mut self,
            window: &mut gpui_kit::Window,
            cx: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            use gpui_kit::{ParentElement as _, Styled as _};
            gpui_kit::div().size_full().children(Root::render_dialog_layer(window, cx))
        }
    }
    let (_, cx) = cx.add_window_view(|window, cx| {
        let host = cx.new(|_| DialogHost);
        Root::new(host, window, cx).bordered(false)
    });
    cx.simulate_resize(size(px(1400.0), px(900.0)));
    cx.update(|window, cx| {
        super::open_document_compare(state.clone(), "shop.items".into(), documents, window, cx)
    });
    draw(cx);
    draw(cx);
    assert!(cx.debug_bounds("document-compare").is_some(), "the dialog opens");
    for (cell, heading) in
        [("compare-value-0", "compare-heading-0"), ("compare-value-3", "compare-heading-1")]
    {
        let value = cx.debug_bounds(cell).unwrap_or_else(|| panic!("missing {cell}"));
        let title = cx.debug_bounds(heading).unwrap();
        assert!(
            (f32::from(value.left() - title.left())).abs() <= 1.0,
            "{cell} is not under {heading}: value={value:?}, heading={title:?}"
        );
    }
    assert!(
        cx.debug_bounds("compare-detail-row-2").is_some(),
        "unchanged fields fold into one row"
    );
}

#[gpui_kit::test]
fn a_closed_connection_picked_in_compare_opens_in_place(cx: &mut TestAppContext) {
    use std::collections::HashMap;

    use crate::models::{ActiveConnection, SavedConnection};
    use crate::state::{AppEvent, View};
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let local = SavedConnection::new("Local".into(), "mongodb://localhost:27017".into());
    let remote = SavedConnection::new("Remote".into(), "mongodb://localhost:27018".into());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = || {
        runtime.block_on(async {
            mongodb::Client::with_options(mongodb::options::ClientOptions::default()).unwrap()
        })
    };
    let active = |saved: &SavedConnection, client| ActiveConnection {
        config: saved.clone(),
        client,
        databases: vec!["shop".into()],
        // Collections are known, so no picker reaches for the runtime.
        collections: HashMap::from([("shop".to_string(), Vec::new())]),
        collection_details: Default::default(),
        runtime_meta: Default::default(),
    };
    let state = cx.new(|_| {
        let mut state = AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(directory.path().into()),
        );
        state.connections = vec![local.clone(), remote.clone()];
        state.insert_active_connection(local.id, active(&local, client()));
        // A connect attempt fails at once instead of dialing a server.
        state.connections_persistence_blocked = true;
        state
    });
    let id = state.update(cx, |state, cx| {
        state.open_compare_tab(
            Some(CompareEndpoint {
                connection_id: Some(local.id),
                database: "shop".into(),
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
    let right = |cx: &mut VisualTestContext| {
        state.update(cx, |state, _| state.compare_tab(id).unwrap().config.sides[1].clone())
    };
    let pick_right_connection = |keys: &[&str], cx: &mut VisualTestContext| {
        let trigger = cx
            .update(|window, _| gpui_kit::base::test_support::snapshots(window))
            .iter()
            .find(|node| node.label() == Some("Right connection"))
            .expect("right connection picker")
            .bounds();
        cx.simulate_click(trigger.center(), gpui_kit::Modifiers::default());
        draw(cx);
        for key in keys {
            cx.simulate_keystrokes(key);
            draw(cx);
        }
    };
    let connect_button = |cx: &mut VisualTestContext| {
        cx.update(|window, _| gpui_kit::base::test_support::snapshots(window))
            .iter()
            .any(|node| node.path().last() == Some(&gpui_kit::ElementId::from("compare-connect")))
    };

    // The closed connection is listed after the open one and picking it stays in this tab.
    pick_right_connection(&["down", "down", "enter"], cx);
    assert_eq!(right(cx).connection_id, Some(remote.id), "closed connections are offered");
    state.update(cx, |state, _| {
        assert_eq!(state.current_view, View::Compare);
        assert_eq!(state.active_compare_tab_id(), Some(id));
    });
    assert!(connect_button(cx), "a failed attempt leaves a way to retry");

    // Once it opens, the right side follows the left database.
    state.update(cx, |state, cx| {
        state.insert_active_connection(remote.id, active(&remote, client()));
        cx.emit(AppEvent::Connected(remote.id));
        cx.notify();
    });
    draw(cx);
    draw(cx);
    assert_eq!(right(cx).database, "shop");
    assert!(!connect_button(cx));

    // Picking the same connection again keeps what was chosen under it.
    pick_right_connection(&["enter"], cx);
    assert_eq!(right(cx).database, "shop");
}

fn listing() -> Vec<crate::connection::ops::compare_database::CollectionPair> {
    use crate::connection::ops::compare_database::{
        CollectionKind::*, CollectionPair, SideCollection,
    };
    let side =
        |kind| Some(SideCollection { kind, estimated: Some(10), bytes: Some(100), indexes: None });
    vec![
        CollectionPair { name: "audit".into(), sides: [side(Collection), None] },
        CollectionPair { name: "orders".into(), sides: [side(Collection), side(Collection)] },
        CollectionPair { name: "recent".into(), sides: [side(View), side(Collection)] },
        CollectionPair { name: "zones".into(), sides: [None, side(Collection)] },
    ]
}

fn database_config(connections: [uuid::Uuid; 2]) -> CompareConfig {
    use crate::state::compare::CompareScope;
    CompareConfig {
        scope: CompareScope::Databases,
        sides: connections.map(|connection| CompareEndpoint {
            connection_id: Some(connection),
            database: "shop".into(),
            collection: String::new(),
        }),
        fields: vec!["sku".into()],
        ignore: vec!["updatedAt".into()],
        ..Default::default()
    }
}

#[test]
fn database_scope_loads_old_setups_and_segments_the_listing() {
    use crate::state::compare::CompareScope;
    let old: CompareConfig = serde_json::from_str(r#"{"fields":["sku"]}"#).unwrap();
    assert_eq!(old.scope, CompareScope::Collections, "saved tabs from before load unchanged");
    let config = database_config([uuid::Uuid::new_v4(); 2]);
    let back: CompareConfig =
        serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert_eq!(back, config);

    let mut tab = CompareTabState::new(config);
    tab.begin();
    let scans = tab.receive_pairs(Ok(listing()));
    assert_eq!(scans.iter().map(|scan| scan.index).collect::<Vec<_>>(), [1]);
    assert!(tab.running, "the content scan follows the listing");
    // All, left only, right only, different, minor, identical, not compared.
    assert_eq!(tab.pair_segments.each_ref().map(Vec::len), [3, 1, 1, 0, 0, 0, 1]);
    assert_eq!(tab.find_pair("ORD"), Some(1));
    assert_eq!(tab.find_pair("missing"), None);
    // A second run keeps the listing on screen until its own result arrives.
    tab.finish_scan();
    tab.begin();
    assert_eq!(tab.pairs.len(), 4);
    tab.receive_pairs(Err("listCollections refused".into()));
    assert!(tab.pairs.is_empty());
    assert!(!tab.running);
    assert_eq!(tab.error.as_deref(), Some("listCollections refused"));
}

fn summary(
    identical: u64,
    different: u64,
    cancelled: bool,
) -> crate::connection::ops::compare::CompareSummary {
    crate::connection::ops::compare::CompareSummary {
        counts: crate::connection::ops::compare::CompareCounts {
            identical,
            different,
            left_read: identical + different,
            right_read: identical + different,
            ..Default::default()
        },
        skipped: None,
        truncated: false,
        cancelled,
        elapsed: std::time::Duration::from_millis(5),
    }
}

#[test]
fn database_scan_settles_rows_in_place_and_says_why_a_collection_was_not_compared() {
    use crate::connection::ops::compare_database::PairMessage;
    use crate::state::compare::{PairProgress, PairStatus};
    let mut tab = CompareTabState::new(database_config([uuid::Uuid::new_v4(); 2]));
    tab.begin();
    tab.receive_pairs(Ok(listing()));
    tab.receive_pair(PairMessage::Started(1));
    assert_eq!(tab.pair_status(1), PairStatus::Scanning);
    tab.receive_pair(PairMessage::Done(1, summary(10, 0, false)));
    assert_eq!(tab.pair_status(1), PairStatus::Identical);
    assert!(tab.pair_segments[0].contains(&1), "rows hold still while the run lasts");
    assert_eq!(tab.pair_segment_counts()[5], 1, "while the counts are already live");
    tab.finish_scan();
    assert!(!tab.running);
    assert!(!tab.pair_segments[0].contains(&1), "identical collections leave All at the end");
    assert_eq!(tab.pair_segments[5], [1]);
    assert!(tab.pair_elapsed.is_some());

    // Recheck, then skip before its turn: skipped at once, and its reader never starts.
    let scan = tab.recheck_pair(1).unwrap();
    assert!(tab.running && matches!(tab.pair_progress[1], PairProgress::Waiting));
    tab.skip_pair(1);
    assert!(scan.cancellation.is_cancelled());
    assert_eq!(tab.pair_status(1), PairStatus::Skipped);
    tab.finish_scan();

    // Cancel: the collection being read and those never reached both say Cancelled.
    let scan = tab.recheck_pair(1).unwrap();
    tab.receive_pair(PairMessage::Started(1));
    tab.cancel_run();
    assert!(scan.cancellation.is_cancelled());
    tab.receive_pair(PairMessage::Done(1, summary(3, 0, true)));
    tab.finish_scan();
    assert_eq!(tab.pair_status(1), PairStatus::Cancelled);
    assert_eq!(tab.pair_segments[6].len(), 2, "the view and the cancelled collection");

    // A collection under Skip collections is never scheduled.
    let mut config = database_config([uuid::Uuid::new_v4(); 2]);
    config.skip = vec!["orders".into()];
    let mut tab = CompareTabState::new(config);
    tab.begin();
    assert!(tab.receive_pairs(Ok(listing())).is_empty());
    assert!(!tab.running);
    assert_eq!(tab.pair_status(1), PairStatus::Skipped);
}

#[gpui_kit::test]
fn a_collection_opened_from_a_database_comparison_keeps_both_sides_and_ignored_fields(
    cx: &mut TestAppContext,
) {
    use crate::state::compare::CompareScope;
    let directory = tempfile::tempdir().unwrap();
    let state = cx.new(|_| {
        AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(directory.path().into()),
        )
    });
    let connections = [uuid::Uuid::new_v4(), uuid::Uuid::new_v4()];
    state.update(cx, |state, cx| {
        let mut unfinished = database_config(connections);
        unfinished.sides[0].database.clear();
        assert_eq!(
            state.compare_disabled_reason(&unfinished).as_deref(),
            Some("Choose a connection and database on the left")
        );
        assert_eq!(
            state.compare_disabled_reason(&database_config(connections)).as_deref(),
            Some("Left connection is closed. Reconnect to compare.")
        );

        let id = state.open_compare_tab_with(database_config(connections), cx);
        let tab = state.compare_tab_mut(id).unwrap();
        tab.begin();
        tab.receive_pairs(Ok(listing()));
        let opened = state.open_pair_comparison(id, 1, cx).unwrap();
        assert_eq!(state.active_compare_tab_id(), Some(opened));
        let config = &state.compare_tab(opened).unwrap().config;
        assert_eq!(config.scope, CompareScope::Collections);
        assert_eq!(
            config.sides.each_ref().map(|side| (side.connection_id, side.collection.as_str())),
            [(Some(connections[0]), "orders"), (Some(connections[1]), "orders")]
        );
        assert_eq!(config.ignore, ["updatedAt"]);
        assert_eq!(config.fields, ["_id"], "every collection of a database is matched by _id");

        // A collection on one side only is copied through Transfer, which opens for review.
        assert!(!state.open_pair_copy(id, 1, cx), "a collection on both sides is compared instead");
        assert!(state.open_pair_copy(id, 0, cx));
        let transfer = state.active_transfer_tab_id().and_then(|tab| state.transfer_tab(tab));
        let config = &transfer.expect("a Transfer tab").config;
        assert_eq!(
            (config.source_connection_id, config.source_collection.as_str()),
            (Some(connections[0]), "audit")
        );
        assert_eq!(
            (config.destination_connection_id, config.destination_collection.as_str()),
            (Some(connections[1]), "audit")
        );
    });
}

#[gpui_kit::test]
fn database_results_take_arrow_keys_find_and_enter(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
        crate::keyboard::bind_keymap(cx, &Default::default());
    });
    let directory = tempfile::tempdir().unwrap();
    let state = cx.new(|_| {
        AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(directory.path().into()),
        )
    });
    let id = state.update(cx, |state, cx| {
        let id = state.open_compare_tab_with(database_config([uuid::Uuid::new_v4(); 2]), cx);
        let tab = state.compare_tab_mut(id).unwrap();
        tab.begin();
        tab.receive_pairs(Ok(listing()));
        id
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| ContentArea::new(state.clone(), cx));
        Root::new(view, window, cx).bordered(false)
    });
    cx.simulate_resize(size(px(1400.0), px(900.0)));
    draw(cx);
    draw(cx);
    let selected = |cx: &mut VisualTestContext| {
        state.update(cx, |state, _| state.compare_tab(id).unwrap().pair_selected)
    };
    assert!(cx.debug_bounds("compare-pair-heading").is_some());
    assert!(cx.debug_bounds("compare-pair-detail").is_none(), "nothing is selected yet");

    let row = cx.debug_bounds("compare-pair-0").expect("the first collection row");
    cx.simulate_click(row.center(), gpui_kit::Modifiers::default());
    draw(cx);
    assert_eq!(selected(cx), Some(0));
    assert!(cx.debug_bounds("compare-pair-detail").is_some());
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(selected(cx), Some(1));
    cx.simulate_keystrokes("up");
    draw(cx);
    assert_eq!(selected(cx), Some(0));

    // With both connections closed, enter opens nothing, not even Transfer for this one.
    let tabs = state.update(cx, |state, _| state.open_tabs().len());
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(state.update(cx, |state, _| state.open_tabs().len()), tabs);

    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("zon");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(selected(cx), Some(3), "Find jumps to a collection by part of its name");

    // While a collection is read, Skip sits beside its name in the status line.
    state.update(cx, |state, _| {
        let tab = state.compare_tab_mut(id).unwrap();
        tab.slow = true;
        tab.receive_pair(crate::connection::ops::compare_database::PairMessage::Started(1));
    });
    draw(cx);
    let skip = cx
        .update(|window, _| gpui_kit::base::test_support::snapshots(window))
        .into_iter()
        .find(|node| node.label() == Some("Skip orders"))
        .expect("Skip beside the collection being read")
        .bounds();
    cx.simulate_click(skip.center(), gpui_kit::Modifiers::default());
    draw(cx);
    assert!(state.update(cx, |state, _| state.compare_tab(id).unwrap().pair_tokens[1].is_none()));
}
