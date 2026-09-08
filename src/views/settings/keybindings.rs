use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{Button, open_confirm_dialog, request_app_quit};
use crate::keyboard::{
    KeybindingCommand, KeybindingIssue, KeybindingIssueSeverity, keybinding_commands,
    normalize_shortcut, validate_keybinding_override,
};
use crate::state::{AppState, StatusMessage};
use crate::theme::{borders, spacing};

pub struct KeybindingsView {
    state: Entity<AppState>,
    focus_handle: FocusHandle,
    search_state: Option<Entity<InputState>>,
    search_focused: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl KeybindingsView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            focus_handle: cx.focus_handle(),
            search_state: None,
            search_focused: false,
            error: None,
            _subscriptions: vec![subscription],
        }
    }

    fn ensure_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_state.is_some() {
            return;
        }
        self._subscriptions.push(
            cx.on_focus_out(&self.focus_handle, window, |view, _, _, cx| view.cancel_capture(cx)),
        );
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search commands, shortcuts, or contexts")
                .clean_on_escape()
        });
        let subscription = cx.subscribe_in(&search, window, |view, _, event, _window, cx| {
            if matches!(event, InputEvent::Change) {
                view.error = None;
                cx.notify();
            }
        });
        self._subscriptions.push(subscription);
        self.search_state = Some(search);
    }

    fn filtered_commands(&self, cx: &App) -> Vec<KeybindingCommand> {
        let query = self
            .search_state
            .as_ref()
            .map(|state| state.read(cx).value().trim().to_ascii_lowercase())
            .unwrap_or_default();
        keybinding_commands(&self.state.read(cx).settings.keybindings)
            .into_iter()
            .filter(|command| {
                query.is_empty()
                    || command.label.to_ascii_lowercase().contains(&query)
                    || command.category.to_ascii_lowercase().contains(&query)
                    || command.context.to_ascii_lowercase().contains(&query)
                    || command
                        .default_shortcuts
                        .iter()
                        .chain(&command.effective_shortcuts)
                        .any(|shortcut| shortcut.to_ascii_lowercase().contains(&query))
            })
            .collect()
    }

    fn begin_capture(&mut self, binding_id: String, cx: &mut Context<Self>) {
        self.error = None;
        self.state.update(cx, |state, cx| {
            state.begin_keybinding_capture(binding_id);
            cx.notify();
        });
    }

    fn cancel_capture(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.cancel_keybinding_capture();
            cx.notify();
        });
    }

    fn save_override(
        &mut self,
        binding_id: String,
        shortcut: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let shortcut = shortcut.map(|shortcut| {
            normalize_shortcut(&shortcut).unwrap_or_else(|_| shortcut.trim().to_string())
        });
        let result = self.state.update(cx, |state, cx| {
            let result = state.set_keybinding_override(binding_id, shortcut);
            if result.is_ok() {
                state.cancel_keybinding_capture();
                state.set_status_message(Some(StatusMessage::info(
                    "Keybinding saved. Restart OpenMango to apply it.",
                )));
            }
            cx.notify();
            result
        });
        self.error = result.err().map(|error| format!("Keybinding could not be saved: {error}"));
        cx.notify();
    }

    fn reset_binding(&mut self, binding_id: String, cx: &mut Context<Self>) {
        let result = self.state.update(cx, |state, cx| {
            let result = state.reset_keybinding(&binding_id);
            if result.is_ok() {
                state.cancel_keybinding_capture();
                state.set_status_message(Some(StatusMessage::info(
                    "Keybinding reset. Restart OpenMango to apply it.",
                )));
            }
            cx.notify();
            result
        });
        self.error = result.err().map(|error| format!("Keybinding could not be reset: {error}"));
        cx.notify();
    }

    fn reset_all(&mut self, cx: &mut Context<Self>) {
        let result = self.state.update(cx, |state, cx| {
            let result = state.reset_all_keybindings();
            if result.is_ok() {
                state.cancel_keybinding_capture();
                state.set_status_message(Some(StatusMessage::info(
                    "All keybindings reset. Restart OpenMango to apply them.",
                )));
            }
            cx.notify();
            result
        });
        self.error = result.err().map(|error| format!("Keybindings could not be reset: {error}"));
        cx.notify();
    }

    fn render_command(
        &self,
        command: KeybindingCommand,
        index: usize,
        capture: Option<&crate::state::KeybindingCapture>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let is_recording = capture.is_some_and(|capture| capture.binding_id == command.id);
        let candidate = capture
            .filter(|capture| capture.binding_id == command.id)
            .and_then(|capture| capture.shortcut.clone());
        let issues = candidate.as_deref().map(|shortcut| {
            validate_keybinding_override(
                &self.state.read(cx).settings.keybindings,
                &command.id,
                shortcut,
            )
            .unwrap_or_else(|error| {
                vec![KeybindingIssue {
                    severity: KeybindingIssueSeverity::Error,
                    message: error,
                    conflicting_binding_id: None,
                }]
            })
        });
        let has_error = issues.as_ref().is_some_and(|issues| {
            issues.iter().any(|issue| issue.severity == KeybindingIssueSeverity::Error)
        });

        div()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.6))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::md())
                    .px(spacing::md())
                    .py(spacing::sm())
                    .when(is_recording, |row| row.bg(cx.theme().secondary.opacity(0.22)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(140.0))
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(cx.theme().foreground)
                                    .child(command.label.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(spacing::xs())
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("When: {}", friendly_context(&command.context)))
                                    .when(command.modified, |meta| {
                                        meta.child("•").child(if command.disabled {
                                            "Disabled"
                                        } else {
                                            "Modified"
                                        })
                                    }),
                            ),
                    )
                    .child(div().w(px(230.0)).flex_shrink_0().child(shortcut_list(
                        &command.effective_shortcuts,
                        command.disabled,
                        cx,
                    )))
                    .child(
                        Button::new(("keybinding-record", index))
                            .xsmall()
                            .ghost()
                            .label(if is_recording { "Recording…" } else { "Edit" })
                            .disabled(is_recording)
                            .on_click({
                                let view = view.clone();
                                let binding_id = command.id.clone();
                                move |_, _window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.begin_capture(binding_id.clone(), cx);
                                    });
                                }
                            }),
                    ),
            )
            .when(is_recording, |item| {
                item.child(self.render_capture_panel(command, candidate, issues, has_error, cx))
            })
            .into_any_element()
    }

    fn render_capture_panel(
        &self,
        command: KeybindingCommand,
        candidate: Option<String>,
        issues: Option<Vec<KeybindingIssue>>,
        has_error: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .px(spacing::md())
            .py(spacing::sm())
            .bg(cx.theme().secondary.opacity(0.12))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(spacing::md())
                    .child(div().text_sm().text_color(cx.theme().secondary_foreground).child(
                        if let Some(candidate) = candidate.as_deref() {
                            format!("New shortcut: {}", shortcut_display(candidate))
                        } else {
                            "Press a shortcut. Esc cancels recording.".to_string()
                        },
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                Button::new("keybinding-capture-save")
                                    .xsmall()
                                    .primary()
                                    .label("Save")
                                    .disabled(candidate.is_none() || has_error)
                                    .on_click({
                                        let view = view.clone();
                                        let binding_id = command.id.clone();
                                        let candidate = candidate.clone();
                                        move |_, _window, cx| {
                                            let Some(candidate) = candidate.clone() else {
                                                return;
                                            };
                                            view.update(cx, |this, cx| {
                                                this.save_override(
                                                    binding_id.clone(),
                                                    Some(candidate),
                                                    cx,
                                                );
                                            });
                                        }
                                    }),
                            )
                            .child(
                                Button::new("keybinding-capture-disable")
                                    .xsmall()
                                    .ghost()
                                    .label("Disable")
                                    .disabled(command.disabled)
                                    .on_click({
                                        let view = view.clone();
                                        let binding_id = command.id.clone();
                                        move |_, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.save_override(binding_id.clone(), None, cx);
                                            });
                                        }
                                    }),
                            )
                            .when(command.modified, |actions| {
                                actions.child(
                                    Button::new("keybinding-capture-reset")
                                        .xsmall()
                                        .ghost()
                                        .label("Reset")
                                        .on_click({
                                            let view = view.clone();
                                            let binding_id = command.id.clone();
                                            move |_, _window, cx| {
                                                view.update(cx, |this, cx| {
                                                    this.reset_binding(binding_id.clone(), cx);
                                                });
                                            }
                                        }),
                                )
                            })
                            .child(
                                Button::new("keybinding-capture-cancel")
                                    .xsmall()
                                    .label("Cancel")
                                    .on_click({
                                        let view = view.clone();
                                        move |_, _window, cx| {
                                            view.update(cx, |this, cx| this.cancel_capture(cx));
                                        }
                                    }),
                            ),
                    ),
            )
            .when(command.modified, |panel| {
                panel.child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!(
                            "Default: {}",
                            command
                                .default_shortcuts
                                .iter()
                                .map(|shortcut| shortcut_display(shortcut))
                                .collect::<Vec<_>>()
                                .join("  ")
                        )))
            })
            .children(issues.unwrap_or_default().into_iter().map(|issue| {
                let color = match issue.severity {
                    KeybindingIssueSeverity::Error => cx.theme().danger,
                    KeybindingIssueSeverity::Warning => cx.theme().warning,
                };
                div().text_xs().text_color(color).child(match issue.severity {
                    KeybindingIssueSeverity::Error => format!("Conflict: {}", issue.message),
                    KeybindingIssueSeverity::Warning => format!("Warning: {}", issue.message),
                })
            }))
    }
}

