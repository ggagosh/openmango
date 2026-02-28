use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::resizable::{resizable_panel, v_resizable};
use gpui_component::scroll::{Scrollbar, ScrollbarAxis};
use gpui_component::spinner::Spinner;

use crate::ai::bridge::AiBridge;
use crate::ai::budget::trim_history_for_context;
use crate::ai::provider::{AiGenerationRequest, generate_text_streaming};
use crate::ai::telemetry::AiRequestSpan;
use crate::ai::{AiChatEntry, AiTurn, ChatRole};
use crate::components::Button;
use crate::state::AppState;
use crate::theme::{borders, spacing};

const SYSTEM_PROMPT: &str =
    "You are a helpful general-purpose AI assistant. Answer concisely and accurately.";

pub struct AiView {
    state: Entity<AppState>,
    input_state: Option<Entity<InputState>>,
    input_subscription: Option<Subscription>,
    scroll_handle: ScrollHandle,
    last_entry_count: usize,
    _subscriptions: Vec<Subscription>,
}

impl AiView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];
        Self {
            state,
            input_state: None,
            input_subscription: None,
            scroll_handle: ScrollHandle::new(),
            last_entry_count: 0,
            _subscriptions: subscriptions,
        }
    }

    fn ensure_input_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if self.input_state.is_none() {
            let input_state = cx.new(|cx| {
                InputState::new(window, cx)
                    .code_editor("text")
                    .soft_wrap(true)
                    .line_number(false)
                    .auto_indent(false)
                    .clean_on_escape()
                    .placeholder("Message AI assistant... (Cmd+Enter to send)")
            });

            let state = self.state.clone();
            let sub =
                cx.subscribe_in(&input_state, window, move |view, entity, event, window, cx| {
                    match event {
                        InputEvent::Change => {
                            let raw = entity.read(cx).value().to_string();
                            state.update(cx, |state, _cx| {
                                state.ai_chat.draft_input = raw;
                            });
                        }
                        InputEvent::PressEnter { secondary: true } => {
                            let can_submit = {
                                let s = state.read(cx);
                                s.settings.ai.enabled
                                    && !s.ai_chat.is_loading
                                    && s.current_ai_session_key().is_some()
                            };

                            let prompt = entity.read(cx).value().to_string().trim().to_string();
                            if !prompt.is_empty() && can_submit {
                                entity.update(cx, |input, cx| {
                                    input.set_value(String::new(), window, cx);
                                });
                                state.update(cx, |s, _| {
                                    s.ai_chat.draft_input.clear();
                                });
                                view.send_message(prompt, cx);
                            }
                        }
                        _ => {}
                    }
                });
            self.input_subscription = Some(sub);
            self.input_state = Some(input_state.clone());
            input_state
        } else {
            self.input_state.clone().unwrap()
        }
    }

    fn send_message(&mut self, prompt: String, cx: &mut Context<Self>) {
        let ai_settings = self.state.read(cx).settings.ai.clone();

        let cancel_flag = Arc::new(AtomicBool::new(false));

        // Begin the turn and streaming response placeholder
        self.state.update(cx, |state, cx| {
            state.ai_chat.begin_turn(&prompt);
            state.ai_chat.is_loading = true;
            state.ai_chat.cancel_flag = Some(cancel_flag.clone());
            cx.notify();
        });

        let turn_id = self.state.read(cx).ai_chat.current_turn_id.unwrap();

        let mut message_id = None;
        self.state.update(cx, |state, cx| {
            message_id = state.ai_chat.begin_turn_streaming_response();
            cx.notify();
        });

        let Some(message_id) = message_id else {
            self.state.update(cx, |state, cx| {
                state.ai_chat.is_loading = false;
                state.ai_chat.cancel_flag = None;
                state.ai_chat.last_error =
                    Some("Failed to initialize streaming response.".to_string());
                cx.notify();
            });
            return;
        };

        // Collect history and trim for context
        let mut history = self.state.read(cx).ai_chat.messages();
        // Remove the last user message (current prompt) — it's passed separately
        if let Some(last) = history.last()
            && last.role == ChatRole::User
            && last.content == prompt
        {
            history.pop();
        }
        // Also remove the empty assistant placeholder
        if let Some(last) = history.last()
            && last.role == ChatRole::Assistant
            && last.content.is_empty()
        {
            history.pop();
        }
        trim_history_for_context(&mut history, SYSTEM_PROMPT.len(), None);

        let request = AiGenerationRequest {
            system_prompt: SYSTEM_PROMPT.to_string(),
            history,
            user_prompt: prompt,
        };

        let provider_label = ai_settings.provider.label().to_string();
        let model_label = ai_settings.model.clone();
        let session_label = turn_id.to_string();

        // Channel for streaming deltas
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        let cancel_for_task = cancel_flag.clone();
        let task = cx.background_spawn(async move {
            AiBridge::block_on(async move {
                generate_text_streaming(&ai_settings, request, move |delta| {
                    if !cancel_for_task.load(Ordering::Relaxed) {
                        let _ = tx.send(delta.to_string());
                    }
                })
                .await
            })
        });

        let state = self.state.clone();
        let cancel_for_poll = cancel_flag;
        cx.spawn(async move |_view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let span = AiRequestSpan::start(&provider_label, &model_label, &session_label);

            // Poll channel for deltas while the background task runs
            loop {
                match rx.try_recv() {
                    Ok(delta) => {
                        if cancel_for_poll.load(Ordering::Relaxed) {
                            break;
                        }
                        let _ = cx.update(|cx| {
                            state.update(cx, |s, cx| {
                                s.ai_chat.append_turn_delta(message_id, &delta);
                                cx.notify();
                            });
                        });
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        // Yield to let the background task produce more
                        gpui::Timer::after(std::time::Duration::from_millis(16)).await;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        break;
                    }
                }
            }

            // Drain any remaining deltas
            while let Ok(delta) = rx.try_recv() {
                let _ = cx.update(|cx| {
                    state.update(cx, |s, cx| {
                        s.ai_chat.append_turn_delta(message_id, &delta);
                        cx.notify();
                    });
                });
            }

            let result = task.await;

            let _ = cx.update(|cx| {
                state.update(cx, |s, cx| {
                    match result {
                        Ok(final_text) => {
                            s.ai_chat.finalize_turn_response(message_id, final_text.clone());
                            span.finish_ok(final_text.len());
                        }
                        Err(ref error) => {
                            let msg = error.user_message();
                            s.ai_chat.last_error = Some(msg.clone());
                            s.ai_chat.push_system_message(msg);
                            span.finish_err(error.kind());
                        }
                    }
                    s.ai_chat.is_loading = false;
                    s.ai_chat.cancel_flag = None;
                    s.ai_chat.current_turn_id = None;
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn stop_generation(&self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            if let Some(flag) = &state.ai_chat.cancel_flag {
                flag.store(true, Ordering::Relaxed);
            }
            state.ai_chat.is_loading = false;
            state.ai_chat.cancel_flag = None;
            state.ai_chat.current_turn_id = None;
            cx.notify();
        });
    }
}

impl Render for AiView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input_state = self.ensure_input_state(window, cx);

        let state = self.state.clone();
        let app_state = self.state.read(cx);
        let ai_chat = &app_state.ai_chat;
        let ai_enabled = app_state.settings.ai.enabled;
        let is_loading = ai_chat.is_loading;
        let last_error = ai_chat.last_error.clone();
        let session_key = app_state.current_ai_session_key();

        // Header
        let header = {
            let clear_state = state.clone();
            let header_buttons = div().flex().items_center().gap(spacing::sm());

            let header_buttons = if is_loading {
                let view = cx.entity();
                header_buttons.child(
                    Button::new("stop-gen").ghost().compact().label("Stop").on_click(
                        move |_, _, cx| {
                            view.update(cx, |this, cx| {
                                this.stop_generation(cx);
                            });
                        },
                    ),
                )
            } else {
                header_buttons.child(
                    Button::new("clear-chat")
                        .ghost()
                        .compact()
                        .label("Clear")
                        .disabled(!ai_enabled)
                        .on_click(move |_, _, cx| {
                            clear_state.update(cx, |state, cx| {
                                state.ai_chat.entries.clear();
                                state.ai_chat.last_error = None;
                                state.ai_chat.cancel_flag = None;
                                state.ai_chat.current_turn_id = None;
                                cx.notify();
                            });
                        }),
                )
            };

            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(44.0))
                .px(spacing::lg())
                .bg(cx.theme().tab_bar)
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cx.theme().foreground)
                        .child("AI Chat"),
                )
                .child(header_buttons)
        };

        // Error banner
        let error_banner = last_error.map(|error_text| {
            let dismiss_state = state.clone();
            div()
                .flex()
                .items_center()
                .justify_between()
                .p(spacing::md())
                .text_xs()
                .text_color(cx.theme().danger_foreground)
                .child(error_text)
                .child(Button::new("dismiss-error").ghost().compact().label("Dismiss").on_click(
                    move |_, _, cx| {
                        dismiss_state.update(cx, |state, cx| {
                            state.ai_chat.last_error = None;
                            cx.notify();
                        });
                    },
                ))
        });

        // Message list with manual scroll for auto-scroll-to-bottom
        let entries = ai_chat.entries.clone();
        let entry_count = entries.len();
        let should_scroll = entry_count != self.last_entry_count || is_loading;
        self.last_entry_count = entry_count;

        if should_scroll {
            // Use a large value; gpui clamps to actual max after layout.
            self.scroll_handle.set_offset(point(px(0.0), px(-1_000_000.0)));
        }

        let scroll_handle = self.scroll_handle.clone();
        let message_list = div()
            .size_full()
            .overflow_hidden()
            .relative()
            .child(
                div()
                    .id("ai-chat-scroll")
                    .flex()
                    .flex_col()
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&scroll_handle)
                    .child(
                        div().flex().flex_col().p(spacing::lg()).gap(spacing::md()).children(
                            entries.iter().map(|entry| render_entry(entry, is_loading, cx)),
                        ),
                    ),
            )
            .child(
                div().absolute().top_0().left_0().right_0().bottom_0().child(
                    Scrollbar::new(&scroll_handle)
                        .id("ai-chat-scrollbar")
                        .axis(ScrollbarAxis::Vertical),
                ),
            );

        // Send/Stop button
        let send_or_stop_button = if is_loading {
            let stop_view = cx.entity();
            Button::new("send-stop").compact().label("Stop").on_click(move |_, _, cx| {
                stop_view.update(cx, |this, cx| {
                    this.stop_generation(cx);
                });
            })
        } else {
            let view = cx.entity();
            let input_state_for_submit = input_state.clone();
            let can_submit = ai_enabled && !is_loading && session_key.is_some();
            Button::new("send-message").compact().label("Send").disabled(!can_submit).on_click(
                move |_, window, cx| {
                    let prompt = input_state_for_submit.read(cx).value().to_string();
                    let prompt = prompt.trim().to_string();
                    if prompt.is_empty() {
                        return;
                    }
                    input_state_for_submit.update(cx, |input, cx| {
                        input.set_value(String::new(), window, cx);
                    });
                    view.update(cx, |this, cx| {
                        this.state.update(cx, |state, _cx| {
                            state.ai_chat.draft_input.clear();
                        });
                        this.send_message(prompt, cx);
                    });
                },
            )
        };

        // Input area panel
        let input_area = div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(div().flex_1().p(spacing::sm()).child(
                Input::new(&input_state).w_full().h_full().max_h(px(200.0)).disabled(is_loading),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .px(spacing::md())
                    .py(spacing::xs())
                    .child(send_or_stop_button),
            );

        // Resizable split between messages and input
        let split_panel = v_resizable("ai-chat-split")
            .child(
                resizable_panel()
                    .size(px(400.0))
                    .size_range(px(100.0)..px(2000.0))
                    .child(message_list),
            )
            .child(
                resizable_panel().size(px(160.0)).size_range(px(80.0)..px(600.0)).child(input_area),
            );

        // Disabled banner
        let disabled_banner = if !ai_enabled {
            Some(
                div()
                    .p(spacing::md())
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child("AI is disabled. Enable it in Settings to use the AI chat."),
            )
        } else {
            None
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .child(header)
            .children(error_banner)
            .children(disabled_banner)
            .child(div().flex_1().flex().flex_col().min_h(px(0.0)).child(split_panel))
    }
}

