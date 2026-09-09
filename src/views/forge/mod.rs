//! Forge - MongoDB Query Shell
//!
//! A database-scoped query shell with a Forge editor for syntax highlighting,
//! autocomplete, and IDE-like experience.

mod actions;
mod completion;
mod controller;
mod editor;
pub(crate) mod editor_behavior;
pub(crate) mod logic;
mod mongosh;
mod output;
pub(crate) mod parser;
mod runtime;
mod state;
mod types;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::Editor;
use gpui_kit::component::resizable::{resizable_panel, v_resizable};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{
    Button, ConnectionIdentity, QueryLibraryDialog, QueryLibraryTarget, connection_identity_badge,
};
use crate::state::{AppEvent, AppState, View};
use crate::theme::{fonts, islands, spacing};
use controller::ForgeController;
use output::format_result_tab_label;
use state::ForgeState;
use types::ForgeOutputTab;

// ============================================================================
// ForgeView
// ============================================================================

pub struct ForgeView {
    app_state: Entity<AppState>,
    state: ForgeState,
    controller: ForgeController,
    _subscriptions: Vec<Subscription>,
}

impl ForgeView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let controller = ForgeController::new();

        let subscriptions = vec![
            cx.observe(&state, |this, state, cx| {
                let mut closed_tabs = Vec::new();
                this.state.editor.buffers.retain(|id, _| {
                    let open = state.read(cx).forge_tab_content(*id).is_some();
                    if !open {
                        closed_tabs.push(*id);
                    }
                    open
                });
                if !closed_tabs.is_empty() {
                    let runtime = this.controller.runtime.clone();
                    state
                        .read(cx)
                        .connection_manager()
                        .runtime_handle()
                        .spawn_blocking(move || runtime.dispose_sessions(&closed_tabs));
                }
                if this
                    .state
                    .editor
                    .active_tab_id
                    .is_some_and(|id| !this.state.editor.buffers.contains_key(&id))
                {
                    this.state.editor.active_tab_id = None;
                    this.state.editor.editor_state = None;
                }
                cx.notify();
            }),
            cx.subscribe(&state, |this, state, event, cx| {
                if matches!(event, AppEvent::ViewChanged) {
                    let visible = matches!(state.read(cx).current_view, View::Forge);
                    this.state.editor.editor_focus_requested = visible;
                    cx.notify();
                }
            }),
        ];

        Self {
            app_state: state,
            state: ForgeState::new(focus_handle),
            controller,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.editor.editor_focus_requested = true;
        ForgeController::focus_editor(self, window, cx);
        cx.notify();
    }

    fn render_header(&self, cx: &App) -> impl IntoElement {
        let target = self.app_state.read(cx).active_forge_tab_key().cloned();
        let database = target
            .as_ref()
            .map(|key| key.database.clone())
            .unwrap_or_else(|| "Unknown".to_string());
        let identity = target.as_ref().and_then(|key| {
            self.app_state
                .read(cx)
                .connection_by_id(key.connection_id)
                .map(ConnectionIdentity::from)
        });

        div()
            .flex()
            .items_center()
            .justify_between()
            .px(spacing::md())
            .py(spacing::sm())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child("Forge"),
                    )
                    .when_some(identity, |header, identity| {
                        header.child(connection_identity_badge(&identity, true, cx))
                    })
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child(database)),
            )
            .child(
                Button::new("forge-query-library")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::BookOpen).xsmall())
                    .label("Query Library")
                    .disabled(target.is_none())
                    .on_click({
                        let state = self.app_state.clone();
                        move |_, window, cx| {
                            let Some(target) = target.clone() else {
                                return;
                            };
                            QueryLibraryDialog::open(
                                state.clone(),
                                QueryLibraryTarget::Forge(target),
                                window,
                                cx,
                            );
                        }
                    }),
            )
    }

    fn render_output(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let raw_focused = self
            .state
            .output
            .raw
            .input
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window));
        let search_focused = self
            .state
            .output
            .results_search_state
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window));
        let removed_focus = match self.state.output.output_tab {
            ForgeOutputTab::Raw => search_focused,
            ForgeOutputTab::Results => {
                raw_focused || (search_focused && self.state.output.result_pages.is_empty())
            }
        };
        if removed_focus {
            ForgeController::focus_output(self, window, cx);
        }
        let forge_view = cx.entity();
        let appearance = self.app_state.read(cx).settings.appearance.clone();

        let clear_button =
            Button::new("forge-output-clear").xsmall().ghost().label("Clear").on_click({
                let forge_view = forge_view.clone();
                move |_, _window, cx| {
                    forge_view.update(cx, |this, cx| {
                        ForgeController::clear_output(this, _window, cx);
                    });
                }
            });

        let selected_index = match self.state.output.output_tab {
            ForgeOutputTab::Raw => 0,
            ForgeOutputTab::Results => {
                if self.state.output.result_pages.is_empty() {
                    1
                } else {
                    self.state
                        .output
                        .result_page_index
                        .min(self.state.output.result_pages.len().saturating_sub(1))
                        + 1
                }
            }
        };

        let has_inline_result = self.state.output.last_result.is_some()
            || self.state.output.last_error.is_some()
            || self.state.runtime.mongosh_error.is_some();
        let result_ids =
            self.state.output.result_pages.iter().map(|page| page.id).collect::<Vec<_>>();

        let tab_bar = islands::tab_bar(TabBar::new("forge-output-tabs"), &appearance)
            .small()
            .selected_index(selected_index)
            .on_click({
                let forge_view = forge_view.clone();
                move |index, window, cx| {
                    let index = *index;
                    forge_view.update(cx, |this, cx| {
                        this.state.output.auto_select_results = false;
                        if index == 0 {
                            this.state.output.output_tab = ForgeOutputTab::Raw;
                        } else {
                            if let Some(id) = result_ids.get(index - 1) {
                                let Some(index) = this
                                    .state
                                    .output
                                    .result_pages
                                    .iter()
                                    .position(|page| page.id == *id)
                                else {
                                    return;
                                };
                                ForgeController::select_result_page(this, index);
                            } else if !this.state.output.result_pages.is_empty() {
                                return;
                            }
                            this.state.output.output_tab = ForgeOutputTab::Results;
                        }
                        ForgeController::focus_output(this, window, cx);
                        cx.notify();
                    });
                }
            })
            .children(
                std::iter::once(Tab::new().label("Console"))
                    .chain(if self.state.output.result_pages.is_empty() && has_inline_result {
                        vec![Tab::new().label("Result")].into_iter()
                    } else {
                        Vec::new().into_iter()
                    })
                    .chain(self.state.output.result_pages.iter().enumerate().map(
                        |(index, page)| {
                            let page_id = page.id;
                            let label = format_result_tab_label(&page.label, index);
                            let view_entity = forge_view.clone();
                            let pin_icon = if page.pinned {
                                Icon::new(IconName::Star).xsmall().text_color(cx.theme().primary)
                            } else {
                                Icon::new(IconName::StarOff)
                                    .xsmall()
                                    .text_color(cx.theme().muted_foreground)
                            };
                            let pin_button = div()
                                .id(("forge-result-pin", index))
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(14.0))
                                .h(px(14.0))
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .hover(|s| s.bg(cx.theme().secondary.opacity(0.45)))
                                .child(pin_icon)
                                .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                    cx.stop_propagation();
                                    view_entity.update(cx, |this, cx| {
                                        let Some(index) = this
                                            .state
                                            .output
                                            .result_pages
                                            .iter()
                                            .position(|page| page.id == page_id)
                                        else {
                                            return;
                                        };
                                        ForgeController::toggle_result_pinned(this, index);
                                        cx.notify();
                                    });
                                });

                            let view_entity = forge_view.clone();
                            let close_button = div()
                                .id(("forge-result-close", index))
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(14.0))
                                .h(px(14.0))
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .text_color(cx.theme().muted_foreground)
                                .hover(|s| {
                                    s.bg(cx.theme().secondary.opacity(0.45))
                                        .text_color(cx.theme().foreground)
                                })
                                .child(Icon::new(IconName::Close).xsmall())
                                .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                    cx.stop_propagation();
                                    view_entity.update(cx, |this, cx| {
                                        let Some(index) = this
                                            .state
                                            .output
                                            .result_pages
                                            .iter()
                                            .position(|page| page.id == page_id)
                                        else {
                                            return;
                                        };
                                        ForgeController::close_result_page(this, index);
                                        cx.notify();
                                    });
                                });

                            Tab::new().label(label).prefix(pin_button).suffix(close_button)
                        },
                    )),
            );

        let body: AnyElement = match self.state.output.output_tab {
            ForgeOutputTab::Results => self.render_results_body(window, cx).into_any_element(),
            ForgeOutputTab::Raw => self.render_raw_output_body(window, cx).into_any_element(),
        };
        let following = self.state.output.raw.following();
        let show_follow = self.state.output.output_tab == ForgeOutputTab::Raw;
        let follow_button = Button::new("forge-output-follow")
            .ghost()
            .xsmall()
            .icon(Icon::new(IconName::ChevronDown).xsmall())
            .label(if following { "Following" } else { "Follow output" })
            .disabled(following)
            .tooltip("Follow new output. Scrolling up pauses following.")
            .on_click({
                let forge_view = forge_view.clone();
                move |_, window, cx| {
                    forge_view.update(cx, |this, cx| this.follow_raw_output(window, cx));
                }
            });
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .size_full()
            .px(spacing::md())
            .pb(spacing::sm())
            .pt(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(spacing::sm())
                    .min_w(px(0.0))
                    .py(px(2.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Output"),
                            )
                            .child(tab_bar.flex_1().min_w(px(0.0))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_shrink_0()
                            .gap(spacing::xs())
                            .when(
                                self.state.output.skipped_output_events > 0
                                    || (show_follow && self.state.output.trimmed_output_lines > 0),
                                |row| {
                                    row.child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(
                                                if self.state.output.skipped_output_events > 0 {
                                                    format!(
                                                        "{} output events skipped",
                                                        self.state.output.skipped_output_events
                                                    )
                                                } else {
                                                    format!(
                                                        "{} lines trimmed",
                                                        self.state.output.trimmed_output_lines
                                                    )
                                                },
                                            ),
                                    )
                                },
                            )
                            .when(show_follow, |row| row.child(follow_button))
                            .child(clear_button),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .mt(spacing::xs())
                    .overflow_hidden()
                    .child(body),
            )
    }

    // render_results_body/render_raw_output_body moved to output module
}