impl Render for KeybindingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_search(window, cx);
        let search_state = self.search_state.clone().expect("search input initialized");
        if !self.search_focused {
            search_state.update(cx, |state, cx| state.focus(window, cx));
            self.search_focused = true;
        }

        // Reserve room for Settings/app chrome and bottom padding. Pinning the list viewport avoids
        // nested flex containers expanding to the full command-list height instead of scrolling.
        let list_height = window.viewport_size().height - px(280.0);
        let commands = self.filtered_commands(cx);
        let visible_count = commands.len();
        let total = keybinding_commands(&self.state.read(cx).settings.keybindings).len();
        let capture = self.state.read(cx).keybinding_capture().cloned();
        let override_count = self.state.read(cx).settings.keybindings.overrides.len();
        let restart_required =
            self.state.read(cx).settings.keybindings != self.state.read(cx).startup_keybindings;
        let view = cx.entity();
        let state = self.state.clone();
        let commands_empty = commands.is_empty();
        let mut list_items = Vec::<AnyElement>::new();
        let mut last_category = None::<String>;
        for (index, command) in commands.into_iter().enumerate() {
            if last_category.as_deref() != Some(command.category.as_str()) {
                last_category = Some(command.category.clone());
                list_items.push(
                    div()
                        .px(spacing::md())
                        .py(spacing::xs())
                        .bg(cx.theme().secondary.opacity(0.18))
                        .border_b_1()
                        .border_color(cx.theme().border.opacity(0.6))
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().muted_foreground)
                        .child(command.category.clone())
                        .into_any_element(),
                );
            }
            list_items.push(self.render_command(command, index, capture.as_ref(), cx));
        }

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .flex_1()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .gap(spacing::md())
            .when(restart_required, |content| {
                content.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(spacing::md())
                        .px(spacing::md())
                        .py(spacing::sm())
                        .rounded(borders::radius_sm())
                        .border_1()
                        .border_color(cx.theme().warning.opacity(0.5))
                        .bg(cx.theme().warning.opacity(0.08))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().secondary_foreground)
                                .child("Restart OpenMango to apply keybinding changes."),
                        )
                        .child(
                            Button::new("restart-for-keybindings")
                                .xsmall()
                                .primary()
                                .label("Restart now")
                                .on_click(move |_, window, cx| {
                                    request_app_quit(state.clone(), window, cx);
                                }),
                        ),
                )
            })
            .when_some(self.error.clone(), |content, error| {
                content.child(
                    div()
                        .px(spacing::md())
                        .py(spacing::sm())
                        .rounded(borders::radius_sm())
                        .border_1()
                        .border_color(cx.theme().danger.opacity(0.45))
                        .bg(cx.theme().danger.opacity(0.08))
                        .text_sm()
                        .text_color(cx.theme().danger_foreground)
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(Input::new(&search_state).w_full()),
                    )
                    .child(
                        Button::new("reset-all-keybindings")
                            .xsmall()
                            .ghost()
                            .label("Reset All…")
                            .disabled(override_count == 0)
                            .on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    open_confirm_dialog(
                                        window,
                                        cx,
                                        "Reset all keybindings",
                                        format!(
                                            "Reset all {override_count} customized keybindings to their defaults?"
                                        ),
                                        "Reset all",
                                        false,
                                        {
                                            let view = view.clone();
                                            move |_window, cx| {
                                                view.update(cx, |this, cx| this.reset_all(cx));
                                            }
                                        },
                                    );
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{visible_count} of {total} commands"))
                    .child("Changes apply after restart."),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .h(list_height)
                    .flex_shrink_0()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(borders::radius_sm())
                    .overflow_y_scrollbar()
                    .when(commands_empty, |list| {
                        list.child(
                            div()
                                .px(spacing::md())
                                .py(spacing::lg())
                                .text_center()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No keybindings match this search."),
                        )
                    })
                    .children(list_items),
            )
    }
}

