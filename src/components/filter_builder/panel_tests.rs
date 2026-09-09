use std::sync::Arc;

use gpui_kit::component::{Root, Theme};
use gpui_kit::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, KeyDownEvent, KeyUpEvent,
    Keystroke, Modifiers, ParentElement as _, Pixels, Render, Styled as _, TestAppContext,
    VisualTestContext, Window, div, px,
};
use mongodb::bson::{Document, doc};

use super::FilterBuilderPanel;
use crate::components::filter_builder::types::{ConditionValue, FieldType};
use crate::connection::ConnectionManager;
use crate::state::{AppState, ConfigManager, SessionKey};

struct Harness {
    panel: Entity<FilterBuilderPanel>,
    width: Pixels,
    _config: tempfile::TempDir,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(self.width).h(px(640.)).child(self.panel.clone())
    }
}

fn harness(
    cx: &mut TestAppContext,
    source: Document,
) -> (Entity<Harness>, Entity<FilterBuilderPanel>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::global_mut(cx).font_family = crate::theme::fonts::ui().into();
        Theme::global_mut(cx).font_size = px(16.);
    });
    let mut harness = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| {
            let config = tempfile::tempdir().unwrap();
            let mut state = AppState::with_config(
                Arc::new(ConnectionManager::new()),
                ConfigManager::with_config_dir(config.path().to_owned()),
            );
            let key = SessionKey::new(uuid::Uuid::new_v4(), "test", "items");
            state.ensure_session(key.clone()).data.loaded = true;
            state.set_filter(&key, "{ active: true }".into(), Some(doc! { "active": true }));
            let state = cx.new(|_| state);
            let input = cx.new(|cx| gpui_kit::component::input::EditorState::new(window, cx));
            let panel = cx.new(|cx| {
                let mut panel = FilterBuilderPanel::new(state, key, input, window, cx);
                panel.populate_from_document(&source, window, cx);
                panel
            });
            Harness { panel, width: px(480.), _config: config }
        });
        harness = Some(view.clone());
        Root::new(view, window, cx).bordered(false)
    });
    let harness = harness.unwrap();
    let panel = harness.read_with(cx, |view, _| view.panel.clone());
    draw(cx);
    (harness, panel, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn collapsing_groups_preserves_the_query_and_draft_inputs(cx: &mut TestAppContext) {
    let (_, panel, cx) = harness(
        cx,
        doc! { "$and": [ { "active": true }, { "$or": [{ "name": "mango" }, { "name": "pear" }] } ] },
    );
    let before = panel.read_with(cx, |panel, _| panel.tree.clone());
    let fields = panel.read_with(cx, |panel, _| {
        panel
            .condition_inputs
            .iter()
            .map(|(id, inputs)| (*id, inputs.field_state.entity_id()))
            .collect::<std::collections::HashMap<_, _>>()
    });
    let toggle = cx.debug_bounds("builder-group-toggle-2").unwrap();
    cx.update(|window, cx| panel.update(cx, |panel, cx| panel.focus_condition(3, window, cx)));
    cx.simulate_click(toggle.center(), Modifiers::default());
    draw(cx);
    assert!(panel.read_with(cx, |panel, _| panel.collapsed_groups.contains(&2)));
    assert!(cx.debug_bounds("builder-group-content-2").is_none());
    assert!(cx.update(|window, cx| panel.read(cx).focus_handle(cx).is_focused(window)));
    assert_eq!(panel.read_with(cx, |panel, _| panel.tree.clone()), before);
    let toggle = cx.debug_bounds("builder-group-toggle-2").unwrap();
    cx.simulate_click(toggle.center(), Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("builder-group-content-2").is_some());
    assert_eq!(panel.read_with(cx, |panel, _| panel.tree.clone()), before);
    panel.read_with(cx, |panel, _| {
        for (id, entity_id) in fields {
            assert_eq!(panel.condition_inputs[&id].field_state.entity_id(), entity_id);
        }
    });
}

#[gpui_kit::test]
fn builder_add_focus_and_invalid_run_preserve_the_applied_filter(cx: &mut TestAppContext) {
    let (_, panel, cx) = harness(cx, doc! {});
    cx.update(|window, cx| panel.update(cx, |panel, cx| panel.add_condition(window, cx)));
    draw(cx);
    let id = panel.read_with(cx, |panel, _| panel.tree.conditions()[0].id);
    assert!(cx.update(|window, cx| {
        panel.read(cx).condition_inputs[&id]
            .field_state
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }));
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            let condition = panel.tree.condition_mut(id).unwrap();
            condition.field = "age".into();
            condition.set_field_type(FieldType::Number);
            condition.value = ConditionValue::Scalar("invalid".into());
            panel.sync_condition_inputs(id, window, cx);
        })
    });
    draw(cx);
    assert!(!panel.read_with(cx, |panel, _| panel.can_run()));
    let key = Keystroke::parse("cmd-enter").unwrap();
    cx.simulate_event(KeyDownEvent {
        keystroke: key.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent { keystroke: key });
    cx.update(|window, cx| panel.update(cx, |panel, cx| panel.apply_filter(window, cx)));
    assert_eq!(
        panel.read_with(cx, |panel, cx| panel
            .state
            .read(cx)
            .session_data(&panel.session_key)
            .unwrap()
            .filter
            .clone()),
        Some(doc! { "active": true })
    );
}

#[gpui_kit::test]
fn builder_nested_controls_fit_compact_widths_and_ui_scale(cx: &mut TestAppContext) {
    let (view, panel, cx) = harness(
        cx,
        doc! { "$and": [ { "active": true }, { "$or": [{ "name": "mango" }, { "name": "pear" }] } ] },
    );
    for (width, font) in [(480., 16.), (360., 16.), (480., 20.)] {
        view.update(cx, |view, cx| {
            view.width = px(width);
            cx.notify();
        });
        cx.update(|window, cx| {
            Theme::global_mut(cx).font_size = px(font);
            window.refresh();
        });
        draw(cx);
        let bounds = cx.debug_bounds("filter-builder").unwrap();
        assert_eq!(bounds.size.width, px(width));
        let footer = cx.debug_bounds("builder-footer").unwrap();
        let body = cx.debug_bounds("builder-body").unwrap();
        assert!(footer.bottom() <= bounds.bottom());
        assert!(
            body.bottom() <= footer.top(),
            "body/footer must not overlap: width={width}, font={font}, body={body:?}, footer={footer:?}, panel={bounds:?}"
        );
        let field = cx.debug_bounds("builder-field-1").unwrap();
        let operator = cx.debug_bounds("builder-operator-1").unwrap();
        assert_eq!(
            field.size.height, operator.size.height,
            "native inputs and operator buttons must align"
        );
        assert!((field.top() - operator.top()).abs() <= px(1.));
        panel.read_with(cx, |panel, cx| {
            for inputs in panel.condition_inputs.values() {
                let field = inputs.field_state.read(cx).input_bounds();
                assert!(
                    field.left() >= bounds.left() && field.right() <= bounds.right(),
                    "field must stay in the panel: {field:?} within {bounds:?}"
                );
                assert!(field.size.width >= px(45.));
            }
        });
    }
}
