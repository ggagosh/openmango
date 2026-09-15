use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::Root;
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, Window, div, px,
    size,
};

use super::types::{ActionCategory, ActionItem, PaletteMode};
use super::{ActionBar, MAX_RESULTS, build_groups};
use crate::state::{AppState, ConfigManager};
use crate::theme::spacing;

fn command(id: &'static str, label: &'static str) -> ActionItem {
    ActionItem { id: id.into(), label: label.into(), available: true, ..Default::default() }
}

#[test]
fn groups_lead_with_recent_and_updates_and_cap_long_lists() {
    let mut actions = vec![
        ActionItem { keywords: &["dump"], ..command("cmd:export", "Export Data") },
        command("cmd:settings", "Settings"),
        ActionItem { highlighted: true, ..command("cmd:install-update", "Restart to Update") },
        ActionItem { category: ActionCategory::Tab, ..command("tab:0", "orders") },
    ];
    actions.extend((0..150).map(|ix| ActionItem {
        id: format!("nav:col:{ix}").into(),
        label: format!("collection {ix}").into(),
        category: ActionCategory::Navigation,
        available: true,
        ..Default::default()
    }));

    let (groups, hidden) = build_groups(&actions, PaletteMode::All, "", &["cmd:export".into()]);
    let labels = groups.iter().map(|group| group.label).collect::<Vec<_>>();
    assert_eq!(labels, [Some("Recent"), Some("Commands"), Some("Tabs"), Some("Navigation")]);
    assert_eq!(groups[1].items[0].id.as_ref(), "cmd:install-update");
    assert_eq!(groups.iter().map(|group| group.items.len()).sum::<usize>(), MAX_RESULTS);
    assert_eq!(hidden, actions.len() - MAX_RESULTS);

    let (groups, hidden) = build_groups(&actions, PaletteMode::All, "dump", &[]);
    assert_eq!(groups[0].items[0].id.as_ref(), "cmd:export");
    assert_eq!(hidden, 0);
}

struct Host {
    bar: Entity<ActionBar>,
    /// A tab stop behind the palette that Tab must not reach.
    workspace: FocusHandle,
    _config: tempfile::TempDir,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .child(div().size_4().track_focus(&self.workspace))
            .child(self.bar.clone())
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

type Executed = Rc<RefCell<Vec<String>>>;

fn open_palette(cx: &mut TestAppContext) -> (Entity<ActionBar>, Executed, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply_design_tokens(cx);
    });
    let config = tempfile::tempdir().unwrap();
    let state = cx.new(|_| {
        AppState::with_config(
            Arc::new(crate::connection::ConnectionManager::new()),
            ConfigManager::with_config_dir(config.path().into()),
        )
    });
    let executed = Rc::new(RefCell::new(Vec::<String>::new()));
    let mut bar = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let executed = executed.clone();
        let action_bar = cx.new(|cx| {
            ActionBar::new(state.clone(), cx).on_execute(move |execution, _, _| {
                executed.borrow_mut().push(execution.action_id.to_string())
            })
        });
        bar = Some(action_bar.clone());
        let host = cx.new(|cx| Host {
            bar: action_bar,
            workspace: cx.focus_handle().tab_stop(true),
            _config: config,
        });
        Root::new(host, window, cx).bordered(false)
    });
    let bar = bar.unwrap();
    cx.update(|window, cx| bar.update(cx, |bar, cx| bar.toggle(window, cx)));
    draw(cx);
    (bar, executed, cx)
}

#[gpui_kit::test]
fn keyboard_stays_in_the_palette_and_steps_back_before_closing(cx: &mut TestAppContext) {
    let (bar, executed, cx) = open_palette(cx);
    let mode = |cx: &mut VisualTestContext| bar.read_with(cx, |bar, _| bar.mode);
    let is_open = |cx: &mut VisualTestContext| bar.read_with(cx, |bar, _| bar.command.is_some());

    cx.simulate_keystrokes("tab");
    cx.update(|window, cx| assert!(bar.read(cx).trap.contains_focused(window, cx)));

    cx.simulate_input("theme");
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(mode(cx), PaletteMode::Theme);
    cx.simulate_keystrokes("backspace");
    draw(cx);
    assert_eq!(mode(cx), PaletteMode::All);

    // A leading @ or # jumps to that scope and keeps the rest of the query.
    let query = |cx: &mut VisualTestContext| {
        bar.read_with(cx, |bar, cx| bar.command.as_ref().unwrap().read(cx).query(cx).to_string())
    };
    cx.simulate_input("@prod");
    draw(cx);
    assert_eq!((mode(cx), query(cx).as_str()), (PaletteMode::Connect, "prod"));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" });
    cx.simulate_keystrokes("backspace");
    draw(cx);
    assert_eq!(mode(cx), PaletteMode::Connect, "clearing the query keeps the scope");
    cx.simulate_keystrokes("backspace");
    draw(cx);
    assert_eq!(mode(cx), PaletteMode::All);
    cx.simulate_input("#");
    draw(cx);
    assert_eq!(mode(cx), PaletteMode::Navigate);
    cx.simulate_keystrokes("backspace");
    draw(cx);
    assert_eq!(mode(cx), PaletteMode::All);

    cx.simulate_input("zz");
    draw(cx);
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(is_open(cx), "the first Escape only clears the query");
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(!is_open(cx));

    cx.update(|window, cx| bar.update(cx, |bar, cx| bar.toggle(window, cx)));
    draw(cx);
    cx.simulate_input("settings");
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(*executed.borrow(), ["cmd:settings"]);
    assert!(!is_open(cx));
}

#[gpui_kit::test]
fn palette_stays_centered_inside_narrow_windows(cx: &mut TestAppContext) {
    let (_, _, cx) = open_palette(cx);
    for (width, expected) in [(px(1200.), px(620.)), (px(420.), px(420.) - spacing::lg() * 2.)] {
        cx.simulate_resize(size(width, px(360.)));
        draw(cx);
        let card = cx.debug_bounds("action-bar-card").expect("palette card rendered");
        assert_eq!(card.size.width, expected, "card width in a {width:?} window");
        assert_eq!(card.left(), width - card.right(), "card centered in a {width:?} window");
        assert!(card.bottom() <= px(360.), "card fits a short window: {card:?}");
    }
}