fn shortcut_list(shortcuts: &[String], disabled: bool, cx: &App) -> AnyElement {
    if disabled {
        return div()
            .flex()
            .justify_end()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .text_color(cx.theme().muted_foreground)
            .child("Disabled")
            .into_any_element();
    }
    div()
        .flex()
        .flex_wrap()
        .justify_end()
        .gap(spacing::xs())
        .children(shortcuts.iter().map(|shortcut| {
            div()
                .px(spacing::xs())
                .py(px(2.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary.opacity(0.2))
                .font_family(crate::theme::fonts::mono())
                .text_xs()
                .text_color(cx.theme().secondary_foreground)
                .child(shortcut_display(shortcut))
        }))
        .into_any_element()
}

fn shortcut_display(shortcut: &str) -> String {
    shortcut
        .split_whitespace()
        .map(|part| Keystroke::parse(part).map_or_else(|_| part.to_string(), |key| key.to_string()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn friendly_context(context: &str) -> String {
    let base = if context.contains("Workspace") && context.contains("!Documents") {
        "Workspace, outside Documents"
    } else if context.contains("Documents || Sidebar") {
        "Documents or sidebar"
    } else if context.contains("Database || Collection") {
        "Database or collection"
    } else if context.contains("JsonEditorWindow") {
        "JSON editor"
    } else if context.contains("TransferQueryModal") && !context.contains("!TransferQueryModal") {
        "Transfer query editor"
    } else if context.contains("!TransferRunning") {
        "Transfer, while idle"
    } else if context.contains("TransferRunning") {
        "Transfer, while running"
    } else if context.contains("Transfer") {
        "Transfer"
    } else if context.contains("Aggregation") {
        "Aggregation"
    } else if context.contains("Indexes") {
        "Indexes"
    } else if context.contains("Schema") {
        "Schema"
    } else if context.contains("Stats") {
        "Statistics"
    } else if context.contains("Forge") {
        "Forge"
    } else if context.contains("Documents") && !context.contains("!Documents") {
        "Documents"
    } else if context.contains("Sidebar") {
        "Sidebar"
    } else if context.contains("Settings") {
        "Settings"
    } else if context.contains("Workspace") {
        "Anywhere in workspace"
    } else {
        return context.to_string();
    };

    if context.contains("!Input") {
        format!("{base}, outside editor")
    } else if context.contains("Input") {
        format!("{base} editor")
    } else if context.contains("List") {
        format!("{base} list")
    } else if context.contains("SearchOpen") {
        format!("{base}, search open")
    } else {
        base.to_string()
    }
}