fn render_entry(entry: &AiChatEntry, is_loading: bool, cx: &App) -> AnyElement {
    match entry {
        AiChatEntry::Turn(turn) => render_turn(turn, is_loading, cx),
        AiChatEntry::SystemMessage(msg) => div()
            .px(spacing::md())
            .py(spacing::sm())
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(msg.content.clone())
            .into_any_element(),
        AiChatEntry::LegacyMessage(msg) => {
            let color = match msg.role {
                ChatRole::User => cx.theme().foreground,
                ChatRole::Assistant => cx.theme().primary,
                ChatRole::System => cx.theme().muted_foreground,
            };
            div()
                .px(spacing::md())
                .py(spacing::sm())
                .text_sm()
                .text_color(color)
                .child(format!("{}: {}", msg.role.label(), msg.content))
                .into_any_element()
        }
    }
}

fn render_turn(turn: &AiTurn, is_loading: bool, cx: &App) -> AnyElement {
    // User message — bordered card
    let user_msg = div()
        .px(spacing::md())
        .py(spacing::sm())
        .bg(cx.theme().sidebar)
        .border_1()
        .border_color(cx.theme().border)
        .rounded(borders::radius_sm())
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child("You"),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().foreground)
                .child(turn.user_message.content.clone()),
        );

    // Assistant message — flat, no border
    let assistant_section = match &turn.assistant_message {
        Some(msg) if !msg.content.is_empty() => Some(
            div()
                .px(spacing::md())
                .py(spacing::sm())
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().primary)
                        .child("Assistant"),
                )
                .child(
                    div().text_sm().text_color(cx.theme().foreground).child(msg.content.clone()),
                ),
        ),
        Some(_) if is_loading => {
            // Empty assistant message while loading — show spinner
            Some(
                div().px(spacing::md()).py(spacing::sm()).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::sm())
                        .child(Spinner::new().xsmall())
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Thinking..."),
                        ),
                ),
            )
        }
        _ => None,
    };

    div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .child(user_msg)
        .children(assistant_section)
        .into_any_element()
}
