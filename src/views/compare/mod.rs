//! Compare is a native data workbench: setup above, differences and document detail below.
//! It inherits the app's controls, theme and keyboard conventions.
//!
//! Visual system: each side has a fixed hue (Left cyan, Right magenta) that marks its column,
//! its picker and every row that exists on that side only. Changed values use the warning tint.
//! The same dot appears wherever a kind or a side is named, so the legend is always on screen.

mod database;
mod detail;
mod detail_tree;
mod results;
mod setup;
mod sync_bar;
#[cfg(test)]
mod tests;

pub(crate) use detail::open_document_compare;

use std::collections::{HashMap, HashSet};

use gpui_kit::component::input::InputState;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use uuid::Uuid;

use crate::components::Button;
use crate::connection::ops::compare::DiffKind;
use crate::keyboard::{
    CancelCompare, CompareNext, ComparePrevious, FindInCompare, FocusCompareDetail, RunCompare,
};
use crate::state::compare::{CompareEndpoint, CompareScope};
use crate::state::{AppCommands, AppState};
use crate::theme::{borders, islands, spacing};

pub struct CompareView {
    state: Entity<AppState>,
    focus: FocusHandle,
    detail_focus: FocusHandle,
    controls: Option<setup::Controls>,
    active: Option<Uuid>,
    options_open: bool,
    auto_right: bool,
    metadata_requested: [Option<CompareEndpoint>; 2],
    collections_requested: HashSet<(Uuid, String)>,
    /// Connections opening now, from a picker here or anywhere else in the app.
    connecting: HashSet<Uuid>,
    connect_errors: HashMap<Uuid, String>,
    scroll: UniformListScrollHandle,
    diff: Option<Entity<detail::DiffTable>>,
    detail_signature: Option<(Uuid, u64, usize, usize)>,
    find_error: Option<String>,
    _subscriptions: Vec<Subscription>,
    control_subscriptions: Vec<Subscription>,
}

pub(super) fn side_name(side: usize) -> &'static str {
    if side == 0 { "Left" } else { "Right" }
}

pub(super) fn side_color(side: usize, cx: &App) -> Hsla {
    if side == 0 { cx.theme().cyan } else { cx.theme().magenta }
}

pub(super) fn kind_color(kind: DiffKind, cx: &App) -> Hsla {
    match kind {
        DiffKind::OnlyLeft => side_color(0, cx),
        DiffKind::OnlyRight => side_color(1, cx),
        DiffKind::Different => cx.theme().warning,
        DiffKind::Minor => cx.theme().muted_foreground,
        DiffKind::MultipleMatches => cx.theme().danger,
    }
}

pub(super) fn kind_label(kind: DiffKind) -> &'static str {
    match kind {
        DiffKind::OnlyLeft => "Left only",
        DiffKind::OnlyRight => "Right only",
        DiffKind::Different => "Different",
        DiffKind::Minor => "Minor",
        DiffKind::MultipleMatches => "Multiple matches",
    }
}

/// The 6px marker that names a side or a difference kind everywhere in the tab.
pub(super) fn dot(color: Hsla) -> Div {
    div().size(px(6.0)).flex_shrink_0().rounded_full().bg(color)
}

pub(super) fn note(text: impl Into<SharedString>, cx: &App) -> Div {
    div().text_xs().text_color(cx.theme().muted_foreground).child(text.into())
}

/// Lucide icons outside the toolkit's default set live in `assets/icons`.
pub(crate) fn app_icon(name: &str) -> Icon {
    Icon::new(IconName::File).path(format!("icons/{name}.svg"))
}

pub(super) fn endpoint_label(app: &AppState, endpoint: &CompareEndpoint) -> String {
    format!(
        "{} · {}",
        endpoint
            .connection_id
            .and_then(|id| app.connection_name(id))
            .unwrap_or_else(|| "Connection".into()),
        if endpoint.collection.is_empty() {
            endpoint.database.clone()
        } else {
            endpoint.namespace()
        }
    )
}

