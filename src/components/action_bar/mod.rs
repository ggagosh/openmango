mod providers;
mod types;

pub use types::ActionExecution;

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::input::{Input, InputEvent, InputState};

use crate::app::search::fuzzy_match_score;
use crate::state::AppState;
use crate::state::settings::AppTheme;
use crate::theme::{borders, fonts, spacing};

use providers::{command_actions, navigation_actions, tab_actions, theme_actions, view_actions};
use types::{FilteredAction, PaletteMode};

type ExecuteHandler = Box<dyn Fn(ActionExecution, &mut Window, &mut App) + 'static>;

pub struct ActionBar {
    state: Entity<AppState>,
    open: bool,
    mode: PaletteMode,
    original_theme: Option<AppTheme>,
    input_state: Option<Entity<InputState>>,
    all_actions: Vec<types::ActionItem>,
    filtered: Vec<FilteredAction>,
    selected_index: usize,
    scroll_offset: usize,
    on_execute: Option<ExecuteHandler>,
    _subscriptions: Vec<Subscription>,
}

impl ActionBar {
    pub fn new(state: Entity<AppState>) -> Self {
        Self {
            state,
            open: false,
            mode: PaletteMode::default(),
            original_theme: None,
            input_state: None,
            all_actions: Vec::new(),
            filtered: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            on_execute: None,
            _subscriptions: Vec::new(),
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
        if self.open {
            self.close(window, cx);
        } else {
            self.open(window, cx);
        }
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.selected_index = 0;
        self.rebuild_actions(cx);
        self.filter_actions("");

        // Create fresh input state
        let input_state = cx.new(|cx| InputState::new(window, cx).placeholder("Type to search..."));
        let input_sub =
            cx.subscribe_in(&input_state, window, |this, _state, event, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = _state.read(cx).value().to_string();
                    this.filter_actions(&query);
                    this.selected_index = 0;
                    this.scroll_offset = 0;
                    cx.notify();
                }
            });
        self.input_state = Some(input_state);

        // Intercept keystrokes globally to capture up/down/enter/escape
        // before the Input component consumes them
        let weak = cx.entity().downgrade();
        let key_sub = cx.intercept_keystrokes(move |event, window, cx| {
            let Some(entity) = weak.upgrade() else {
                return;
            };
            let key = event.keystroke.key.as_str();
            match key {
                "escape" => {
                    entity.update(cx, |bar, cx| bar.close(window, cx));
                    cx.stop_propagation();
                }
                "up" => {
                    entity.update(cx, |bar, cx| bar.move_selection(-1, window, cx));
                    cx.stop_propagation();
                }
                "down" => {
                    entity.update(cx, |bar, cx| bar.move_selection(1, window, cx));
                    cx.stop_propagation();
                }
                "enter" | "return" => {
                    entity.update(cx, |bar, cx| bar.execute_selected(window, cx));
                    cx.stop_propagation();
                }
                _ => {}
            }
        });

        self._subscriptions = vec![input_sub, key_sub];
        cx.notify();

