mod providers;
#[cfg(test)]
mod tests;
mod types;

pub use types::ActionExecution;

use std::rc::Rc;

use gpui_kit::base::actions::Cancel;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::input::{Backspace, Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{
    ActiveTheme as _, FocusTrapElement as _, Icon, IconName, IndexPath, Sizable as _, h_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::search::{fuzzy_match_score, label_match_ranges};
use crate::components::connection_identity_tags;
use crate::state::AppState;
use crate::state::settings::AppTheme;
use crate::theme::{colors, fonts, islands, sizing, spacing};

use providers::{
    command_actions, connection_switcher_actions, disconnect_actions, navigation_actions,
    tab_actions, theme_actions, view_actions,
};
use types::{ActionCategory, ActionItem, NavScope, PaletteMode};

type ExecuteHandler = Box<dyn Fn(ActionExecution, &mut Window, &mut App) + 'static>;

/// Rows beyond this stay hidden behind a "keep typing" footer; the list measures every row it gets.
const MAX_RESULTS: usize = 100;
const MAX_RECENT: usize = 5;

struct PaletteGroup {
    label: Option<&'static str>,
    items: Vec<ActionItem>,
}

/// A level of the palette to come back to, as it was left.
#[derive(Clone)]
struct Page {
    mode: PaletteMode,
    scope: NavScope,
    query: SharedString,
    /// The highlighted row, highlighted again on return.
    selected: Option<SharedString>,
}

impl Page {
    fn root() -> Self {
        Self {
            mode: PaletteMode::All,
            scope: NavScope::default(),
            query: SharedString::default(),
            selected: None,
        }
    }
}

/// The tag a level shows in front of the query.
#[derive(Clone)]
struct Tag {
    label: SharedString,
    icon: Option<(IconName, Hsla)>,
}

pub struct ActionBar {
    state: Entity<AppState>,
    mode: PaletteMode,
    /// What Go to is narrowed to.
    scope: NavScope,
    /// The levels behind this one, outermost first. Backspace on an empty query and Escape
    /// return to the last.
    back: Vec<Page>,
    /// The search field, with a tag per level in front of the query. Present while open.
    input: Option<Entity<InputState>>,
    /// Present while the palette is open; replaced for each level so its highlight starts on
    /// the first row.
    command: Option<Entity<CommandState>>,
    /// The row to highlight once a level returned to is drawn.
    reselect: Option<SharedString>,
    trap: FocusHandle,
    original_theme: Option<AppTheme>,
    previous_focus: Option<FocusHandle>,
    actions: Vec<ActionItem>,
    groups: Rc<Vec<PaletteGroup>>,
    hidden: usize,
    /// What the rows were filtered by, so each can paint the letters that matched.
    query: SharedString,
    recent: Vec<SharedString>,
    on_execute: Option<ExecuteHandler>,
    _keys: Subscription,
    _input_changes: Option<Subscription>,
}

impl ActionBar {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let bar = cx.entity().downgrade();
        // Tab, or → at the end of the query, goes into the highlighted connection or database.
        // Intercepted, so it runs before the focus trap or the field moves anything.
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            let key = &event.keystroke;
            if key.modifiers.modified() || !matches!(key.key.as_str(), "tab" | "right") {
                return;
            }
            let Some(bar) = bar.upgrade() else { return };
            if !bar.read(cx).trap.contains_focused(window, cx)
                || (key.key == "right" && !bar.read(cx).caret_at_end(cx))
            {
                return;
            }
            if bar.update(cx, |bar, cx| bar.narrow(window, cx)) {
                cx.stop_propagation();
            }
        });
        Self {
            state,
            mode: PaletteMode::default(),
            scope: NavScope::default(),
            back: Vec::new(),
            input: None,
            command: None,
            reselect: None,
            trap: cx.focus_handle(),
            original_theme: None,
            previous_focus: None,
            actions: Vec::new(),
            groups: Rc::default(),
            hidden: 0,
            query: SharedString::default(),
            recent: Vec::new(),
            on_execute: None,
            _keys: keys,
            _input_changes: None,
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
            self.open(Page::root(), window, cx);
        }
    }

    /// Opens the palette straight into the connection switcher, or closes it when the
    /// switcher is already showing.
    pub fn toggle_connections(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.command.is_some() && self.mode == PaletteMode::Connect {
            self.close(window, cx);
        } else {
            self.back.clear();
            self.open(Page { mode: PaletteMode::Connect, ..Page::root() }, window, cx);
        }
    }

    /// Shows `page`, opening the palette if it is closed. The levels behind it are left as
    /// they are.
    fn open(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.command.is_none() {
            self.previous_focus = window.focused(cx);
        }
        if page.mode != PaletteMode::Theme {
            self.revert_theme_preview(cx);
        } else if self.original_theme.is_none() {
            self.original_theme = Some(self.state.read(cx).settings.appearance.theme);
        }

        self.mode = page.mode;
        self.scope = page.scope;
        self.reselect = page.selected;
        let state = self.state.read(cx);
        self.actions = match self.mode {
            PaletteMode::All => {
                let mut actions = tab_actions(state);
                actions.extend(command_actions(state, window));
                actions.extend(navigation_actions(state, &NavScope::default()));
                actions.extend(view_actions(state, window));
                actions
            }
            PaletteMode::Theme => theme_actions(state),
            PaletteMode::Connect => connection_switcher_actions(state, window),
            PaletteMode::Disconnect => disconnect_actions(state),
            PaletteMode::Navigate => navigation_actions(state, &self.scope),
        };

        let placeholder =
            self.scope.placeholder().unwrap_or_else(|| self.mode.placeholder().to_string());
        let input = self.input(window, cx);
        input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
            // A programmatic write sends no change event.
            input.set_value(page.query.clone(), window, cx);
        });
        self.command = Some(cx.new(|cx| CommandState::new(window, cx)));
        self.apply_query(&page.query, window, cx);
        cx.notify();
    }

    /// The search field, created and focused when the palette opens.
    fn input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(input) = &self.input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx));
        self._input_changes = Some(cx.subscribe_in(&input, window, {
            move |bar, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value();
                    bar.search(&query, window, cx);
                }
            }
        }));
        cx.defer_in(window, {
            let input = input.clone();
            move |_, window, cx| input.update(cx, |input, cx| input.focus(window, cx))
        });
        self.input = Some(input.clone());
        input
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.revert_theme_preview(cx);
        self.command = None;
        self.input = None;
        self._input_changes = None;
        self.mode = PaletteMode::All;
        self.scope = NavScope::default();
        self.back.clear();
        self.reselect = None;
        self.actions.clear();
        self.groups = Rc::default();
        self.hidden = 0;
        if let Some(previous_focus) = self.previous_focus.take() {
            window.focus(&previous_focus, cx);
        }
        cx.notify();
    }

    /// A leading `#` or `@` in the command search goes to that list, keeping the rest.
    fn search(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        match PaletteMode::from_prefix(query) {
            Some((mode, rest)) if self.mode == PaletteMode::All => {
                let page = Page { mode, query: rest.to_string().into(), ..Page::root() };
                self.push(page, window, cx);
            }
            _ => {
                self.apply_query(query, window, cx);
                cx.notify();
            }
        }
    }

    fn apply_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let query = query.trim();
        // An empty Go to lists what is directly inside its level; typing searches all of it.
        let (groups, hidden) = if self.mode == PaletteMode::Navigate && query.is_empty() {
            let prefix = self.scope.child_prefix();
            let children = self
                .actions
                .iter()
                .filter(|item| item.id.starts_with(prefix))
                .cloned()
                .collect::<Vec<_>>();
            build_groups(&children, self.mode, query, &self.recent)
        } else {
            build_groups(&self.actions, self.mode, query, &self.recent)
        };
        self.groups = Rc::new(groups);
        self.hidden = hidden;
        self.query = query.to_string().into();
        // The list's own query moves its highlight back to the first row, as typing does.
        if let Some(command) = &self.command {
            command.update(cx, |command, cx| command.set_query(query.to_string(), window, cx));
        }
    }

    fn page(&self, cx: &App) -> Page {
        Page {
            mode: self.mode,
            scope: self.scope.clone(),
            query: self.query.clone(),
            selected: self.selected_item(cx).map(|item| item.id),
        }
    }

    /// Goes one level in, keeping this one to come back to.
    fn push(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.back.push(self.page(cx));
        self.open(page, window, cx);
    }

    /// Goes into the highlighted connection or database. False when that row has nothing
    /// inside, so the key does what it would anyway.
    fn narrow(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(item) = self.selected_item(cx) else { return false };
        let state = self.state.read(cx);
        let Some(scope) = NavScope::of(&item.id, |id| state.connection_name(id)) else {
            return false;
        };
        self.back.push(self.page(cx));
        // A database found from outside its connection adds the connection's level too, so
        // each tag is one step back.
        if scope.database.is_some() && self.scope.connection != scope.connection {
            self.back.push(Page {
                mode: PaletteMode::Navigate,
                scope: NavScope { connection: scope.connection.clone(), database: None },
                query: SharedString::default(),
                selected: Some(item.id.clone()),
            });
        }
        self.open(Page { mode: PaletteMode::Navigate, scope, ..Page::root() }, window, cx);
        true
    }

    /// Returns to the level before `level` (the outermost is 0), dropping its tag and every
    /// tag after it.
    fn back_to(&mut self, level: usize, window: &mut Window, cx: &mut Context<Self>) {
        let page = level.checked_sub(1).and_then(|ix| self.back.get(ix).cloned());
        self.back.truncate(level.saturating_sub(1));
        self.open(page.unwrap_or_else(Page::root), window, cx);
    }

    /// Goes back one level. False at the top, where there is nothing to go back to.
    fn step_back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.back.is_empty() && self.mode == PaletteMode::All {
            return false;
        }
        self.back_to(self.back.len(), window, cx);
        true
    }

    fn selected_item(&self, cx: &App) -> Option<ActionItem> {
        let index = self.command.as_ref()?.read(cx).selected_index()?;
        item_at(&self.groups, index)
    }

    fn query_text(&self, cx: &App) -> SharedString {
        self.input.as_ref().map(|input| input.read(cx).value()).unwrap_or_default()
    }

    fn caret_at_end(&self, cx: &App) -> bool {
        self.input.as_ref().is_some_and(|input| {
            let input = input.read(cx);
            input.selected_range().is_empty() && input.cursor() == input.value().len()
        })
    }

    /// One tag per level, each with the level it removes when clicked.
    fn tags(&self, cx: &App) -> Vec<(usize, Tag)> {
        let state = self.state.read(cx);
        self.back
            .iter()
            .map(|page| (page.mode, &page.scope))
            .chain([(self.mode, &self.scope)])
            .enumerate()
            .filter_map(|(level, (mode, scope))| {
                let tag = match (mode, &scope.connection, &scope.database) {
                    (PaletteMode::All, ..) => return None,
                    (PaletteMode::Navigate, _, Some(database)) => Tag {
                        label: database.clone(),
                        icon: Some((IconName::LayoutDashboard, cx.theme().info)),
                    },
                    (PaletteMode::Navigate, Some((id, name)), None) => {
                        let color = state
                            .connection_by_id(*id)
                            .and_then(|connection| connection.color)
                            .map(|color| colors::connection_accent(color, cx));
                        Tag {
                            label: name.clone(),
                            icon: Some((
                                IconName::Globe,
                                color.unwrap_or(cx.theme().muted_foreground),
                            )),
                        }
                    }
                    _ => Tag { label: mode.title().into(), icon: None },
                };
                Some((level, tag))
            })
            .collect()
    }

    fn revert_theme_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(original) = self.original_theme.take() {
            crate::theme::apply_theme(original, cx);
        }
    }

    fn preview(&mut self, item: ActionItem, window: &mut Window, cx: &mut Context<Self>) {
        let theme = match item.id.strip_prefix("theme:") {
            Some("system") => {
                let mut appearance = self.state.read(cx).settings.appearance.clone();
                appearance.follow_system = true;
                Some(crate::theme::resolved_theme(&appearance, window.appearance()))
            }
            Some(id) => AppTheme::from_theme_id(id),
            None => None,
        };
        if let Some(theme) = theme {
            crate::theme::apply_theme(theme, cx);
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
            self.push(Page { mode, ..Page::root() }, window, cx);
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
            (
                PaletteMode::Connect | PaletteMode::Disconnect | PaletteMode::Theme,
                ActionCategory::Command,
            ) => None,
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
/// The footer: on the left, hidden results or a tip; on the right, the keys for the highlighted
/// row.
fn footer(text: Option<SharedString>, keys: Vec<(&'static str, SharedString)>, cx: &App) -> Div {
    h_flex()
        .justify_between()
        .gap(spacing::md())
        .px(spacing::md())
        .py(spacing::xs())
        .border_t_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(div().min_w_0().truncate().children(text))
        .child(h_flex().flex_none().gap(spacing::md()).children(keys.into_iter().filter_map(
            |(key, label)| {
                let key = Keystroke::parse(key).ok()?;
                Some(h_flex().gap(spacing::xs()).child(Kbd::new(key)).child(label))
            },
        )))
}

/// The keys the footer offers for `item`: Enter for its own action, Tab for what is inside it.
fn row_keys(item: &ActionItem) -> Vec<(&'static str, SharedString)> {
    let enter = match item.id.split(':').next() {
        Some("theme") => "Apply",
        Some("connect") => "Connect",
        Some("disconnect") => "Disconnect",
        Some("cmd") => "Run",
        _ => "Open",
    };
    let mut keys = vec![("enter", enter.into())];
    if item.id.starts_with("nav:conn:") {
        keys.push(("tab", "Databases".into()));
    } else if item.id.starts_with("nav:db:") {
        keys.push(("tab", "Collections".into()));
    }
    keys
}

fn item_at(groups: &[PaletteGroup], index: IndexPath) -> Option<ActionItem> {
    groups.get(index.section)?.items.get(index.row).cloned()
}

fn index_of(groups: &[PaletteGroup], id: &str) -> Option<IndexPath> {
    groups.iter().enumerate().find_map(|(section, group)| {
        let row = group.items.iter().position(|item| item.id == id)?;
        Some(IndexPath::default().section(section).row(row))
    })
}

fn command_item(item: &ActionItem, query: &SharedString) -> CommandItem {
    let row = item.clone();
    let matched = label_match_ranges(query, &item.label);
    CommandItem::new()
        .label(item.label.clone())
        .checked(item.checked)
        .child(move |_, cx| row_content(&row, &matched, cx))
}

fn row_content(item: &ActionItem, matched: &[std::ops::Range<usize>], cx: &App) -> Div {
    let muted = cx.theme().muted_foreground;
    let highlight = HighlightStyle { color: Some(cx.theme().blue), ..Default::default() };
    let label = StyledText::new(item.label.clone())
        .with_highlights(matched.iter().map(|range| (range.clone(), highlight)));
    // Databases and collections carry the sidebar's icons, so each level reads the same here.
    let kind_icon = if item.id.starts_with("nav:db:") {
        Some(Icon::new(IconName::LayoutDashboard).text_color(cx.theme().info))
    } else if item.id.starts_with("nav:col:") {
        Some(Icon::new(crate::assets::AppIcon::Braces).text_color(muted))
    } else {
        None
    };
    let has_inside = item.id.starts_with("nav:conn:") || item.id.starts_with("nav:db:");
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
        .when_some(kind_icon, |row, icon| row.child(icon.size(sizing::icon_md())))
        .child(
            div()
                .flex_none()
                .max_w(relative(0.7))
                .truncate()
                .when(item.highlighted, |label| label.text_color(cx.theme().primary))
                .child(label),
        )
        .when_some(item.connection.as_ref(), |row, identity| {
            row.child(connection_identity_tags(identity, cx))
        })
        .when_some(item.detail.clone(), |row, detail| {
            row.child(div().min_w_0().truncate().text_xs().text_color(muted).child(detail))
        })
        .child(div().flex_1())
        .when_some(item.shortcut.clone(), |row, keystroke| row.child(Kbd::new(keystroke)))
        .when(has_inside, |row| {
            row.child(Icon::new(IconName::ChevronRight).xsmall().text_color(muted))
        })
}

/// A level's tag in the search field. Clicking it goes back to before that level.
fn tag_chip(level: usize, tag: Tag, bar: WeakEntity<ActionBar>, cx: &App) -> Stateful<Div> {
    let hover = cx.theme().secondary_hover;
    h_flex()
        .id(("action-bar-tag", level))
        .flex_none()
        .h(px(20.0))
        .max_w(px(160.0))
        .px(px(6.0))
        .gap(px(4.0))
        .rounded(px(4.0))
        .bg(cx.theme().secondary)
        .hover(move |chip| chip.bg(hover))
        .cursor_pointer()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().secondary_foreground)
        .when_some(tag.icon, |chip, (icon, color)| {
            chip.child(Icon::new(icon).size(px(12.0)).text_color(color))
        })
        .child(div().min_w_0().truncate().child(tag.label))
        .on_click(move |_, window, cx| {
            _ = bar.update(cx, |bar, cx| bar.back_to(level, window, cx));
        })
}

impl Render for ActionBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (Some(command), Some(input)) = (self.command.clone(), self.input.clone()) else {
            return div().into_any_element();
        };

        // A level returned to highlights the row it was left on, once its list is installed.
        if let Some(path) = self.reselect.take().and_then(|id| index_of(&self.groups, &id)) {
            let command = command.clone();
            window.on_next_frame(move |window, cx| {
                command
                    .update(cx, |command, cx| command.set_selected_index(Some(path), window, cx));
            });
        }

        let appearance = self.state.read(cx).settings.appearance.clone();
        let bar = cx.entity().downgrade();
        let mode = self.mode;
        let groups = self.groups.clone();
        let hidden = self.hidden;
        let query = self.query.clone();
        let can_go_back = !(self.back.is_empty() && mode == PaletteMode::All);
        let tags = self.tags(cx);
        // Short windows shrink the list instead of pushing the palette off screen.
        let list_max_h = (window.viewport_size().height - px(240.0)).max(px(120.0)).min(px(420.0));

        let mut palette = Command::new(&command)
            .bordered(false)
            .searchable(false)
            .max_h(list_max_h)
            .bg(islands::card_bg(&appearance, cx))
            // The search field is the palette's own, so each level's tag sits in it, before the
            // query. It stays inside the list's key context, so arrows and Enter drive the list.
            .header({
                let bar = bar.clone();
                move |_, _, cx| {
                    let prefix = h_flex()
                        .gap(px(6.0))
                        .mr(px(2.0))
                        .child(Icon::new(IconName::Search).text_color(cx.theme().muted_foreground))
                        .children(
                            tags.iter()
                                .map(|(level, tag)| tag_chip(*level, tag.clone(), bar.clone(), cx)),
                        );
                    div()
                        .flex_none()
                        .px_3()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(Input::new(&input).prefix(prefix).appearance(false).p_0())
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
            .empty({
                let query = query.clone();
                move |_, _, cx| {
                    let message = if query.is_empty() && mode == PaletteMode::Navigate {
                        "Open a connection to search its databases and collections.".to_string()
                    } else if query.is_empty() {
                        "Nothing here yet.".to_string()
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
                }
            })
            .footer({
                let (groups, query) = (groups.clone(), query.clone());
                move |state, _, cx| {
                    let text = if hidden > 0 {
                        let noun = if hidden == 1 { "result" } else { "results" };
                        Some(
                            format!("{hidden} more {noun}. Keep typing to narrow the list.").into(),
                        )
                    } else if mode == PaletteMode::All && query.is_empty() {
                        Some("Type # for databases and collections, @ for connections.".into())
                    } else {
                        None
                    };
                    let mut keys = state
                        .selected_index()
                        .and_then(|index| item_at(&groups, index))
                        .map(|item| row_keys(&item))
                        .unwrap_or_default();
                    if can_go_back && query.is_empty() {
                        keys.push(("backspace", "Back".into()));
                    }
                    footer(text, keys, cx)
                }
            });
        for (ix, group) in groups.iter().enumerate() {
            if ix > 0 && group.label.is_none() {
                palette = palette.separator();
            }
            let entry = CommandGroup::new()
                .items(group.items.iter().map(|item| command_item(item, &query)));
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
                    // Escape clears the query, then goes back a level, then closes.
                    .on_action(cx.listener(|bar, _: &Cancel, window, cx| {
                        if !bar.query_text(cx).is_empty() {
                            if let Some(input) = bar.input.clone() {
                                input.update(cx, |input, cx| input.set_value("", window, cx));
                            }
                            bar.apply_query("", window, cx);
                            cx.notify();
                        } else if !bar.step_back(window, cx) {
                            bar.close(window, cx);
                        }
                    }))
                    // The field passes Backspace up when there is nothing left to delete.
                    .on_action(cx.listener(|bar, _: &Backspace, window, cx| {
                        if !(bar.query_text(cx).is_empty() && bar.step_back(window, cx)) {
                            cx.propagate();
                        }
                    }))
                    .child(palette)
                    .focus_trap("action-bar-trap", &self.trap),
            )
            .into_any_element()
    }
}