/// Open one side's collection in a new tab, filtered. Does nothing once that connection closed.
pub(super) fn open_side(
    state: &Entity<AppState>,
    endpoint: &CompareEndpoint,
    filter: &mongodb::bson::Document,
    cx: &mut App,
) {
    state.update(cx, |app, cx| {
        if !endpoint.connection_id.is_some_and(|id| app.is_connected(id)) {
            return;
        }
        app.select_connection(endpoint.connection_id, cx);
        app.open_collection_in_new_tab(
            endpoint.database.clone(),
            endpoint.collection.clone(),
            crate::state::relations::filter_text(filter),
            Some(filter.clone()),
            cx,
        );
    });
}

pub(super) fn relative_time(at: mongodb::bson::DateTime) -> String {
    let seconds = (chrono::Utc::now().timestamp_millis() - at.timestamp_millis()).max(0) / 1_000;
    if seconds < 60 {
        "just now".into()
    } else if seconds < 3_600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3_600)
    } else {
        crate::bson::format_datetime_displayed(at)
    }
}

pub(super) fn format_elapsed(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if seconds < 60.0 {
        format!("{seconds:.2} s")
    } else {
        format!("{}m {:02}s", elapsed.as_secs() / 60, elapsed.as_secs() % 60)
    }
}

pub(super) fn run_shortcut(window: &Window) -> Keystroke {
    crate::keyboard::display_keystroke(&window.bindings_for_action(&RunCompare))
        .unwrap_or_else(|| Keystroke::parse("cmd-enter").unwrap())
}

