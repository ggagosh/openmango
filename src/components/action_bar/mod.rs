mod providers;
#[cfg(test)]
mod tests;
mod types;

pub use types::ActionExecution;

use std::rc::Rc;

use gpui_kit::base::actions::Cancel;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::input::Backspace;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{
    ActiveTheme as _, FocusTrapElement as _, Icon, IconName, IndexPath, Sizable as _, h_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::search::fuzzy_match_score;
use crate::components::connection_identity_tags;
use crate::state::AppState;
use crate::state::settings::AppTheme;
use crate::theme::{colors, fonts, islands, sizing, spacing};

use providers::{
    command_actions, connection_switcher_actions, disconnect_actions, navigation_actions,
    tab_actions, theme_actions, view_actions,
};
use types::{ActionCategory, ActionItem, PaletteMode};

type ExecuteHandler = Box<dyn Fn(ActionExecution, &mut Window, &mut App) + 'static>;

/// Rows beyond this stay hidden behind a "keep typing" footer; the list measures every row it gets.
const MAX_RESULTS: usize = 100;
const MAX_RECENT: usize = 5;

struct PaletteGroup {
    label: Option<&'static str>,
    items: Vec<ActionItem>,
}

pub struct ActionBar {
    state: Entity<AppState>,
    mode: PaletteMode,
    /// Present while the palette is open; replaced on every mode switch for a fresh query.
    command: Option<Entity<CommandState>>,
    trap: FocusHandle,
    original_theme: Option<AppTheme>,
    previous_focus: Option<FocusHandle>,
    actions: Vec<ActionItem>,
    groups: Rc<Vec<PaletteGroup>>,
    hidden: usize,
    recent: Vec<SharedString>,
    on_execute: Option<ExecuteHandler>,
}

impl ActionBar {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        Self {
            state,
            mode: PaletteMode::default(),
            command: None,
            trap: cx.focus_handle(),
            original_theme: None,
            previous_focus: None,
            actions: Vec::new(),
            groups: Rc::default(),
            hidden: 0,
            recent: Vec::new(),
            on_execute: None,
        }
    }

    pub fn on_execute(
        mut self,
        handler: impl Fn(ActionExecution, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_execute = Some(Box::new(handler));
        self
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.command.is_some() {
            self.close(window, cx);
        } else {
            self.show(PaletteMode::All, "", window, cx);
        }
    }

    /// Opens the palette straight into the connection switcher, or closes it when the
    /// switcher is already showing.
    pub fn toggle_connections(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.command.is_some() && self.mode == PaletteMode::Connect {
            self.close(window, cx);
        } else {
            self.show(PaletteMode::Connect, "", window, cx);
        }
    }

    /// Opens the palette in `mode`, or switches the open palette to it, searching for `query`.
    fn show(
        &mut self,
        mode: PaletteMode,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.command.is_none() {
            self.previous_focus = window.focused(cx);
        }
        if mode != PaletteMode::Theme {
            self.revert_theme_preview(window, cx);
        } else if self.original_theme.is_none() {
            self.original_theme = Some(self.state.read(cx).settings.appearance.theme);
        }

        self.mode = mode;
        let state = self.state.read(cx);
        self.actions = match mode {
            PaletteMode::All => {
                let mut actions = tab_actions(state);
                actions.extend(command_actions(state, window));
                actions.extend(navigation_actions(state));
                actions.extend(view_actions(state, window));
                actions
            }
            PaletteMode::Theme => theme_actions(state),
            PaletteMode::Connect => connection_switcher_actions(state, window),
            PaletteMode::Disconnect => disconnect_actions(state),
            PaletteMode::Navigate => navigation_actions(state),
        };
        self.set_query(query);

        let command = cx.new(|cx| {
            let mut command = CommandState::new(window, cx);
            // "@prod" typed or pasted in one go keeps "prod" in the scoped field.
            command.set_query(query.to_string(), window, cx);
            command
        });
        cx.defer_in(window, {
            let command = command.clone();
            move |_, window, cx| command.update(cx, |command, cx| command.focus(window, cx))
        });
        self.command = Some(command);
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.revert_theme_preview(window, cx);
        self.command = None;
        self.mode = PaletteMode::All;
        self.actions.clear();
        self.groups = Rc::default();
        self.hidden = 0;
        if let Some(previous_focus) = self.previous_focus.take() {
            window.focus(&previous_focus, cx);
        }
        cx.notify();
    }

    /// A leading `#` or `@` in the command search moves to that scope.
    fn search(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        match PaletteMode::from_prefix(query) {
            Some((mode, rest)) if self.mode == PaletteMode::All => {
                self.show(mode, rest, window, cx)
            }
            _ => {
                self.set_query(query);
                cx.notify();
            }
        }
    }

    fn set_query(&mut self, query: &str) {
        let (groups, hidden) = build_groups(&self.actions, self.mode, query, &self.recent);
        self.groups = Rc::new(groups);
        self.hidden = hidden;
    }

    fn revert_theme_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(original) = self.original_theme.take() {
            apply_theme(&self.state, original, window, cx);
        }
    }

    fn preview(&mut self, item: ActionItem, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(theme) = item.id.strip_prefix("theme:").and_then(AppTheme::from_theme_id) {
            apply_theme(&self.state, theme, window, cx);
        }
    }

    fn confirm(&mut self, item: ActionItem, window: &mut Window, cx: &mut Context<Self>) {
        self.remember(&item.id);
        let submenu = match item.id.as_ref() {
            "cmd:change-theme" => Some(PaletteMode::Theme),
            "cmd:connect" => Some(PaletteMode::Connect),
            "cmd:disconnect" => Some(PaletteMode::Disconnect),
            _ => None,
        };
        if let Some(mode) = submenu {
            self.show(mode, "", window, cx);
            return;
        }

        // A confirmed theme stays applied.
        if self.mode == PaletteMode::Theme {
            self.original_theme = None;
        }
        self.close(window, cx);
        if let Some(handler) = &self.on_execute {
            handler(ActionExecution { action_id: item.id }, window, cx);
        }
    }

    // ponytail: session-only recents; persist them in settings if they should survive restarts.
    fn remember(&mut self, id: &SharedString) {
        // Tab ids are positions, which point elsewhere once tabs move.
        if id.starts_with("tab:") {
            return;
        }
        self.recent.retain(|recent| recent != id);
        self.recent.insert(0, id.clone());
        self.recent.truncate(MAX_RECENT);
    }
}