        // Focus the input after a frame so it's rendered
        let input = self.input_state.clone().unwrap();
        cx.defer_in(window, move |_this, window, cx| {
            input.update(cx, |state, cx| {
                state.focus(window, cx);
            });
        });
    }

    fn switch_to_themes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = PaletteMode::Theme;
        self.original_theme = Some(self.state.read(cx).settings.appearance.theme);
        self.rebuild_actions(cx);
        self.filter_actions("");
        self.selected_index = 0;
        self.scroll_offset = 0;
        if let Some(input) = self.input_state.clone() {
            input.update(cx, |state, cx| {
                state.set_placeholder("Select Theme...", window, cx);
                state.set_value("", window, cx);
            });
        }
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Revert to original theme if we were previewing
        if let Some(original) = self.original_theme.take() {
            let vibrancy = self.state.read(cx).startup_vibrancy;
            crate::theme::apply_theme(original, vibrancy, window, cx);
        }
        self.open = false;
        self.mode = PaletteMode::All;
        self.all_actions.clear();
        self.filtered.clear();
        self.input_state = None;
        self._subscriptions.clear();
        cx.notify();
    }

    fn rebuild_actions(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        self.all_actions = match self.mode {
            PaletteMode::All => {
                let mut actions = Vec::new();
                actions.extend(tab_actions(state));
                actions.extend(command_actions(state));
                actions.extend(navigation_actions(state));
                actions.extend(view_actions(state));
                actions
            }
            PaletteMode::Theme => theme_actions(state),
        };
    }

    fn filter_actions(&mut self, query: &str) {
        let query = query.trim().to_lowercase();

        if query.is_empty() {
            let mut filtered: Vec<FilteredAction> = self
                .all_actions
                .iter()
                .filter(|a| a.available)
                .map(|item| FilteredAction { item: item.clone(), score: 0 })
                .collect();
            filtered.sort_by(|a, b| {
                // Highlighted items always first
                b.item
                    .highlighted
                    .cmp(&a.item.highlighted)
                    .then_with(|| a.item.category.sort_order().cmp(&b.item.category.sort_order()))
                    .then_with(|| a.item.priority.cmp(&b.item.priority))
            });
            self.filtered = filtered;
            return;
        }

        let mut filtered: Vec<FilteredAction> = self
            .all_actions
            .iter()
            .filter(|a| a.available)
            .filter_map(|item| {
                let label = item.label.to_lowercase();
                let detail = item.detail.as_ref().map(|d| d.to_lowercase()).unwrap_or_default();
                let combined = format!("{} {}", label, detail);

                fuzzy_match_score(&query, &combined)
                    .map(|score| FilteredAction { item: item.clone(), score })
            })
            .collect();

        filtered.sort_by(|a, b| {
            // Highlighted items first, then by score
            b.item
                .highlighted
                .cmp(&a.item.highlighted)
                .then_with(|| a.score.cmp(&b.score))
                .then_with(|| a.item.category.sort_order().cmp(&b.item.category.sort_order()))
                .then_with(|| a.item.priority.cmp(&b.item.priority))
        });

        self.filtered = filtered;
    }

    fn execute_at(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.filtered.get(index) else {
            return;
        };

        // Intercept "Theme Selector: Toggle" — switch to theme mode instead of closing
        if action.item.id.as_ref() == "cmd:change-theme" {
            self.switch_to_themes(window, cx);
            return;
        }

        // Theme confirm: clear original so close doesn't revert, then fire handler
        if self.mode == PaletteMode::Theme {
            self.original_theme = None;
        }

        let execution = ActionExecution { action_id: action.item.id.clone() };
        self.close(window, cx);

        if let Some(handler) = &self.on_execute {
            handler(execution, window, cx);
        }
    }

    fn execute_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.selected_index;
        self.execute_at(index, window, cx);
    }

    fn move_selection(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        if self.filtered.is_empty() {
            return;
        }
        let count = self.filtered.len() as i32;
        let new_index = (self.selected_index as i32 + delta).rem_euclid(count) as usize;
        self.selected_index = new_index;

        // Keep selection visible in the window
        let max_visible = 8;
        if new_index < self.scroll_offset {
            self.scroll_offset = new_index;
        } else if new_index >= self.scroll_offset + max_visible {
            self.scroll_offset = new_index + 1 - max_visible;
        }
        // Handle wrap-around
        if delta > 0 && new_index == 0 {
            self.scroll_offset = 0;
        } else if delta < 0 && new_index == self.filtered.len() - 1 {
            self.scroll_offset = self.filtered.len().saturating_sub(max_visible);
        }

        // Live-preview theme on selection change
        if self.mode == PaletteMode::Theme
            && let Some(item) = self.filtered.get(new_index)
            && let Some(theme_id) = item.item.id.as_ref().strip_prefix("theme:")
            && let Some(theme) = AppTheme::from_theme_id(theme_id)
        {
            let vibrancy = self.state.read(cx).startup_vibrancy;
            crate::theme::apply_theme(theme, vibrancy, window, cx);
        }

        cx.notify();
    }
}