impl CompareView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        let connection_subscription = cx.subscribe(&state, |view, _, event, cx| {
            use crate::state::AppEvent;
            match event {
                AppEvent::Connecting(id) => {
                    view.connecting.insert(*id);
                    view.connect_errors.remove(id);
                }
                AppEvent::ConnectionFailed { connection_id, error } => {
                    view.connecting.remove(connection_id);
                    view.connect_errors.insert(*connection_id, error.clone());
                }
                AppEvent::Connected(id) | AppEvent::Disconnected(id) => {
                    view.connecting.remove(id);
                    view.connect_errors.remove(id);
                    view.metadata_requested = [None, None];
                    view.collections_requested.retain(|(connection, _)| connection != id);
                }
                _ => return,
            }
            cx.notify();
        });
        Self {
            state,
            focus: cx.focus_handle(),
            detail_focus: cx.focus_handle(),
            controls: None,
            active: None,
            options_open: false,
            auto_right: false,
            metadata_requested: [None, None],
            collections_requested: Default::default(),
            connecting: Default::default(),
            connect_errors: Default::default(),
            scroll: UniformListScrollHandle::new(),
            diff: None,
            detail_signature: None,
            find_error: None,
            _subscriptions: vec![subscription, connection_subscription],
            control_subscriptions: Vec::new(),
        }
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn move_selection(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.active else {
            return;
        };
        if self.database_results(id, cx) {
            let target = {
                let Some(tab) = self.state.read(cx).compare_tab(id) else {
                    return;
                };
                let visible = tab.visible_pairs();
                if visible.is_empty() {
                    return;
                }
                let at = tab.pair_selected.and_then(|pair| visible.iter().position(|i| *i == pair));
                let next =
                    at.map_or(0, |at| at.saturating_add_signed(delta).min(visible.len() - 1));
                (next, visible[next])
            };
            self.scroll.scroll_to_item(target.0, ScrollStrategy::Nearest);
            self.state.update(cx, |app, cx| {
                if let Some(tab) = app.compare_tab_mut(id) {
                    tab.pair_selected = Some(target.1);
                }
                cx.notify();
            });
            window.focus(&self.focus, cx);
            return;
        }
        let target = {
            let app = self.state.read(cx);
            let Some(tab) = app.compare_tab(id) else {
                return;
            };
            let visible = tab.visible();
            if visible.is_empty() {
                return;
            }
            let at = tab.selected.and_then(|row| visible.iter().position(|i| *i == row));
            let next = at.map_or(0, |at| at.saturating_add_signed(delta).min(visible.len() - 1));
            (next, visible[next])
        };
        self.scroll.scroll_to_item(target.0, ScrollStrategy::Nearest);
        AppCommands::select_compare_row(self.state.clone(), id, target.1, cx);
        window.focus(&self.focus, cx);
    }

    fn find(&mut self, input: &Entity<InputState>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.active else {
            return;
        };
        let text = input.read(cx).value().to_string();
        if self.database_results(id, cx) {
            let found = self.state.read(cx).compare_tab(id).and_then(|tab| tab.find_pair(&text));
            self.find_error = found.is_none().then(|| "No collection by that name".into());
            if let Some(index) = found {
                let position = self.state.update(cx, |app, cx| {
                    let tab = app.compare_tab_mut(id).unwrap();
                    if !tab.visible_pairs().contains(&index) {
                        tab.pair_segment = 0;
                    }
                    tab.pair_selected = Some(index);
                    cx.notify();
                    tab.visible_pairs().iter().position(|i| *i == index).unwrap()
                });
                self.scroll.scroll_to_item(position, ScrollStrategy::Nearest);
                window.focus(&self.focus, cx);
            }
            cx.notify();
            return;
        }
        let found = self.state.read(cx).compare_tab(id).and_then(|tab| tab.find_key(&text));
        self.find_error =
            if found.is_none() { Some("Not among the differences".into()) } else { None };
        if let Some(index) = found {
            let position = self.state.update(cx, |app, cx| {
                let tab = app.compare_tab_mut(id).unwrap();
                if !tab.visible().contains(&index) {
                    tab.segment = crate::state::compare::segment_for(tab.rows[index].kind);
                }
                let position = tab.visible().iter().position(|i| *i == index).unwrap();
                cx.notify();
                position
            });
            self.scroll.scroll_to_item(position, ScrollStrategy::Nearest);
            AppCommands::select_compare_row(self.state.clone(), id, index, cx);
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// Whether the results on screen came from a database comparison.
    fn database_results(&self, id: Uuid, cx: &App) -> bool {
        self.state
            .read(cx)
            .compare_tab(id)
            .is_some_and(|tab| tab.results_config().scope == CompareScope::Databases)
    }

    /// Before the first run the tab explains itself instead of showing empty panes.
    fn render_empty(&self, id: Uuid, window: &Window, cx: &Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let databases = self.database_results(id, cx);
        let (title, text) = if databases {
            (
                "Compare two databases",
                "Choose a connection and database on each side. Collections are paired by name, and you can open any of them to compare its documents.",
            )
        } else {
            (
                "Compare two collections",
                "Choose a connection, database and collection on each side. Differences stream in while the scan runs; afterwards you can sync either way.",
            )
        };
        div()
            .debug_selector(|| "compare-empty".into())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(spacing::sm())
            .p(spacing::lg())
            .child(app_icon("git-compare-arrows").size(px(28.0)).text_color(muted))
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
            .child(div().text_xs().text_color(muted).text_center().max_w(px(400.0)).child(text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .text_xs()
                    .text_color(muted)
                    .child("Press")
                    .child(Kbd::new(run_shortcut(window)))
                    .child("to start"),
            )
            .into_any_element()
    }
}

impl Render for CompareView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(id) = self.state.read(cx).active_compare_tab_id() else {
            return div().into_any_element();
        };
        self.ensure_controls(id, window, cx);
        self.sync_controls(id, window, cx);
        let (running, has_results, appearance) = {
            let app = self.state.read(cx);
            let tab = app.compare_tab(id).unwrap();
            (
                tab.running || tab.sync.running,
                tab.compared.is_some(),
                app.settings.appearance.clone(),
            )
        };
        let header = self.render_setup(id, window, cx);
        let databases = self.database_results(id, cx);
        let body = if has_results {
            let (summary, list, detail) = if databases {
                (
                    self.render_database_summary(id, cx),
                    self.render_database_list(id, cx),
                    self.render_database_detail(id, cx),
                )
            } else {
                (
                    self.render_summary(id, cx),
                    self.render_results(id, cx),
                    self.render_detail(id, cx),
                )
            };
            // The collection list carries two counts and a result, so it starts wider.
            let (split, size) = if databases {
                ("compare-database-split", 560.0)
            } else {
                ("compare-split", 320.0)
            };
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(summary)
                .child(
                    div().flex_1().min_h_0().min_w_0().overflow_hidden().child(
                        h_resizable(split)
                            .child(
                                resizable_panel()
                                    .size(px(size))
                                    .size_range(px(120.0)..px(650.0))
                                    .child(list),
                            )
                            .child(
                                resizable_panel().size_range(px(260.0)..Pixels::MAX).child(detail),
                            ),
                    ),
                )
                .into_any_element()
        } else {
            self.render_empty(id, window, cx)
        };
        let sync_bar =
            if databases { div().into_any_element() } else { self.render_sync_bar(id, cx) };
        div()
            .id("compare-view")
            .debug_selector(|| "compare-view".into())
            .key_context(if running { "Compare CompareRunning" } else { "Compare" })
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .bg(islands::content_bg(&appearance, cx))
            .overflow_hidden()
            .on_action(cx.listener(move |this, _: &RunCompare, _, cx| {
                this.options_open = false;
                AppCommands::run_compare(this.state.clone(), id, cx);
                cx.notify();
            }))
            .on_action(cx.listener(move |this, _: &CancelCompare, _, cx| {
                AppCommands::cancel_compare(&this.state, id, cx);
                AppCommands::cancel_compare_sync(&this.state, id, cx);
            }))
            .on_action(
                cx.listener(|this, _: &CompareNext, window, cx| this.move_selection(1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ComparePrevious, window, cx| {
                this.move_selection(-1, window, cx)
            }))
            .on_action(cx.listener(move |this, _: &FocusCompareDetail, window, cx| {
                // In the database scope the detail has nothing to focus; enter opens the row.
                if this.database_results(id, cx) {
                    if let Some(index) =
                        this.state.read(cx).compare_tab(id).and_then(|tab| tab.pair_selected)
                    {
                        database::open_pair(&this.state, id, index, cx);
                    }
                } else {
                    window.focus(&this.detail_focus, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &FindInCompare, window, cx| {
                if let Some(controls) = &this.controls {
                    window.focus(&controls.find.read(cx).focus_handle(cx), cx);
                }
            }))
            .on_action(cx.listener(
                move |this, _: &crate::keyboard::ToggleCompareSelection, _, cx| {
                    this.state.update(cx, |app, cx| {
                        if let Some(tab) = app.compare_tab_mut(id)
                            && let Some(row) = tab.selected
                        {
                            tab.select_sync_row(row, false, true);
                        }
                        cx.notify();
                    });
                },
            ))
            .on_action(cx.listener(
                move |this, _: &crate::keyboard::SelectCompareSegment, _, cx| {
                    this.state.update(cx, |app, cx| {
                        if let Some(tab) = app.compare_tab_mut(id) {
                            for category in 0..4 {
                                if tab.segment == category + 1 || (tab.segment == 0 && category < 3)
                                {
                                    tab.sync.set_category(category, true);
                                }
                            }
                        }
                        cx.notify();
                    });
                },
            ))
            .on_action(cx.listener(move |this, _: &crate::keyboard::ClearCompareTarget, _, cx| {
                let cleared = this.state.update(cx, |app, cx| {
                    let tab = app.compare_tab_mut(id);
                    let cleared = tab.as_ref().is_some_and(|tab| tab.sync.target.is_some());
                    if let Some(tab) = tab {
                        tab.sync.clear_target();
                    }
                    cx.notify();
                    cleared
                });
                if !cleared {
                    cx.propagate();
                }
            }))
            .child(header)
            .child(body)
            .child(sync_bar)
            .into_any_element()
    }
}