fn apply_theme(state: &Entity<AppState>, theme: AppTheme, window: &mut Window, cx: &mut App) {
    let vibrancy =
        crate::theme::effective_vibrancy(theme, state.read(cx).settings.appearance.vibrancy);
    crate::theme::apply_theme(theme, vibrancy, window, cx);
}

/// Filters, ranks and groups the actions for one palette view, and returns how many
/// matches were left out by [`MAX_RESULTS`].
fn build_groups(
    actions: &[ActionItem],
    mode: PaletteMode,
    query: &str,
    recent: &[SharedString],
) -> (Vec<PaletteGroup>, usize) {
    let query = query.trim();
    let mut matches = actions
        .iter()
        .filter(|item| item.available)
        .filter_map(|item| {
            if query.is_empty() {
                Some((0, item))
            } else {
                fuzzy_match_score(query, &item.search_text()).map(|score| (score, item))
            }
        })
        .collect::<Vec<_>>();
    matches.sort_by(|(a_score, a), (b_score, b)| {
        b.highlighted
            .cmp(&a.highlighted)
            .then(a_score.cmp(b_score))
            .then(a.priority.cmp(&b.priority))
    });

    let mut groups = Vec::new();
    if mode == PaletteMode::All && query.is_empty() {
        let items = recent
            .iter()
            .filter_map(|id| matches.iter().find(|(_, item)| &item.id == id))
            .map(|(_, item)| (*item).clone())
            .collect::<Vec<_>>();
        if !items.is_empty() {
            matches.retain(|(_, item)| !recent.contains(&item.id));
            groups.push(PaletteGroup { label: Some("Recent"), items });
        }
    }

    // Groups follow their best match, so the first row is the best result.
    let mut categories = Vec::<ActionCategory>::new();
    for (_, item) in &matches {
        if !categories.contains(&item.category) {
            categories.push(item.category);
        }
    }
    if query.is_empty() || mode == PaletteMode::Connect {
        let highlighted = |category: &ActionCategory| {
            matches.iter().any(|(_, item)| item.highlighted && item.category == *category)
        };
        categories.sort_by_key(|category| (!highlighted(category), category.sort_order()));
    }
    for category in categories {
        let items = matches
            .iter()
            .filter(|(_, item)| item.category == category)
            .map(|(_, item)| (*item).clone())
            .collect();
        let label = match (mode, category) {
            // Switcher actions and disconnect targets sit under a divider, not a heading.
            (PaletteMode::Connect | PaletteMode::Disconnect, ActionCategory::Command) => None,
            (PaletteMode::Navigate, _) => None,
            _ => Some(category.label()),
        };
        groups.push(PaletteGroup { label, items });
    }

    let mut remaining = MAX_RESULTS;
    let mut hidden = 0;
    for group in &mut groups {
        hidden += group.items.len().saturating_sub(remaining);
        group.items.truncate(remaining);
        remaining -= group.items.len();
    }
    groups.retain(|group| !group.items.is_empty());
    (groups, hidden)
}

