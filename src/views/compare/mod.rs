//! Compare is a native data workbench: setup above, differences and document detail below.
//! It inherits the app's controls, theme and keyboard conventions.
//!
//! Visual system: each side has a fixed hue (Left cyan, Right magenta) that marks its column,
//! its picker and every row that exists on that side only. Changed values use the warning tint.
//! The same dot appears wherever a kind or a side is named, so the legend is always on screen.

mod detail;
mod detail_tree;
mod results;
mod setup;
mod sync_bar;
#[cfg(test)]
mod tests;

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
use crate::state::compare::CompareEndpoint;
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
    collections_requested: std::collections::HashSet<(Uuid, String)>,
    scroll: UniformListScrollHandle,
    detail_scroll: UniformListScrollHandle,
    detail_rows: Vec<detail::DetailRow>,
    detail_signature: Option<(Uuid, u64, usize, usize)>,
    expansion: detail_tree::Expansion,
    tree_error: Option<String>,
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
pub(super) fn app_icon(name: &str) -> Icon {
    Icon::new(IconName::File).path(format!("icons/{name}.svg"))
}

pub(super) fn endpoint_label(app: &AppState, endpoint: &CompareEndpoint) -> String {
    format!(
        "{} · {}",
        endpoint
            .connection_id
            .and_then(|id| app.connection_name(id))
            .unwrap_or_else(|| "Connection".into()),
        endpoint.namespace()
    )
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
            if let crate::state::AppEvent::Connected(id)
            | crate::state::AppEvent::Disconnected(id) = event
            {
                view.metadata_requested = [None, None];
                view.collections_requested.retain(|(connection, _)| connection != id);
                cx.notify();
            }
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
            scroll: UniformListScrollHandle::new(),
            detail_scroll: UniformListScrollHandle::new(),
            detail_rows: Vec::new(),
            detail_signature: None,
            expansion: Default::default(),
            tree_error: None,
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

    /// Before the first run the tab explains itself instead of showing empty panes.
    fn render_empty(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
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
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child("Compare two collections"))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .text_center()
                    .max_w(px(400.0))
                    .child("Choose a connection, database and collection on each side. Differences stream in while the scan runs; afterwards you can sync either way."),
            )
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
        let body = if has_results {
            let summary = self.render_summary(id, cx);
            let list = self.render_results(id, cx);
            let detail = self.render_detail(id, cx);
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
                        h_resizable("compare-split")
                            .child(
                                resizable_panel()
                                    .size(px(320.0))
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
            self.render_empty(window, cx)
        };
        let sync_bar = self.render_sync_bar(id, cx);
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
            .on_action(cx.listener(|this, _: &FocusCompareDetail, window, cx| {
                window.focus(&this.detail_focus, cx)
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