impl Render for ForgeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.app_state.clone();
        let appearance = state.read(cx).settings.appearance.clone();

        // Check if we have an active Forge tab
        let has_forge_tab = state.read(cx).active_forge_tab_id().is_some();
        log::debug!("ForgeView::render - has_forge_tab: {}", has_forge_tab);

        // If no Forge tab, return empty placeholder
        if !has_forge_tab {
            return div().size_full().into_any_element();
        }

        self.sync_active_tab_content(window, cx);
        let Some(editor_state) = &self.state.editor.editor_state else {
            return div().size_full().into_any_element();
        };
        if self.state.editor.editor_focus_requested {
            self.state.editor.editor_focus_requested = false;
            let focus = editor_state.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        };
        let forge_view = cx.entity();
        let editor_child: AnyElement = Editor::new(editor_state)
            .bordered(false)
            .aria_label("Forge JavaScript editor")
            .font_family(fonts::mono())
            .text_sm()
            .text_color(cx.theme().foreground)
            .h_full()
            .w_full()
            .into_any_element();
        let status_text = if self.state.runtime.mongosh_error.is_some() {
            "Shell error"
        } else if self.state.runtime.is_running {
            "Running..."
        } else {
            "Ready"
        };

        let editor_panel = {
            let mut panel = div()
                .id("forge-editor-container")
                .relative()
                .flex()
                .flex_col()
                .min_h(px(0.0))
                .px(spacing::md())
                .pt(spacing::sm())
                .pb(px(6.0))
                .child(
                    div().relative().flex_1().min_h(px(0.0)).overflow_hidden().child(editor_child),
                );

            if self.state.output.output_visible {
                panel = panel.flex_1().min_h(px(0.0));
            } else {
                panel = panel.flex_1();
            }
            panel
        };

        let output_panel = if self.state.output.output_visible {
            Some(self.render_output(window, cx).into_any_element())
        } else {
            None
        };

        let show_output_button = if self.state.output.output_visible {
            None
        } else {
            Some(
                Button::new("forge-output-show")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::ChevronDown).xsmall())
                    .label("Show output")
                    .on_click({
                        let forge_view = forge_view.clone();
                        move |_, _window, cx| {
                            forge_view.update(cx, |this, _cx| {
                                this.state.output.output_visible = true;
                            });
                        }
                    })
                    .into_any_element(),
            )
        };

        let split_panel = if self.state.output.output_visible {
            v_resizable("forge-main-split")
                .child(
                    resizable_panel()
                        .size(px(320.0))
                        .size_range(px(200.0)..px(1200.0))
                        .child(editor_panel),
                )
                .child(
                    resizable_panel()
                        .size(px(320.0))
                        .size_range(px(200.0)..px(1600.0))
                        .child(output_panel.unwrap_or_else(|| div().into_any_element())),
                )
                .into_any_element()
        } else {
            div().flex().flex_col().flex_1().min_h(px(0.0)).child(editor_panel).into_any_element()
        };

        let root = div()
            .key_context("ForgeView")
            .track_focus(&self.state.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(islands::content_bg(&appearance, cx))
            .child(self.render_header(cx))
            .child(div().flex_1().flex().flex_col().min_h(px(0.0)).child(split_panel))
            .child(
                // Status bar / help text
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(spacing::md())
                    .py(spacing::xs())
                    .bg(islands::content_bg(&appearance, cx))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .font_family(fonts::ui())
                            .child("⌘↩ Run all | ⌘⇧↩ Run selection/statement | Esc Cancel"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .children(show_output_button)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(spacing::xs())
                                    .when(self.state.runtime.is_running, |el: Div| {
                                        el.child(Spinner::new().xsmall())
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(status_text),
                                    ),
                            )
                            .child(
                                Button::new("forge-restart")
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(IconName::Redo).xsmall())
                                    .label("Restart")
                                    .on_click({
                                        let forge_view = forge_view.clone();
                                        move |_, _window, cx| {
                                            forge_view.update(cx, |this, cx| {
                                                this.restart_session(cx);
                                            });
                                        }
                                    }),
                            ),
                    ),
            )
            .children(
                self.state
                    .editor
                    .active_tab_id
                    .and_then(|id| self.state.editor.buffers.get(&id))
                    .map(|buffer| buffer.completion_menu.clone()),
            );

        actions::bind_root_actions(root, self.app_state.clone(), cx).into_any_element()
    }
}

// ============================================================================
// Forge Results Tree Rendering (Aggregation-style)
// ============================================================================