/// Says how many results the cap hid, and teaches the scope prefixes on an empty search.
fn footer(mode: PaletteMode, hidden: usize, query_empty: bool, cx: &App) -> AnyElement {
    let hint = mode == PaletteMode::All && query_empty;
    let mut text = String::new();
    if hidden > 0 {
        let noun = if hidden == 1 { "result" } else { "results" };
        text = format!("{hidden} more {noun}. ");
        if !hint {
            text.push_str("Keep typing to narrow the list.");
        }
    }
    if hint {
        text.push_str("Type # to search databases and collections, or @ for connections.");
    }
    if text.is_empty() {
        return div().into_any_element();
    }
    div()
        .px(spacing::md())
        .py(spacing::sm())
        .border_t_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

fn item_at(groups: &[PaletteGroup], index: IndexPath) -> Option<ActionItem> {
    groups.get(index.section)?.items.get(index.row).cloned()
}

fn command_item(item: &ActionItem) -> CommandItem {
    let row = item.clone();
    CommandItem::new()
        .label(item.label.clone())
        .checked(item.checked)
        .child(move |_, cx| row_content(&row, cx))
}

fn row_content(item: &ActionItem, cx: &App) -> Div {
    let muted = cx.theme().muted_foreground;
    h_flex()
        .flex_1()
        .min_w_0()
        .gap(spacing::sm())
        .when_some(item.connection.as_ref(), |row, identity| {
            let color = identity.color.map(|color| colors::connection_accent(color, cx));
            row.child(
                Icon::new(IconName::Globe)
                    .size(sizing::icon_md())
                    .text_color(color.unwrap_or(muted)),
            )
        })
        .child(
            div()
                .flex_none()
                .max_w(relative(0.7))
                .truncate()
                .when(item.highlighted, |label| label.text_color(cx.theme().primary))
                .child(item.label.clone()),
        )
        .when_some(item.connection.as_ref(), |row, identity| {
            row.child(connection_identity_tags(identity, cx))
        })
        .when_some(item.detail.clone(), |row, detail| {
            row.child(div().min_w_0().truncate().text_xs().text_color(muted).child(detail))
        })
        .child(div().flex_1())
        .when_some(item.shortcut.clone(), |row, keystroke| row.child(Kbd::new(keystroke)))
}

impl Render for ActionBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(command) = self.command.clone() else {
            return div().into_any_element();
        };

        let appearance = self.state.read(cx).settings.appearance.clone();
        let bar = cx.entity().downgrade();
        let mode = self.mode;
        let groups = self.groups.clone();
        let hidden = self.hidden;
        // Short windows shrink the list instead of pushing the palette off screen.
        let list_max_h = (window.viewport_size().height - px(240.0)).max(px(120.0)).min(px(420.0));

        let mut palette = Command::new(&command)
            .bordered(false)
            .filterable(false)
            .placeholder(mode.placeholder())
            .max_h(list_max_h)
            .bg(islands::card_bg(&appearance, cx))
            .on_query({
                let bar = bar.clone();
                move |query, window, cx| {
                    _ = bar.update(cx, |bar, cx| bar.search(query, window, cx));
                }
            })
            // Callbacks read the groups this render installed, which the index paths address.
            .on_select({
                let (bar, groups) = (bar.clone(), groups.clone());
                move |index, window, cx| {
                    if let Some(item) = item_at(&groups, index) {
                        _ = bar.update(cx, |bar, cx| bar.preview(item, window, cx));
                    }
                }
            })
            .on_confirm({
                let (bar, groups) = (bar.clone(), groups.clone());
                move |index, window, cx| {
                    if let Some(item) = item_at(&groups, index) {
                        _ = bar.update(cx, |bar, cx| bar.confirm(item, window, cx));
                    }
                }
            })
            .empty(move |state, _, cx| {
                let query = state.query(cx);
                let message = if query.is_empty() && mode == PaletteMode::Navigate {
                    "Open a connection to search its databases and collections.".to_string()
                } else {
                    format!("No results for “{query}”")
                };
                div()
                    .py_6()
                    .px(spacing::md())
                    .w_full()
                    .text_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(message)
            });

        if mode != PaletteMode::All {
            palette = palette.header(move |_, _, _| {
                let bar = bar.clone();
                h_flex().px(spacing::sm()).pt(spacing::sm()).child(
                    Button::new("action-bar-back")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::ChevronLeft).xsmall())
                        .label(mode.title())
                        .tooltip("Back to all commands (Backspace)")
                        .on_click(move |_, window, cx| {
                            _ = bar
                                .update(cx, |bar, cx| bar.show(PaletteMode::All, "", window, cx));
                        }),
                )
            });
        }
        if hidden > 0 || mode == PaletteMode::All {
            palette = palette
                .footer(move |state, _, cx| footer(mode, hidden, state.query(cx).is_empty(), cx));
        }
        for (ix, group) in groups.iter().enumerate() {
            if ix > 0 && group.label.is_none() {
                palette = palette.separator();
            }
            let entry = CommandGroup::new().items(group.items.iter().map(command_item));
            palette = palette.group(match group.label {
                Some(label) => entry.label(label),
                None => entry,
            });
        }

        div()
            .id("action-bar")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(60.0))
            .px(spacing::lg())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|bar, _, window, cx| bar.close(window, cx)),
            )
            .child(
                div()
                    .debug_selector(|| "action-bar-card".into())
                    .w_full()
                    .max_w(px(620.0))
                    .bg(islands::card_bg(&appearance, cx))
                    .border_1()
                    .border_color(islands::panel_border(&appearance, cx))
                    .rounded(islands::radius_md(&appearance))
                    .shadow_lg()
                    .overflow_hidden()
                    .text_color(cx.theme().foreground)
                    .font_family(fonts::ui())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    // Command clears a non-empty query on Escape and passes the rest up.
                    .on_action(cx.listener(|bar, _: &Cancel, window, cx| bar.close(window, cx)))
                    // The query field passes Backspace up when there is nothing left to delete.
                    .on_action(cx.listener(|bar, _: &Backspace, window, cx| {
                        let empty = bar
                            .command
                            .as_ref()
                            .is_some_and(|command| command.read(cx).query(cx).is_empty());
                        if bar.mode != PaletteMode::All && empty {
                            bar.show(PaletteMode::All, "", window, cx);
                        } else {
                            cx.propagate();
                        }
                    }))
                    .child(palette)
                    .focus_trap("action-bar-trap", &self.trap),
            )
            .into_any_element()
    }
}