impl Render for ActionBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }

        let entity = cx.entity().clone();
        let dismiss_entity = entity.clone();
        let item_count = self.filtered.len();

        div()
            .absolute()
            .inset_0()
            // Backdrop: click to dismiss
            .child(
                div()
                    .id("action-bar-backdrop")
                    .size_full()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(move |_, window, cx| {
                        dismiss_entity.update(cx, |bar, cx| {
                            bar.close(window, cx);
                        });
                    }),
            )
            // Palette container (centered at top)
            .child(
                div()
                    .absolute()
                    .top(px(60.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .mx_auto()
                    .w(px(620.0))
                    .flex()
                    .flex_col()
                    .bg(cx.theme().tab_bar)
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(borders::radius_sm())
                    .shadow_lg()
                    .overflow_hidden()
                    .text_color(cx.theme().foreground)
                    .font_family(fonts::ui())
                    // Input row
                    .child(
                        div()
                            .p(spacing::md())
                            .border_b_1()
                            .border_color(cx.theme().sidebar_border)
                            .child(if let Some(input_state) = &self.input_state {
                                Input::new(input_state)
                                    .appearance(false)
                                    .font_family(fonts::mono())
                                    .text_size(px(14.0))
                                    .into_any_element()
                            } else {
                                div().into_any_element()
                            }),
                    )
                    // Results list
                    .child({
                        if item_count == 0 {
                            div()
                                .px(spacing::md())
                                .py(spacing::lg())
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("No matching actions"),
                                )
                                .into_any_element()
                        } else {
                            let max_visible: usize = 8;
                            let visible_end = (self.scroll_offset + max_visible).min(item_count);
                            let visible_start = self.scroll_offset;

                            let scroll_entity = entity.clone();
                            let total = item_count;
                            let mut list = div()
                                .id("action-bar-results")
                                .py(spacing::sm())
                                .flex_col()
                                .on_scroll_wheel(move |event, _window, cx| {
                                    let delta = event.delta.pixel_delta(px(1.0));
                                    let steps = if delta.y < px(0.0) { 1i32 } else { -1i32 };
                                    scroll_entity.update(cx, |bar, cx| {
                                        let max_offset = total.saturating_sub(8) as i32;
                                        let new_offset = (bar.scroll_offset as i32 + steps)
                                            .clamp(0, max_offset)
                                            as usize;
                                        if new_offset != bar.scroll_offset {
                                            bar.scroll_offset = new_offset;
                                            cx.notify();
                                        }
                                    });
                                });

                            let show_categories = self.mode == PaletteMode::All;
                            let mut last_category = None;
                            for ix in visible_start..visible_end {
                                let item = &self.filtered[ix].item;
                                // Category group header (only in All mode)
                                if show_categories && last_category.as_ref() != Some(&item.category)
                                {
                                    last_category = Some(item.category.clone());
                                    list = list.child(
                                        div()
                                            .px(spacing::md())
                                            .pt(spacing::sm())
                                            .pb(spacing::xs())
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(item.category.label()),
                                    );
                                }
                                list = list.child(render_action_row(self, ix, &entity, cx));
                            }
                            list.into_any_element()
                        }
                    }),
            )
            .into_any_element()
    }
}

fn render_action_row(
    bar: &ActionBar,
    index: usize,
    entity: &Entity<ActionBar>,
    cx: &App,
) -> AnyElement {
    let Some(filtered) = bar.filtered.get(index) else {
        return div().into_any_element();
    };

    let is_selected = index == bar.selected_index;
    let item = &filtered.item;

    let mut row = div()
        .id(ElementId::Name(format!("action-{}", index).into()))
        .h(px(34.0))
        .px(spacing::md())
        .mx(spacing::xs())
        .rounded(borders::radius_sm())
        .flex()
        .items_center()
        .cursor_pointer();

    if is_selected {
        row = row.bg(cx.theme().list_active);
    } else {
        row = row.hover(|s| s.bg(cx.theme().list_hover));
    }

    // Left side: label + detail
    let label_color = if item.highlighted { cx.theme().primary } else { cx.theme().foreground };
    let mut left = div().flex().flex_1().items_center().gap(spacing::sm()).overflow_hidden();
    left = left
        .child(div().text_sm().text_color(label_color).flex_shrink_0().child(item.label.clone()));
    if let Some(detail) = &item.detail {
        left = left.child(
            div()
                .text_xs()
                .text_color(if item.highlighted {
                    cx.theme().primary
                } else {
                    cx.theme().muted_foreground
                })
                .overflow_hidden()
                .child(detail.clone()),
        );
    }

    // Right side: shortcut
    let mut right = div().flex().items_center().flex_shrink_0().ml(spacing::sm());
    if let Some(shortcut) = &item.shortcut {
        right = right
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(shortcut.clone()));
    }

    let click_entity = entity.clone();
    row = row
        .child(left)
        .child(right)
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_click(move |_, window, cx| {
            click_entity.update(cx, |bar, cx| {
                bar.execute_at(index, window, cx);
            });
        });

    row.into_any_element()
}
