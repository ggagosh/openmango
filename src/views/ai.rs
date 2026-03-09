use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Disableable as _;
use gpui_component::Sizable as _;
use gpui_component::button::ButtonVariants as _;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_component::scroll::{Scrollbar, ScrollbarAxis};
use gpui_component::spinner::Spinner;
use gpui_component::text::TextViewStyle;

use uuid::Uuid;

use crate::ai::bridge::AiBridge;
use crate::ai::budget::trim_history_for_context;
use crate::ai::context::build_ai_context;
use crate::ai::model_registry::{self, ModelCache};
use crate::ai::provider::{AiGenerationRequest, generate_text_streaming};
use crate::ai::safety::SafetyTier;
use crate::ai::telemetry::AiRequestSpan;
use crate::ai::tools::{MongoContext, StreamEvent};
use crate::ai::{
    AiChatEntry, AiTurn, ChatMessage, ChatMessageTone, ChatRole, ContentBlock, ToolActivity,
    ToolActivityStatus,
};
use crate::components::Button;
use crate::state::{AiProvider, AppState};
use crate::theme::{islands, spacing};
use gpui_component::{Icon, IconName, Size};

pub struct AiView {
    state: Entity<AppState>,
    input_state: Option<Entity<InputState>>,
    input_subscription: Option<Subscription>,
    scroll_handle: ScrollHandle,
    last_entry_count: usize,
    was_loading: bool,
    /// When true, the user is interacting with the chat (toggling tools, etc.)
    /// and auto-scroll should be suppressed until new data arrives.
    user_interacted: bool,
    /// Number of unseen updates while the user is reading older messages.
    unseen_updates: usize,
    /// Fingerprint of the rendered timeline; changes only when visible content changes.
    last_timeline_revision: u64,
    /// Keep follow-latest active for a few frames after explicit jump/send.
    pending_follow_frames: u8,
    /// User's manual expand/collapse overrides for tool groups.
    /// Key = id of the first ToolActivity in the group.
    /// Absent = auto (expanded while running, collapsed when done).
    tool_group_overrides: HashMap<Uuid, bool>,
    last_seen_provider: AiProvider,
    _subscriptions: Vec<Subscription>,
    /// @-mention popup state
    mention_query: Option<String>,
    mention_filtered: Vec<String>,
    mention_selected_index: usize,
}

impl AiView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let last_seen_provider = state.read(cx).settings.ai.provider;
        model_registry::spawn_model_fetch(&state, cx);
        let subscriptions = vec![cx.observe(&state, |this: &mut Self, _, cx| {
            let current = this.state.read(cx).settings.ai.provider;
            if current != this.last_seen_provider {
                this.last_seen_provider = current;
                model_registry::spawn_model_fetch(&this.state, cx);
            }
            cx.notify();
        })];
        Self {
            state,
            input_state: None,
            input_subscription: None,
            scroll_handle: ScrollHandle::new(),
            last_entry_count: 0,
            was_loading: false,
            user_interacted: false,
            unseen_updates: 0,
            last_timeline_revision: 0,
            pending_follow_frames: 0,
            tool_group_overrides: HashMap::new(),
            last_seen_provider,
            _subscriptions: subscriptions,
            mention_query: None,
            mention_filtered: Vec::new(),
            mention_selected_index: 0,
        }
    }

    fn mark_user_interaction(&mut self, cx: &mut Context<Self>) {
        if !self.user_interacted {
            self.user_interacted = true;
        }
        cx.notify();
    }

    fn is_near_latest(&self) -> bool {
        let max_offset_y = f32::from(self.scroll_handle.max_offset().height);
        if max_offset_y <= 0.5 {
            return true;
        }

        let offset_y = f32::from(self.scroll_handle.offset().y);
        let latest_offset_y = -max_offset_y;
        (offset_y - latest_offset_y).abs() <= 6.0
    }

    fn scroll_to_latest(&self) {
        self.scroll_handle.scroll_to_bottom();
    }

    fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.user_interacted = false;
        self.unseen_updates = 0;
        self.pending_follow_frames = 3;
        self.scroll_to_latest();
        cx.notify();
    }

    fn detect_mention_trigger(&mut self, text: &str, cursor: usize, cx: &mut Context<Self>) {
        // Scan backward from cursor for `@` preceded by whitespace or at start
        let before = &text[..cursor.min(text.len())];
        let mut at_pos = None;
        for (i, c) in before.char_indices().rev() {
            if c == '@' && (i == 0 || before.as_bytes()[i - 1].is_ascii_whitespace()) {
                at_pos = Some(i);
                break;
            }
            if c.is_whitespace() {
                break;
            }
        }

        if let Some(pos) = at_pos {
            let query = &before[pos + 1..];
            self.mention_query = Some(query.to_string());
            self.filter_mention_collections(query, cx);
            self.mention_selected_index = 0;
        } else {
            self.mention_query = None;
            self.mention_filtered.clear();
        }
    }

    fn filter_mention_collections(&mut self, query: &str, cx: &Context<Self>) {
        let s = self.state.read(cx);
        let conn_id = match s.selected_connection_id() {
            Some(id) => id,
            None => {
                self.mention_filtered.clear();
                return;
            }
        };
        let active = match s.active_connection_by_id(conn_id) {
            Some(c) => c,
            None => {
                self.mention_filtered.clear();
                return;
            }
        };
        let db = match s.selected_database_name() {
            Some(db) => db,
            None => {
                self.mention_filtered.clear();
                return;
            }
        };
        let cols = match active.collections.get(&db) {
            Some(c) => c,
            None => {
                self.mention_filtered.clear();
                return;
            }
        };
        let already_mentioned = &s.ai_chat.mentioned_collections;
        let query_lower = query.to_lowercase();
        self.mention_filtered = cols
            .iter()
            .filter(|c| !already_mentioned.contains(c))
            .filter(|c| query_lower.is_empty() || c.to_lowercase().contains(&query_lower))
            .take(10)
            .cloned()
            .collect();
    }

    /// Remove pills whose `@collection` text is no longer present in the input.
    fn sync_mentions_with_text(&self, text: &str, cx: &mut Context<Self>) {
        let mentioned = self.state.read(cx).ai_chat.mentioned_collections.clone();
        let removed: Vec<String> = mentioned
            .iter()
            .filter(|col| {
                let needle = format!("@{col}");
                !text.contains(&needle)
            })
            .cloned()
            .collect();
        if !removed.is_empty() {
            self.state.update(cx, |s, _| {
                for col in &removed {
                    s.ai_chat.remove_mention(col);
                }
            });
        }
    }

    /// Recompute inline highlight ranges for all `@collection` tokens in the text.
    fn update_mention_highlights(
        &self,
        input: &Entity<InputState>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let mentioned = self.state.read(cx).ai_chat.mentioned_collections.clone();
        if mentioned.is_empty() {
            input.update(cx, |s, _| s.set_custom_highlights(Vec::new()));
            return;
        }
        let theme = cx.theme();
        let fg = theme.link;
        let bg = Hsla { a: 0.15, ..fg };
        let style = HighlightStyle {
            background_color: Some(bg),
            color: Some(fg),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let mut highlights = Vec::new();
        for col in &mentioned {
            let needle = format!("@{col}");
            let mut start = 0;
            while let Some(pos) = text[start..].find(&needle) {
                let abs = start + pos;
                let end = abs + needle.len();
                // Only match at word boundary (next char is whitespace, punctuation, or end)
                let at_boundary =
                    text[end..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_');
                if at_boundary {
                    highlights.push((abs..end, style));
                }
                start = abs + 1;
            }
        }
        highlights.sort_by_key(|(r, _)| r.start);
        input.update(cx, |s, _| s.set_custom_highlights(highlights));
    }

    fn confirm_mention(&mut self, cx: &mut Context<Self>) {
        let Some(collection) = self.mention_filtered.get(self.mention_selected_index).cloned()
        else {
            return;
        };

        self.state.update(cx, |s, _| {
            s.ai_chat.add_mention(collection.clone());
        });

        // Trigger on-demand schema fetch if not cached
        let fetch_key = {
            let s = self.state.read(cx);
            if let (Some(conn_id), Some(db)) =
                (s.selected_connection_id(), s.selected_database_name())
            {
                let key = crate::state::SessionKey::new(conn_id, &db, &collection);
                if s.collection_meta_stale(&key) && !s.is_collection_meta_inflight(&key) {
                    Some(key)
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(key) = fetch_key {
            crate::state::AppCommands::fetch_single_collection_meta(self.state.clone(), key, cx);
        }

        self.mention_query = None;
        self.mention_filtered.clear();
        cx.notify();
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
                    .submit_on_enter(true)
                    .clean_on_escape()
                    .placeholder("Ask AI Assistant...")
            });

            let state = self.state.clone();
            let sub =
                cx.subscribe_in(&input_state, window, move |view, entity, event, window, cx| {
                    match event {
                        InputEvent::Change => {
                            let text = entity.read(cx).value().to_string();
                            let cursor = entity.read(cx).cursor();

                            // If mention popup was visible and whitespace was just
                            // inserted (Enter → \n, Tab → spaces/\t), complete the
                            // @mention and confirm.
                            if view.mention_query.is_some()
                                && !view.mention_filtered.is_empty()
                                && cursor > 0
                                && text.as_bytes()[cursor - 1].is_ascii_whitespace()
                                && let Some(col) =
                                    view.mention_filtered.get(view.mention_selected_index).cloned()
                            {
                                // Strip the inserted whitespace, then find @trigger
                                let ws_start = text[..cursor]
                                    .rfind(|c: char| !c.is_ascii_whitespace())
                                    .map_or(0, |p| p + 1);
                                let before_ws = &text[..ws_start];
                                if let Some(at_pos) = find_at_trigger(before_ws) {
                                    let replacement = format!("@{} ", col);
                                    let after = &text[cursor..];
                                    let new_text =
                                        format!("{}{}{}", &text[..at_pos], replacement, after,);
                                    let new_cursor_byte = at_pos + replacement.len();
                                    entity.update(cx, |input, cx| {
                                        input.set_value(new_text.clone(), window, cx);
                                        let before_cursor = &new_text[..new_cursor_byte];
                                        let line = before_cursor.matches('\n').count() as u32;
                                        let last_nl =
                                            before_cursor.rfind('\n').map_or(0, |p| p + 1);
                                        let character =
                                            before_cursor[last_nl..].chars().count() as u32;
                                        input.set_cursor_position(
                                            gpui_component::input::Position::new(line, character),
                                            window,
                                            cx,
                                        );
                                    });
                                    view.confirm_mention(cx);
                                    view.update_mention_highlights(entity, &new_text, cx);
                                    return;
                                }
                            }

                            view.detect_mention_trigger(&text, cursor, cx);
                            view.sync_mentions_with_text(&text, cx);
                            view.update_mention_highlights(entity, &text, cx);
                        }
                        InputEvent::Blur => {
                            let raw = entity.read(cx).value().to_string();
                            state.update(cx, |s, _| {
                                s.ai_chat.draft_input = raw;
                            });
                        }
                        InputEvent::PressEnter { secondary: false } => {
                            let can_submit = {
                                let s = state.read(cx);
                                s.settings.ai.enabled
                                    && !s.ai_chat.is_loading
                                    && s.current_ai_session_key().is_some()
                            };

                            let prompt = entity.read(cx).value().to_string().trim().to_string();
                            if !prompt.is_empty() && can_submit {
                                // Take mentions before clearing input (clear triggers Change
                                // which would strip them via sync_mentions_with_text).
                                let mentioned = state.update(cx, |s, _| s.ai_chat.take_mentions());
                                entity.update(cx, |input, cx| {
                                    input.set_custom_highlights(Vec::new());
                                    input.set_value(String::new(), window, cx);
                                });
                                state.update(cx, |s, _| {
                                    s.ai_chat.draft_input.clear();
                                });
                                view.send_message_with_mentions(prompt, mentioned, cx);
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

    fn send_message_with_mentions(
        &mut self,
        prompt: String,
        mentioned: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let ai_settings = self.state.read(cx).settings.ai.clone();

        let cancel_flag = Arc::new(AtomicBool::new(false));

        // User-submitted turns should always pin the timeline to the latest message.
        self.user_interacted = false;
        self.unseen_updates = 0;
        self.pending_follow_frames = 3;

        // Begin the turn and streaming response placeholder
        self.state.update(cx, |state, cx| {
            state.ai_chat.begin_turn(&prompt);
            state.ai_chat.is_loading = true;
            state.ai_chat.cancel_flag = Some(cancel_flag.clone());
            cx.notify();
        });
        self.scroll_to_latest();

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
        let system_prompt = build_ai_context(self.state.read(cx), &mentioned);
        log::debug!(
            "[ai-chat] system_prompt len={} history_msgs={}",
            system_prompt.len(),
            history.len()
        );
        log::debug!("[ai-chat] system_prompt:\n{system_prompt}");
        trim_history_for_context(&mut history, system_prompt.len(), None);

        let tool_ctx = {
            let s = self.state.read(cx);
            s.selected_connection_id().and_then(|id| {
                let client = s.active_connection_client(id)?;
                let db = s.selected_database_name()?;
                let col = s.selected_collection_name();
                Some(MongoContext { client, database: db, collection: col, event_tx: None })
            })
        };

        let request = AiGenerationRequest { system_prompt, history, user_prompt: prompt };

        let provider_label = ai_settings.provider.label().to_string();
        let model_label = ai_settings.model.clone();
        let session_label = turn_id.to_string();

        // Channel for streaming deltas
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();

        let task = cx.background_spawn(async move {
            AiBridge::block_on(async move {
                generate_text_streaming(&ai_settings, request, tool_ctx, tx).await
            })
        });

        let state = self.state.clone();
        let cancel_for_poll = cancel_flag;
        cx.spawn(async move |_view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let span = AiRequestSpan::start(&provider_label, &model_label, &session_label);
            const MAX_EVENTS_PER_FLUSH: usize = 24;
            const MAX_TEXT_CHARS_PER_FLUSH: usize = 640;

            let mut cancelled = false;

            // Poll channel for events while the background task runs
            let mut pending_events: Vec<StreamEvent> = Vec::new();

            let flush_pending = |pending: &mut Vec<StreamEvent>, cx: &mut AsyncApp| {
                if pending.is_empty() {
                    return;
                }
                let merged = coalesce_stream_events(std::mem::take(pending));
                let _ = cx.update(|cx| {
                    state.update(cx, |s, cx| {
                        for event in merged {
                            handle_stream_event(s, message_id, event);
                        }
                        cx.notify();
                    });
                });
            };

            loop {
                match rx.try_recv() {
                    Ok(event) => {
                        if cancel_for_poll.load(Ordering::Relaxed) {
                            cancelled = true;
                            break;
                        }
                        let mut events_in_batch = 0usize;
                        let mut text_chars_in_batch = 0usize;
                        let mut maybe_event = Some(event);
                        while let Some(event) = maybe_event.take() {
                            if let StreamEvent::TextDelta(text) = &event {
                                text_chars_in_batch =
                                    text_chars_in_batch.saturating_add(text.len());
                            }
                            pending_events.push(event);
                            events_in_batch = events_in_batch.saturating_add(1);

                            if events_in_batch >= MAX_EVENTS_PER_FLUSH
                                || text_chars_in_batch >= MAX_TEXT_CHARS_PER_FLUSH
                            {
                                break;
                            }
                            maybe_event = rx.try_recv().ok();
                        }
                        flush_pending(&mut pending_events, cx);
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        if cancel_for_poll.load(Ordering::Relaxed) {
                            cancelled = true;
                            break;
                        }
                        flush_pending(&mut pending_events, cx);
                        gpui::Timer::after(std::time::Duration::from_millis(16)).await;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        flush_pending(&mut pending_events, cx);
                        break;
                    }
                }
            }

            if cancelled {
                // Drop the task without awaiting — it may be blocked on a
                // confirmation oneshot that will never be answered.
                drop(task);
                span.finish_err(crate::ai::AiErrorKind::Cancelled);
                let _ = cx.update(|cx| {
                    state.update(cx, |s, cx| {
                        s.ai_chat.is_loading = false;
                        s.ai_chat.cancel_flag = None;
                        s.ai_chat.current_turn_id = None;
                        cx.notify();
                    });
                });
                return;
            }

            // Drain any remaining events
            while let Ok(event) = rx.try_recv() {
                pending_events.push(event);
                if pending_events.len() >= MAX_EVENTS_PER_FLUSH {
                    flush_pending(&mut pending_events, cx);
                }
            }
            flush_pending(&mut pending_events, cx);

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
                            s.ai_chat.clear_error();
                            s.ai_chat.fail_turn_response(message_id, msg);
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
        let appearance = app_state.settings.appearance.clone();
        let ai_chat = &app_state.ai_chat;
        let ai_enabled = app_state.settings.ai.enabled;
        let is_loading = ai_chat.is_loading;
        let session_key = app_state.current_ai_session_key();
        let streaming_turn_id = ai_chat.current_turn_id;
        let current_provider = app_state.settings.ai.provider;
        let current_model = app_state.settings.ai.model.clone();
        let selected_db = app_state.selected_database_name();
        let selected_collection = app_state.selected_collection_name();
        let session_ready = session_key.is_some();
        let input_focused = input_state.read(cx).focus_handle(cx).is_focused(window);
        let subtitle = match (&selected_db, &selected_collection) {
            (Some(db), Some(col)) => format!("{db}.{col}"),
            (Some(db), None) => format!("{db} (database selected)"),
            _ => "No active collection context".to_string(),
        };

        let panel_border = islands::ai_border(&appearance, cx);
        let muted_surface_bg = islands::ai_surface_muted_bg(&appearance, cx);

        // Header
        let header = {
            let close_state = state.clone();
            let header_buttons = div().flex().items_center().gap(px(6.0));

            let has_entries = !ai_chat.entries.is_empty();
            let header_buttons = if is_loading {
                let view = cx.entity();
                header_buttons.child(
                    Button::new("stop-gen")
                        .ghost()
                        .compact()
                        .icon(Icon::new(IconName::CircleX).xsmall())
                        .tooltip("Stop generation")
                        .on_click(move |_, _, cx| {
                            view.update(cx, |this, cx| {
                                this.stop_generation(cx);
                            });
                        }),
                )
            } else {
                header_buttons
            };

            let header_buttons = if has_entries && !is_loading {
                let clear_state = state.clone();
                header_buttons.child(
                    Button::new("clear-chat")
                        .ghost()
                        .compact()
                        .icon(Icon::new(IconName::Delete).xsmall())
                        .tooltip("Clear chat")
                        .on_click(move |_, _, cx| {
                            clear_state.update(cx, |state, cx| {
                                state.ai_chat.clear_chat();
                                cx.notify();
                            });
                        }),
                )
            } else {
                header_buttons
            };

            let close_button = Button::new("ai-panel-close")
                .ghost()
                .icon(Icon::new(IconName::Close).xsmall())
                .tooltip("Close AI panel")
                .on_click(move |_, _, cx| {
                    close_state.update(cx, |state, cx| {
                        state.toggle_ai_panel(cx);
                    });
                });

            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(46.0))
                .px(spacing::md())
                .bg(islands::ai_header_bg(&appearance, cx))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child("AI Chat"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::xs())
                        .child(header_buttons)
                        .child(close_button),
                )
        };

        let status_rows: Vec<AnyElement> = Vec::new();

        // Message list with manual scroll for auto-scroll-to-bottom
        let entries = ai_chat.entries.clone();
        let timeline_revision = timeline_revision(&entries);
        let content_changed = timeline_revision != self.last_timeline_revision;
        if self.user_interacted
            && self.unseen_updates > 0
            && self.scroll_handle.max_offset().height > px(0.5)
            && self.is_near_latest()
        {
            self.user_interacted = false;
            self.unseen_updates = 0;
        }
        let entry_count = entries.len();
        if self.user_interacted && content_changed {
            self.unseen_updates = self.unseen_updates.saturating_add(1).min(999);
        } else if !self.user_interacted {
            self.unseen_updates = 0;
        }

        let should_scroll = self.pending_follow_frames > 0
            || (!self.user_interacted && (content_changed || self.was_loading));
        self.last_entry_count = entry_count;
        self.last_timeline_revision = timeline_revision;
        self.was_loading = is_loading;

        if should_scroll {
            self.scroll_to_latest();
            self.pending_follow_frames = self.pending_follow_frames.saturating_sub(1);
        }

        let view_entity = cx.entity();
        let empty_title =
            if session_ready { "Ask about your data" } else { "AI stays open while you work" };
        let empty_subtitle = if session_ready {
            format!("Connected to {subtitle}. Ask for queries, indexes, or summaries.")
        } else {
            "Select a collection to unlock database-aware answers and actions.".to_string()
        };
        let empty_features = if session_ready {
            vec![
                ("Explain schema", "Break down fields, relationships, and document structure."),
                (
                    "Draft queries",
                    "Turn natural language into filters, projections, and pipelines.",
                ),
                ("Review indexes", "Spot missing indexes and explain likely query tradeoffs."),
            ]
        } else {
            vec![
                ("Keep context nearby", "Leave chat open while switching tabs and tools."),
                (
                    "Grounded when ready",
                    "Open any collection to connect the assistant to your data.",
                ),
                (
                    "One place to think",
                    "Use one panel for analysis, drafting, and follow-up questions.",
                ),
            ]
        };
        let empty_state = div()
            .flex()
            .flex_col()
            .items_start()
            .justify_center()
            .size_full()
            .px(px(28.0))
            .py(px(32.0))
            .gap(spacing::lg())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(spacing::xs())
                    .w_full()
                    .max_w(px(360.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().foreground)
                            .child(empty_title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(empty_subtitle),
                    ),
            )
            .child(
                div().flex().flex_col().gap(spacing::sm()).w_full().max_w(px(360.0)).children(
                    empty_features
                        .into_iter()
                        .map(|(label, hint)| render_empty_feature(label, hint, &appearance, cx)),
                ),
            );

        let scroll_handle = self.scroll_handle.clone();
        let on_scroll_view = view_entity.clone();
        let on_drag_view = view_entity.clone();
        let message_list = div()
            .size_full()
            .overflow_hidden()
            .relative()
            .bg(islands::ai_shell_bg(&appearance, cx).opacity(0.72))
            .child(
                div()
                    .id("ai-chat-scroll")
                    .flex()
                    .flex_col()
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&scroll_handle)
                    .on_scroll_wheel(move |_, _, cx| {
                        on_scroll_view.update(cx, |this, cx| {
                            this.mark_user_interaction(cx);
                        });
                    })
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        on_drag_view.update(cx, |this, cx| {
                            this.mark_user_interaction(cx);
                        });
                    })
                    .child({
                        if entries.is_empty() {
                            div().size_full().child(empty_state)
                        } else {
                            let blocks = group_entries(&entries);

                            // Helper: compute expand state for a tool group
                            let mut overrides_to_remove = Vec::new();
                            let compute_expand =
                                |tools: &[&ToolActivity],
                                 overrides: &HashMap<Uuid, bool>,
                                 removals: &mut Vec<Uuid>| {
                                    let key = tools[0].id;
                                    let any_running = tools.iter().any(|t| {
                                        matches!(
                                            t.status,
                                            ToolActivityStatus::Running
                                                | ToolActivityStatus::AwaitingConfirmation { .. }
                                        )
                                    });
                                    match overrides.get(&key) {
                                        Some(&val) => {
                                            if any_running && !val {
                                                removals.push(key);
                                                true
                                            } else {
                                                val
                                            }
                                        }
                                        None => any_running,
                                    }
                                };

                            let mut rendered: Vec<AnyElement> = Vec::with_capacity(blocks.len());
                            for block in &blocks {
                                let el = match block {
                                    RenderBlock::Turn { turn, tools } => {
                                        let tool_section = if !tools.is_empty() {
                                            let group_key = tools[0].id;
                                            let expanded = compute_expand(
                                                tools,
                                                &self.tool_group_overrides,
                                                &mut overrides_to_remove,
                                            );
                                            Some(render_tool_group(
                                                tools,
                                                expanded,
                                                group_key,
                                                view_entity.clone(),
                                                state.clone(),
                                                &appearance,
                                                window,
                                                cx,
                                            ))
                                        } else {
                                            None
                                        };
                                        let reports: Vec<(String, Vec<crate::ai::ReportSheet>)> =
                                            tools
                                                .iter()
                                                .filter_map(|t| match &t.result_block {
                                                    Some(ContentBlock::Report {
                                                        title,
                                                        sheets,
                                                    }) => Some((title.clone(), sheets.clone())),
                                                    _ => None,
                                                })
                                                .collect();
                                        render_turn(
                                            turn,
                                            tool_section,
                                            TurnReportContext { reports, state: state.clone() },
                                            streaming_turn_id == Some(turn.id),
                                            &appearance,
                                            window,
                                            cx,
                                        )
                                    }
                                    RenderBlock::ToolGroup(tools) => {
                                        let group_key = tools[0].id;
                                        let expanded = compute_expand(
                                            tools,
                                            &self.tool_group_overrides,
                                            &mut overrides_to_remove,
                                        );
                                        render_tool_group(
                                            tools,
                                            expanded,
                                            group_key,
                                            view_entity.clone(),
                                            state.clone(),
                                            &appearance,
                                            window,
                                            cx,
                                        )
                                    }
                                    RenderBlock::Other(entry) => match entry {
                                        AiChatEntry::SystemMessage(msg) => {
                                            render_status_message(msg, &appearance, cx)
                                        }
                                        AiChatEntry::LegacyMessage(msg) => {
                                            let color = match msg.role {
                                                ChatRole::User => cx.theme().foreground,
                                                ChatRole::Assistant => cx.theme().primary,
                                                ChatRole::System => cx.theme().muted_foreground,
                                            };
                                            div()
                                                .px(spacing::md())
                                                .py(spacing::sm())
                                                .bg(muted_surface_bg)
                                                .rounded(islands::radius_sm(&appearance))
                                                .border_1()
                                                .border_color(panel_border)
                                                .text_sm()
                                                .text_color(color)
                                                .child(format!(
                                                    "{}: {}",
                                                    msg.role.label(),
                                                    msg.content,
                                                ))
                                                .into_any_element()
                                        }
                                        _ => div().into_any_element(),
                                    },
                                };
                                rendered.push(el);
                            }

                            for key in overrides_to_remove {
                                self.tool_group_overrides.remove(&key);
                            }

                            div()
                                .flex()
                                .flex_col()
                                .p(spacing::md())
                                .gap(spacing::md())
                                .children(rendered)
                                .child(div().h(px(18.0)))
                        }
                    }),
            )
            .child(
                div().absolute().top_0().left_0().right_0().bottom_0().child(
                    Scrollbar::new(&scroll_handle)
                        .id("ai-chat-scrollbar")
                        .axis(ScrollbarAxis::Vertical),
                ),
            );
        let message_list = if self.user_interacted && self.unseen_updates > 0 {
            let jump_view = cx.entity();
            message_list.child(
                div().absolute().right(px(12.0)).bottom(px(12.0)).child(
                    Button::new("ai-jump-latest")
                        .primary()
                        .compact()
                        .label(format!("Jump to latest ({})", self.unseen_updates))
                        .on_click(move |_, _, cx| {
                            jump_view.update(cx, |this, cx| {
                                this.jump_to_latest(cx);
                            });
                        }),
                ),
            )
        } else {
            message_list
        };

        // Model selector dropdown — shows only current provider's models
        let model_selector = {
            let selector_label =
                compact_label(&current_provider.model_display_name(&current_model), 24);
            let state_for_menu = state.clone();
            gpui_component::button::Button::new("ai-model-selector")
                .ghost()
                .compact()
                .label(selector_label)
                .dropdown_caret(true)
                .rounded(islands::radius_sm(&appearance))
                .with_size(Size::Small)
                .disabled(is_loading)
                .dropdown_menu_with_anchor(
                    Corner::TopLeft,
                    move |mut menu: PopupMenu, _window, _cx| {
                        let state_read = state_for_menu.read(_cx);
                        let provider = state_read.settings.ai.provider;
                        let active_model = state_read.settings.ai.model.clone();
                        let cached = &state_read.ai_chat.cached_models;

                        menu = menu.label(provider.label());

                        let models: Vec<String> = match provider {
                            AiProvider::Ollama => match cached {
                                ModelCache::Loaded(list) => {
                                    let mut m = list.clone();
                                    if !active_model.trim().is_empty() && !m.contains(&active_model)
                                    {
                                        m.push(active_model.clone());
                                        m.sort();
                                    }
                                    m
                                }
                                _ => {
                                    if !active_model.trim().is_empty() {
                                        vec![active_model.clone()]
                                    } else {
                                        vec![]
                                    }
                                }
                            },
                            _ => provider.model_options(&active_model),
                        };

                        // Show status hints for Ollama non-Loaded states
                        if provider == AiProvider::Ollama {
                            match cached {
                                ModelCache::Loading => {
                                    menu = menu.item(
                                        PopupMenuItem::new("Loading models...").disabled(true),
                                    );
                                }
                                ModelCache::Error(msg) => {
                                    let hint = if msg.len() > 60 {
                                        format!("{}...", &msg[..57])
                                    } else {
                                        msg.clone()
                                    };
                                    menu = menu.item(PopupMenuItem::new(hint).disabled(true));
                                }
                                ModelCache::NotFetched => {
                                    menu = menu.item(
                                        PopupMenuItem::new("Fetching models...").disabled(true),
                                    );
                                }
                                _ => {}
                            }
                        }

                        // Show NoKey hint for cloud providers
                        if !matches!(provider, AiProvider::Ollama)
                            && matches!(cached, ModelCache::NoKey)
                        {
                            menu = menu
                                .item(PopupMenuItem::new("Add API key in Settings").disabled(true));
                        }

                        for model in models {
                            let is_current = model == active_model;
                            let s = state_for_menu.clone();
                            let m = model.clone();
                            let display_name = provider.model_display_name(&model);
                            let note = AiProvider::model_note(&model);
                            let item = if let Some(note) = note {
                                let model_label = display_name.clone();
                                let note = note.to_string();
                                PopupMenuItem::element(move |_window, cx| {
                                    div()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().foreground)
                                                .child(model_label.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(note.clone()),
                                        )
                                })
                                .checked(is_current)
                            } else {
                                PopupMenuItem::new(display_name).checked(is_current)
                            };
                            menu = menu.item(item.on_click(move |_, _, cx| {
                                s.update(cx, |state, cx| {
                                    state.settings.ai.set_model(m.clone());
                                    state.save_settings();
                                    cx.notify();
                                });
                            }));
                        }
                        menu
                    },
                )
        };

        // Send/Stop icon button
        let send_or_stop_button = if is_loading {
            let stop_view = cx.entity();
            Button::new("send-stop")
                .danger()
                .compact()
                .icon(Icon::new(IconName::CircleX).xsmall())
                .tooltip("Stop generation")
                .on_click(move |_, _, cx| {
                    stop_view.update(cx, |this, cx| {
                        this.stop_generation(cx);
                    });
                })
        } else {
            let view = cx.entity();
            let input_state_for_submit = input_state.clone();
            let can_submit = ai_enabled && !is_loading && session_key.is_some();
            Button::new("send-message")
                .primary()
                .compact()
                .icon(Icon::new(IconName::ArrowUp).xsmall())
                .tooltip("Send (Enter)")
                .disabled(!can_submit)
                .on_click(move |_, window, cx| {
                    let prompt = input_state_for_submit.read(cx).value().to_string();
                    let prompt = prompt.trim().to_string();
                    if prompt.is_empty() {
                        return;
                    }
                    // Take mentions before clearing input
                    let mentioned = view.update(cx, |this, cx| {
                        this.state.update(cx, |s, _| s.ai_chat.take_mentions())
                    });
                    input_state_for_submit.update(cx, |input, cx| {
                        input.set_custom_highlights(Vec::new());
                        input.set_value(String::new(), window, cx);
                    });
                    view.update(cx, |this, cx| {
                        this.state.update(cx, |state, _cx| {
                            state.ai_chat.draft_input.clear();
                        });
                        this.send_message_with_mentions(prompt, mentioned, cx);
                    });
                })
        };

        let db_chip = match (&selected_db, &selected_collection) {
            (Some(db), Some(col)) => format!("{db}.{col}"),
            (Some(db), None) => format!("{db}.*"),
            _ => "No collection context".to_string(),
        };
        let context_chip_label = compact_label(&db_chip, 34);
        let composer_border =
            if input_focused { cx.theme().primary } else { panel_border.opacity(0.88) };

        // Mention pills above input
        let mentioned = self.state.read(cx).ai_chat.mentioned_collections.clone();
        let mention_pills: Option<AnyElement> = if mentioned.is_empty() {
            None
        } else {
            let state_for_pills = state.clone();
            let input_for_pills = input_state.clone();
            let pills: Vec<AnyElement> = mentioned
                .iter()
                .map(|col| {
                    let col_name = col.clone();
                    let st = state_for_pills.clone();
                    let inp = input_for_pills.clone();
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .px(spacing::xs())
                        .py(px(2.0))
                        .rounded(px(6.0))
                        .bg(cx.theme().primary.opacity(0.12))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().primary)
                                .child(format!("@{col_name}")),
                        )
                        .child(
                            div()
                                .id(ElementId::Name(format!("mention-remove-{col_name}").into()))
                                .cursor_pointer()
                                .child(
                                    Icon::new(IconName::Close)
                                        .xsmall()
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    move |_: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                                        cx.stop_propagation();
                                        let col = col_name.clone();
                                        // Strip @collection from input text
                                        let text = inp.read(cx).value().to_string();
                                        let needle = format!("@{col}");
                                        if let Some(pos) = text.find(&needle) {
                                            let end = pos + needle.len();
                                            // Also consume trailing space if present
                                            let end = if text.as_bytes().get(end) == Some(&b' ') {
                                                end + 1
                                            } else {
                                                end
                                            };
                                            let new_text =
                                                format!("{}{}", &text[..pos], &text[end..],);
                                            inp.update(cx, |input, cx| {
                                                input.set_value(new_text, window, cx);
                                            });
                                        }
                                        st.update(cx, |s, cx| {
                                            s.ai_chat.remove_mention(&col);
                                            cx.notify();
                                        });
                                    },
                                ),
                        )
                        .into_any_element()
                })
                .collect();
            Some(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(spacing::xs())
                    .px(px(2.0))
                    .children(pills)
                    .into_any_element(),
            )
        };

        // Mention popup — rendered between message_list and input_area to overlay chat
        let mention_popup: Option<AnyElement> = if self.mention_query.is_some()
            && !self.mention_filtered.is_empty()
        {
            let selected_idx = self.mention_selected_index;
            let view_for_popup = view_entity.clone();
            let items: Vec<AnyElement> = self
                .mention_filtered
                .iter()
                .enumerate()
                .map(|(i, col)| {
                    let is_selected = i == selected_idx;
                    let col_name = col.clone();
                    let v = view_for_popup.clone();
                    let bg = if is_selected {
                        cx.theme().primary.opacity(0.12)
                    } else {
                        gpui::transparent_black()
                    };
                    div()
                        .id(ElementId::Name(format!("mention-item-{i}").into()))
                        .flex()
                        .items_center()
                        .gap(spacing::sm())
                        .px(spacing::sm())
                        .py(spacing::xs())
                        .cursor_pointer()
                        .rounded(px(4.0))
                        .bg(bg)
                        .hover(|s: gpui::StyleRefinement| s.bg(cx.theme().secondary.opacity(0.2)))
                        .child(
                            Icon::new(IconName::Braces)
                                .xsmall()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(div().text_xs().text_color(cx.theme().foreground).child(col_name))
                        .on_mouse_down(
                            MouseButton::Left,
                            move |_: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
                                cx.stop_propagation();
                                v.update(cx, |this, cx| {
                                    this.mention_selected_index = i;
                                    this.confirm_mention(cx);
                                });
                            },
                        )
                        .into_any_element()
                })
                .collect();

            Some(
                div()
                    .id("mention-popup")
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .max_h(px(240.0))
                    .overflow_y_scroll()
                    .mx(spacing::md())
                    .py(spacing::xs())
                    .px(spacing::sm())
                    .bg(islands::ai_surface_bg(&appearance, cx))
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(islands::radius_sm(&appearance))
                    .shadow_md()
                    .children(items)
                    .into_any_element(),
            )
        } else {
            None
        };

        // Input area panel
        let mut input_area = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(spacing::sm())
            .mx(spacing::md())
            .mb(spacing::md())
            .p(px(6.0))
            .bg(islands::ai_surface_bg(&appearance, cx).opacity(0.96))
            .border_1()
            .border_color(composer_border)
            .rounded(islands::radius_md(&appearance));

        input_area = input_area.children(mention_pills);

        input_area = input_area
            .child(
                div().px(px(2.0)).py(px(2.0)).child(
                    Input::new(&input_state)
                        .xsmall()
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h(px(64.0)),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(spacing::sm())
                    .pt(px(2.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .min_w(px(0.0))
                            .gap(spacing::xs())
                            .child(model_selector)
                            .child(info_chip(
                                &context_chip_label,
                                if session_ready {
                                    cx.theme().muted_foreground
                                } else {
                                    cx.theme().warning
                                },
                            )),
                    )
                    .child(send_or_stop_button),
            );

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(islands::ai_shell_bg(&appearance, cx))
            .child(header)
            .children((!status_rows.is_empty()).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::sm())
                    .px(spacing::md())
                    .pt(spacing::sm())
                    .children(status_rows)
            }))
            .child(div().flex_1().min_h(px(0.0)).child(message_list))
            .children(mention_popup)
            .child(input_area)
    }
}

/// Find the byte position of an `@` trigger scanning backward from the end of `text`.
fn find_at_trigger(text: &str) -> Option<usize> {
    for (i, c) in text.char_indices().rev() {
        if c == '@' && (i == 0 || text.as_bytes()[i - 1].is_ascii_whitespace()) {
            return Some(i);
        }
        if c.is_whitespace() {
            return None;
        }
    }
    None
}

fn info_chip(label: &str, accent: Hsla) -> AnyElement {
    div()
        .px(spacing::xs())
        .py(px(2.0))
        .rounded(px(6.0))
        .bg(accent.opacity(0.08))
        .text_xs()
        .text_color(accent)
        .child(label.to_string())
        .into_any_element()
}

fn render_empty_feature(
    title: &str,
    hint: &str,
    appearance: &crate::state::AppearanceSettings,
    cx: &App,
) -> AnyElement {
    let title = title.to_string();
    let hint = hint.to_string();
    div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .w_full()
        .px(spacing::sm())
        .py(spacing::sm())
        .rounded(islands::radius_sm(appearance))
        .bg(islands::ai_surface_bg(appearance, cx).opacity(0.74))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .child(title),
        )
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(hint))
        .into_any_element()
}

fn compact_label(label: &str, max_chars: usize) -> String {
    if label.chars().count() <= max_chars {
        return label.to_string();
    }
    if max_chars <= 1 {
        return "…".to_string();
    }
    let compact: String = label.chars().take(max_chars - 1).collect();
    format!("{compact}…")
}

enum RenderBlock<'a> {
    Turn { turn: &'a AiTurn, tools: Vec<&'a ToolActivity> },
    ToolGroup(Vec<&'a ToolActivity>),
    Other(&'a AiChatEntry),
}

fn group_entries(entries: &[AiChatEntry]) -> Vec<RenderBlock<'_>> {
    let mut blocks: Vec<RenderBlock<'_>> = Vec::new();
    let mut tool_buf: Vec<&ToolActivity> = Vec::new();

    for entry in entries {
        if let AiChatEntry::ToolActivity(activity) = entry {
            tool_buf.push(activity);
            continue;
        }

        // Flush accumulated tools
        if !tool_buf.is_empty() {
            let tools = std::mem::take(&mut tool_buf);
            // Attach to preceding Turn if possible, otherwise standalone group
            if let Some(RenderBlock::Turn { tools: t, .. }) = blocks.last_mut() {
                t.extend(tools);
            } else {
                blocks.push(RenderBlock::ToolGroup(tools));
            }
        }

        match entry {
            AiChatEntry::Turn(turn) => {
                blocks.push(RenderBlock::Turn { turn, tools: Vec::new() });
            }
            _ => {
                blocks.push(RenderBlock::Other(entry));
            }
        }
    }

    // Flush trailing tools
    if !tool_buf.is_empty() {
        if let Some(RenderBlock::Turn { tools: t, .. }) = blocks.last_mut() {
            t.extend(tool_buf);
        } else {
            blocks.push(RenderBlock::ToolGroup(tool_buf));
        }
    }

    blocks
}

fn timeline_revision(entries: &[AiChatEntry]) -> u64 {
    fn tool_status_code(status: &ToolActivityStatus) -> u64 {
        match status {
            ToolActivityStatus::Running => 1,
            ToolActivityStatus::AwaitingConfirmation { .. } => 2,
            ToolActivityStatus::Completed => 3,
            ToolActivityStatus::Failed(_) => 4,
            ToolActivityStatus::Rejected => 5,
        }
    }

    let mut rev = (entries.len() as u64).wrapping_mul(0x9E37_79B1);
    for entry in entries {
        match entry {
            AiChatEntry::Turn(turn) => {
                rev = rev
                    .wrapping_mul(131)
                    .wrapping_add(turn.user_message.content.len() as u64)
                    .wrapping_add(turn.user_message.id.as_u128() as u64);
                if let Some(msg) = &turn.assistant_message {
                    rev = rev
                        .wrapping_mul(131)
                        .wrapping_add(msg.content.len() as u64)
                        .wrapping_add(msg.blocks.len() as u64)
                        .wrapping_add(match msg.tone {
                            ChatMessageTone::Normal => 1,
                            ChatMessageTone::Error => 2,
                        })
                        .wrapping_add(msg.id.as_u128() as u64);
                }
            }
            AiChatEntry::ToolActivity(activity) => {
                rev = rev
                    .wrapping_mul(131)
                    .wrapping_add(activity.id.as_u128() as u64)
                    .wrapping_add(activity.tool_name.len() as u64)
                    .wrapping_add(activity.args_preview.len() as u64)
                    .wrapping_add(activity.result_preview.as_ref().map_or(0, |s| s.len() as u64))
                    .wrapping_add(activity.result_block.as_ref().map_or(0, |_| 7))
                    .wrapping_add(tool_status_code(&activity.status));
            }
            AiChatEntry::SystemMessage(msg) | AiChatEntry::LegacyMessage(msg) => {
                rev = rev
                    .wrapping_mul(131)
                    .wrapping_add(msg.content.len() as u64)
                    .wrapping_add(match msg.tone {
                        ChatMessageTone::Normal => 1,
                        ChatMessageTone::Error => 2,
                    })
                    .wrapping_add(msg.id.as_u128() as u64);
            }
        }
    }

    rev
}

fn ai_block_gap() -> Pixels {
    spacing::sm()
}

fn ai_section_gap() -> Pixels {
    spacing::md()
}

fn ai_markdown_style(cx: &App) -> TextViewStyle {
    let code_block_style = gpui::StyleRefinement::default()
        .mt(spacing::xs())
        .mb(spacing::xs())
        .border_1()
        .border_color(cx.theme().border.opacity(0.82));

    TextViewStyle {
        paragraph_gap: rems(0.72),
        heading_base_font_size: px(13.0),
        highlight_theme: cx.theme().highlight_theme.clone(),
        is_dark: cx.theme().mode.is_dark(),
        code_block: code_block_style,
        ..TextViewStyle::default()
    }
    .heading_font_size(|level, base| {
        let scale = match level {
            1 => 1.42,
            2 => 1.28,
            3 => 1.16,
            _ => 1.0,
        };
        base * scale
    })
}

struct TurnReportContext {
    reports: Vec<(String, Vec<crate::ai::ReportSheet>)>,
    state: Entity<AppState>,
}

fn render_turn(
    turn: &AiTurn,
    tool_section: Option<AnyElement>,
    report_ctx: TurnReportContext,
    is_streaming: bool,
    appearance: &crate::state::AppearanceSettings,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let border = islands::ai_border(appearance, cx).opacity(0.82);
    let user_bg = cx.theme().primary.opacity(0.1);
    let assistant_bg = islands::ai_surface_bg(appearance, cx).opacity(0.88);

    let user_msg = div().flex().w_full().justify_end().child(
        div()
            .w_full()
            .max_w(px(820.0))
            .min_w(px(0.0))
            .px(spacing::md())
            .py(spacing::sm())
            .bg(user_bg)
            .border_1()
            .border_color(cx.theme().primary.opacity(0.26))
            .rounded(islands::radius_sm(appearance))
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().primary)
                    .child("You"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .min_w(px(0.0))
                    .child(turn.user_message.content.clone()),
            ),
    );

    let assistant_section = match &turn.assistant_message {
        Some(msg) if msg.tone == ChatMessageTone::Error => {
            let mut body = div().flex().flex_col().gap(ai_block_gap()).child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().danger)
                    .child("Error"),
            );
            if let Some(ts) = tool_section {
                body = body.child(ts);
            }
            body = body.child(
                div()
                    .text_sm()
                    .min_w(px(0.0))
                    .child(render_plain_text_lines(&msg.content, cx.theme().foreground)),
            );

            Some(
                div()
                    .px(spacing::md())
                    .py(spacing::sm())
                    .bg(cx.theme().danger.opacity(0.1))
                    .border_1()
                    .border_color(cx.theme().danger.opacity(0.42))
                    .rounded(islands::radius_sm(appearance))
                    .child(body),
            )
        }
        Some(msg) if is_streaming && !msg.content.is_empty() => {
            let mut body = div().flex().flex_col().gap(ai_block_gap()).child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().primary)
                    .child("Assistant"),
            );
            if let Some(ts) = tool_section {
                body = body.child(ts);
            }
            body = body
                .child(
                    div()
                        .min_w(px(0.0))
                        .child(render_plain_text_lines(&msg.content, cx.theme().foreground)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::xs())
                        .child(Spinner::new().xsmall())
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Streaming..."),
                        ),
                );

            Some(
                div()
                    .px(spacing::md())
                    .py(spacing::sm())
                    .bg(assistant_bg)
                    .border_1()
                    .border_color(border)
                    .rounded(islands::radius_sm(appearance))
                    .child(body),
            )
        }
        Some(msg) if !msg.content.is_empty() => {
            let md_style = ai_markdown_style(cx);
            let blocks = crate::components::ai_blocks::render_content_blocks_or_fallback(
                &format!("ai-md-{}", msg.id),
                msg,
                md_style,
                window,
                cx,
            );

            let mut body = div().flex().flex_col().gap(ai_block_gap()).child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().primary)
                    .child("Assistant"),
            );
            if let Some(ts) = tool_section {
                body = body.child(ts);
            }
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .min_w(px(0.0))
                    .child(div().flex().flex_col().gap(ai_block_gap()).children(blocks)),
            );

            if !report_ctx.reports.is_empty() {
                let buttons = render_report_download_buttons(
                    &report_ctx.reports,
                    &report_ctx.state,
                    &turn.id,
                    cx,
                );
                body = body.child(buttons);
            }

            Some(
                div()
                    .px(spacing::md())
                    .py(spacing::sm())
                    .bg(assistant_bg)
                    .border_1()
                    .border_color(border)
                    .rounded(islands::radius_sm(appearance))
                    .child(body),
            )
        }
        Some(_) if is_streaming => {
            let mut body = div().flex().flex_col().gap(ai_block_gap());
            if let Some(ts) = tool_section {
                body = body.child(ts);
            }
            body = body.child(
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
            );

            Some(
                div()
                    .px(spacing::md())
                    .py(spacing::sm())
                    .bg(assistant_bg)
                    .border_1()
                    .border_color(border)
                    .rounded(islands::radius_sm(appearance))
                    .child(body),
            )
        }
        _ => tool_section.map(|ts| {
            div()
                .px(spacing::md())
                .py(spacing::sm())
                .child(div().flex().flex_col().gap(ai_block_gap()).child(ts))
        }),
    };

    div()
        .flex()
        .flex_col()
        .gap(ai_section_gap())
        .child(user_msg)
        .children(assistant_section)
        .into_any_element()
}

fn render_status_message(
    msg: &ChatMessage,
    appearance: &crate::state::AppearanceSettings,
    cx: &App,
) -> AnyElement {
    let (title, border_color, bg, body_color) = if msg.tone == ChatMessageTone::Error {
        (
            "Error",
            cx.theme().danger.opacity(0.42),
            cx.theme().danger.opacity(0.1),
            cx.theme().foreground,
        )
    } else {
        (
            msg.role.label(),
            islands::ai_border(appearance, cx).opacity(0.78),
            islands::ai_surface_muted_bg(appearance, cx),
            cx.theme().muted_foreground,
        )
    };

    div()
        .px(spacing::md())
        .py(spacing::sm())
        .bg(bg)
        .border_1()
        .border_color(border_color)
        .rounded(islands::radius_sm(appearance))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if msg.tone == ChatMessageTone::Error {
                    cx.theme().danger
                } else {
                    cx.theme().muted_foreground
                })
                .child(title),
        )
        .child(div().text_xs().child(render_plain_text_lines(&msg.content, body_color)))
        .into_any_element()
}

fn render_plain_text_lines(text: &str, color: Hsla) -> AnyElement {
    let mut lines: Vec<AnyElement> = Vec::new();
    for line in text.lines() {
        let element = if line.is_empty() {
            div().h(px(10.0)).into_any_element()
        } else {
            div().text_color(color).whitespace_normal().child(line.to_string()).into_any_element()
        };
        lines.push(element);
    }

    if text.ends_with('\n') {
        lines.push(div().h(px(10.0)).into_any_element());
    }

    if lines.is_empty() {
        div().into_any_element()
    } else {
        div().flex().flex_col().gap(px(2.0)).children(lines).into_any_element()
    }
}

#[allow(clippy::too_many_arguments)]
fn render_tool_group(
    tools: &[&ToolActivity],
    expanded: bool,
    group_key: Uuid,
    view: Entity<AiView>,
    state: Entity<AppState>,
    appearance: &crate::state::AppearanceSettings,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let any_running = tools.iter().any(|t| {
        matches!(
            t.status,
            ToolActivityStatus::Running | ToolActivityStatus::AwaitingConfirmation { .. }
        )
    });
    let any_awaiting =
        tools.iter().any(|t| matches!(t.status, ToolActivityStatus::AwaitingConfirmation { .. }));

    // Header icon: spinner while running, chevron when done
    let header_icon = if any_running && !any_awaiting {
        Spinner::new().xsmall().into_any_element()
    } else if any_awaiting {
        Icon::new(IconName::TriangleAlert)
            .xsmall()
            .text_color(cx.theme().warning)
            .into_any_element()
    } else if expanded {
        Icon::new(IconName::ChevronDown)
            .xsmall()
            .text_color(cx.theme().muted_foreground)
            .into_any_element()
    } else {
        Icon::new(IconName::ChevronRight)
            .xsmall()
            .text_color(cx.theme().muted_foreground)
            .into_any_element()
    };

    // Header label
    let label = if any_awaiting {
        "Awaiting confirmation...".to_string()
    } else if any_running {
        if tools.len() == 1 {
            format!("Running {}...", display_tool_name(&tools[0].tool_name))
        } else {
            "Running tools...".to_string()
        }
    } else if tools.len() == 1 {
        format!("Used {}", display_tool_name(&tools[0].tool_name))
    } else {
        format!("Used {} tools", tools.len())
    };

    let header = div()
        .id(ElementId::Name(format!("tool-group-{group_key}").into()))
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::sm())
        .px(spacing::sm())
        .py(spacing::xs())
        .bg(islands::ai_surface_bg(appearance, cx).opacity(0.86))
        .border_1()
        .border_color(islands::ai_border(appearance, cx).opacity(0.78))
        .rounded(islands::radius_sm(appearance))
        .cursor_pointer()
        .hover(|s| s.bg(cx.theme().secondary.opacity(0.2)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .child(header_icon)
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label)),
        )
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!(
            "{} call{}",
            tools.len(),
            if tools.len() == 1 { "" } else { "s" }
        )))
        .on_mouse_down(MouseButton::Left, {
            move |_, _, cx| {
                cx.stop_propagation();
                view.update(cx, |this, cx| {
                    this.tool_group_overrides.insert(group_key, !expanded);
                    this.mark_user_interaction(cx);
                });
            }
        });

    // Interleave each tool's status row with its result block so results
    // appear directly under the tool that produced them. Keep spacing
    // deterministic by rendering each tool call in its own stack.
    let mut tool_elements: Vec<AnyElement> = Vec::new();
    for (i, t) in tools.iter().enumerate() {
        if !expanded {
            continue;
        }

        let mut item =
            div().flex().flex_col().gap(ai_block_gap()).px(spacing::xs()).py(spacing::xs());

        if i + 1 < tools.len() {
            item =
                item.pb(ai_block_gap()).border_b_1().border_color(cx.theme().border.opacity(0.35));
        }

        item = item.child(render_tool_row(t, state.clone(), appearance, cx));
        if let Some(block) = t.result_block.as_ref() {
            if let ContentBlock::Report { title, sheets } = block {
                let st = state.clone();
                let title_dl = title.clone();
                let sheets_dl = sheets.clone();
                let on_download: crate::components::ai_blocks::report::DownloadHandler =
                    Box::new(move |_, _, cx| {
                        download_report_as_excel(
                            st.clone(),
                            title_dl.clone(),
                            sheets_dl.clone(),
                            cx,
                        );
                    });
                item = item.child(crate::components::ai_blocks::report::render_report_preview(
                    title,
                    sheets,
                    ElementId::Name(format!("tool-result-{group_key}-rpt-{i}").into()),
                    Some(on_download),
                    cx,
                ));
            } else {
                let style = ai_markdown_style(cx);
                item = item.child(crate::components::ai_blocks::render_single_block(
                    &format!("tool-result-{group_key}"),
                    i,
                    block,
                    &style,
                    window,
                    cx,
                ));
            }
            if matches!(block, ContentBlock::DataTable { .. }) && t.tool_name == "find_documents" {
                let col_name = t.collection.clone().or_else(|| {
                    serde_json::from_str::<serde_json::Value>(&t.args_preview)
                        .ok()
                        .and_then(|v| v.get("collection")?.as_str().map(String::from))
                });
                let current_col = state.read(cx).selected_collection_name();
                if let Some(col) = col_name
                    && current_col.as_deref() != Some(col.as_str())
                {
                    let st = state.clone();
                    item = item.child(
                        div().flex().child(
                            Button::new(ElementId::Name(
                                format!("open-col-{group_key}-{i}").into(),
                            ))
                            .ghost()
                            .compact()
                            .icon(Icon::new(IconName::SquareTerminal).xsmall())
                            .label("Open Collection")
                            .on_click(move |_, _, cx| {
                                let col = col.clone();
                                let should_load = st.update(cx, |state, cx| {
                                    if let Some(db) = state.selected_database_name() {
                                        state.select_collection(db, col, cx);
                                        if let Some(key) = state.current_session_key() {
                                            state.clear_filter(&key);
                                        }
                                        cx.notify();
                                        true
                                    } else {
                                        false
                                    }
                                });
                                if should_load && let Some(key) = st.read(cx).current_session_key()
                                {
                                    crate::state::AppCommands::load_documents_for_session(
                                        st.clone(),
                                        key,
                                        cx,
                                    );
                                }
                            }),
                        ),
                    );
                }
            }
            if t.tool_name == "aggregate"
                && let Some(args_json) = &t.args_full
            {
                let mut row = div().flex().gap(spacing::xs());
                if let Some((col, stages)) = parse_pipeline_from_args(args_json) {
                    let st = state.clone();
                    let col_for_agg = col.clone();
                    let stages_for_agg = stages.clone();
                    row = row.child(
                        Button::new(ElementId::Name(format!("open-agg-{group_key}-{i}").into()))
                            .ghost()
                            .compact()
                            .icon(Icon::new(IconName::SquareTerminal).xsmall())
                            .label("Open in Aggregation")
                            .on_click(move |_, _, cx| {
                                let col = col_for_agg.clone();
                                let stages = stages_for_agg.clone();
                                st.update(cx, |state, cx| {
                                    if let Some(db) = state.selected_database_name() {
                                        if !col.is_empty() {
                                            state.select_collection(db, col, cx);
                                        }
                                        if let Some(key) = state.current_session_key() {
                                            state.set_collection_subview(
                                                &key,
                                                crate::state::CollectionSubview::Aggregation,
                                            );
                                            state.replace_pipeline_stages(&key, stages);
                                        }
                                        cx.notify();
                                    }
                                });
                            }),
                    );
                    let st2 = state.clone();
                    if let Some(content) = build_forge_aggregate_command(args_json) {
                        row = row.child(
                            Button::new(ElementId::Name(
                                format!("open-forge-{group_key}-{i}").into(),
                            ))
                            .ghost()
                            .compact()
                            .icon(Icon::new(IconName::SquareTerminal).xsmall())
                            .label("Open in Forge")
                            .on_click(move |_, _, cx| {
                                let content = content.clone();
                                st2.update(cx, |state, cx| {
                                    if let Some(conn_id) = state.selected_connection_id()
                                        && let Some(db) = state.selected_database_name()
                                    {
                                        state.open_forge_tab_with_content(conn_id, db, content, cx);
                                    }
                                });
                            }),
                        );
                    }
                }
                item = item.child(row);
            }
        }
        tool_elements.push(item.into_any_element());
    }

    div()
        .flex()
        .flex_col()
        .gap(ai_block_gap())
        .child(header)
        .children(tool_elements)
        .into_any_element()
}

fn render_tool_row(
    activity: &ToolActivity,
    state: Entity<AppState>,
    appearance: &crate::state::AppearanceSettings,
    cx: &App,
) -> AnyElement {
    let display_name = display_tool_name(&activity.tool_name);

    match &activity.status {
        ToolActivityStatus::AwaitingConfirmation { .. } => {
            render_confirmation_card(activity, state, appearance, cx)
        }
        ToolActivityStatus::Rejected => div()
            .flex()
            .items_center()
            .gap(spacing::sm())
            .py(spacing::xs())
            .child(Icon::new(IconName::Close).xsmall().text_color(cx.theme().warning))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(format!("{display_name} rejected")),
            )
            .into_any_element(),
        status => {
            let (icon_el, suffix) = match status {
                ToolActivityStatus::Running => {
                    (Spinner::new().xsmall().into_any_element(), "running...")
                }
                ToolActivityStatus::Completed => (
                    Icon::new(IconName::Check)
                        .xsmall()
                        .text_color(cx.theme().success)
                        .into_any_element(),
                    "completed",
                ),
                ToolActivityStatus::Failed(reason) => {
                    return div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .p(spacing::sm())
                        .border_1()
                        .border_color(cx.theme().danger)
                        .rounded(islands::radius_sm(appearance))
                        .bg(cx.theme().danger.opacity(0.06))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(spacing::sm())
                                .child(
                                    Icon::new(IconName::TriangleAlert)
                                        .xsmall()
                                        .text_color(cx.theme().danger),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(cx.theme().danger)
                                        .child(format!("{display_name} blocked")),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(reason.clone()),
                        )
                        .into_any_element();
                }
                _ => unreachable!(),
            };

            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .py(spacing::xs())
                .child(icon_el)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{display_name} {suffix}")),
                )
                .into_any_element()
        }
    }
}

fn handle_stream_event(state: &mut AppState, message_id: Uuid, event: StreamEvent) {
    match event {
        StreamEvent::TextDelta(delta) => {
            state.ai_chat.append_turn_delta(message_id, &delta);
        }
        StreamEvent::ToolCallStart { name, args_preview, args_full } => {
            state.ai_chat.push_tool_start(name, args_preview, args_full);
        }
        StreamEvent::ToolCallEnd { name, result_preview, result_json } => {
            state.ai_chat.complete_tool(&name, result_preview, result_json);
        }
        StreamEvent::ConfirmationRequired {
            tool_name,
            description,
            tier,
            preview,
            response_tx,
        } => {
            state.ai_chat.set_tool_awaiting_confirmation(
                &tool_name,
                description,
                tier,
                preview,
                response_tx,
            );
        }
    }
}

fn coalesce_stream_events(events: Vec<StreamEvent>) -> Vec<StreamEvent> {
    let mut merged: Vec<StreamEvent> = Vec::with_capacity(events.len());
    for event in events {
        match event {
            StreamEvent::TextDelta(delta) => {
                if delta.is_empty() {
                    continue;
                }
                if let Some(StreamEvent::TextDelta(current)) = merged.last_mut() {
                    current.push_str(&delta);
                } else {
                    merged.push(StreamEvent::TextDelta(delta));
                }
            }
            other => merged.push(other),
        }
    }
    merged
}

fn confirmation_button_label(tool_name: &str) -> &'static str {
    match tool_name {
        "insert_documents" => "Insert",
        "update_documents" => "Update",
        "delete_documents" => "Delete",
        "create_index" => "Create Index",
        "drop_index" => "Drop Index",
        _ => "Approve",
    }
}

fn is_danger_tool(tool_name: &str) -> bool {
    matches!(tool_name, "update_documents" | "delete_documents" | "drop_index")
}

fn render_confirmation_card(
    activity: &ToolActivity,
    state: Entity<AppState>,
    appearance: &crate::state::AppearanceSettings,
    cx: &App,
) -> AnyElement {
    let ToolActivityStatus::AwaitingConfirmation {
        ref description,
        ref tier,
        ref preview,
        ref response_tx,
    } = activity.status
    else {
        unreachable!();
    };
    let activity_id = activity.id;
    let tool_name = &activity.tool_name;
    let is_blocked = matches!(tier, SafetyTier::Blocked);
    let danger = is_blocked || is_danger_tool(tool_name);

    let tier_icon = match tier {
        SafetyTier::Blocked => {
            Icon::new(IconName::TriangleAlert).xsmall().text_color(cx.theme().danger)
        }
        SafetyTier::AlwaysConfirm => {
            Icon::new(IconName::TriangleAlert).xsmall().text_color(cx.theme().warning)
        }
        _ => Icon::new(IconName::Info).xsmall().text_color(cx.theme().primary),
    };

    // Summary line
    let summary = if is_blocked {
        description.clone()
    } else if preview.affected_count > 0 {
        format!("{} {} documents in {}", description, preview.affected_count, preview.collection)
    } else {
        format!("{} on {}", description, preview.collection)
    };

    let confirm_label = if is_blocked { "Override" } else { confirmation_button_label(tool_name) };

    let approve_tx = response_tx.clone();
    let reject_tx = response_tx.clone();
    let approve_state = state.clone();
    let reject_state = state;

    let confirm_id: SharedString = format!("confirm-{activity_id}").into();
    let cancel_id: SharedString = format!("cancel-{activity_id}").into();

    let confirm_button = if danger {
        Button::new(confirm_id).danger().compact().label(confirm_label).on_click(move |_, _, cx| {
            approve_tx.respond(true);
            approve_state.update(cx, |s, cx| {
                s.ai_chat.approve_tool_confirmation(activity_id);
                cx.notify();
            });
        })
    } else {
        Button::new(confirm_id).primary().compact().label(confirm_label).on_click(
            move |_, _, cx| {
                approve_tx.respond(true);
                approve_state.update(cx, |s, cx| {
                    s.ai_chat.approve_tool_confirmation(activity_id);
                    cx.notify();
                });
            },
        )
    };

    let cancel_button =
        Button::new(cancel_id).ghost().compact().label("Cancel").on_click(move |_, _, cx| {
            reject_tx.respond(false);
            reject_state.update(cx, |s, cx| {
                s.ai_chat.reject_tool_confirmation(activity_id);
                cx.notify();
            });
        });

    // Sample docs preview (truncated JSON) — skip for blocked ops (not useful)
    let sample_preview = if !is_blocked && !preview.sample_docs.is_empty() {
        let sample_text = preview
            .sample_docs
            .iter()
            .take(3)
            .filter_map(|doc| serde_json::to_string_pretty(doc).ok())
            .collect::<Vec<_>>()
            .join("\n");
        let truncated = if sample_text.len() > 500 {
            format!("{}...", &sample_text[..sample_text.floor_char_boundary(500)])
        } else {
            sample_text
        };
        Some(
            div()
                .mt(spacing::sm())
                .p(spacing::sm())
                .bg(islands::ai_surface_muted_bg(appearance, cx))
                .border_1()
                .border_color(islands::ai_border(appearance, cx).opacity(0.72))
                .rounded(islands::radius_sm(appearance))
                .max_h(px(150.0))
                .overflow_hidden()
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(truncated)),
        )
    } else {
        None
    };

    // Blocked reason line
    let blocked_reason = if is_blocked {
        preview.reason.as_ref().map(|reason| {
            div()
                .mt(spacing::xs())
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(reason.clone())
        })
    } else {
        None
    };

    let (border_color, bg) = if is_blocked {
        (cx.theme().danger, cx.theme().danger.opacity(0.08))
    } else if danger {
        (cx.theme().danger, islands::ai_surface_bg(appearance, cx).opacity(0.9))
    } else {
        (
            islands::ai_border(appearance, cx).opacity(0.8),
            islands::ai_surface_bg(appearance, cx).opacity(0.9),
        )
    };

    let summary_color = if is_blocked { cx.theme().danger } else { cx.theme().foreground };

    div()
        .flex()
        .flex_col()
        .p(spacing::sm())
        .border_1()
        .border_color(border_color)
        .rounded(islands::radius_sm(appearance))
        .bg(bg)
        .child(
            div().flex().items_center().gap(spacing::sm()).child(tier_icon).child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(summary_color)
                    .child(summary),
            ),
        )
        .children(blocked_reason)
        .children(sample_preview)
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(spacing::sm())
                .mt(spacing::sm())
                .child(cancel_button)
                .child(confirm_button),
        )
        .into_any_element()
}

fn display_tool_name(name: &str) -> String {
    name.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => {
                    let mut s = c.to_uppercase().to_string();
                    s.push_str(chars.as_str());
                    s
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_pipeline_from_args(
    args_json: &str,
) -> Option<(String, Vec<crate::state::app_state::PipelineStage>)> {
    let args: serde_json::Value = serde_json::from_str(args_json).ok()?;
    let pipeline_str = args.get("pipeline")?.as_str()?;
    let collection = args.get("collection").and_then(|v| v.as_str()).map(String::from);
    let pipeline: Vec<serde_json::Value> = serde_json::from_str(pipeline_str).ok()?;

    let stages: Vec<crate::state::app_state::PipelineStage> = pipeline
        .iter()
        .filter_map(|stage| {
            let obj = stage.as_object()?;
            let (op, body_val) = obj.iter().next()?;
            let body = serde_json::to_string_pretty(body_val).ok()?;
            Some(crate::state::app_state::PipelineStage {
                operator: op.clone(),
                body,
                enabled: true,
            })
        })
        .collect();

    if stages.is_empty() {
        return None;
    }
    Some((collection.unwrap_or_default(), stages))
}

fn build_forge_aggregate_command(args_json: &str) -> Option<String> {
    let args: serde_json::Value = serde_json::from_str(args_json).ok()?;
    let collection = args.get("collection")?.as_str()?;
    let pipeline_str = args.get("pipeline")?.as_str()?;
    let pipeline: Vec<serde_json::Value> = serde_json::from_str(pipeline_str).ok()?;
    if pipeline.is_empty() {
        return None;
    }
    let pipeline_array = serde_json::Value::Array(pipeline);
    let formatted = crate::bson::format_relaxed_json_value(&pipeline_array);
    let escaped = collection.replace('"', "\\\"");
    Some(format!("db.getCollection(\"{escaped}\").aggregate({formatted})"))
}

fn render_report_download_buttons(
    reports: &[(String, Vec<crate::ai::ReportSheet>)],
    state: &Entity<AppState>,
    turn_id: &Uuid,
    cx: &App,
) -> AnyElement {
    let border = cx.theme().border.opacity(0.5);
    let mut row = div().flex().flex_wrap().gap(spacing::sm()).pt(spacing::sm());

    for (i, (title, sheets)) in reports.iter().enumerate() {
        let st = state.clone();
        let title_dl = title.clone();
        let sheets_dl = sheets.clone();
        let label = if reports.len() == 1 {
            "Download Excel".to_string()
        } else {
            format!("Download: {}", title)
        };
        row = row.child(
            Button::new(ElementId::Name(format!("chat-dl-rpt-{turn_id}-{i}").into()))
                .primary()
                .compact()
                .icon(Icon::new(IconName::Download).xsmall())
                .label(label)
                .on_click(move |_, _, cx| {
                    download_report_as_excel(st.clone(), title_dl.clone(), sheets_dl.clone(), cx);
                }),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .pt(spacing::xs())
        .border_t_1()
        .border_color(border)
        .child(row)
        .into_any_element()
}

fn download_report_as_excel(
    state: Entity<AppState>,
    title: String,
    sheets: Vec<crate::ai::ReportSheet>,
    cx: &mut App,
) {
    let (client, database, manager) = {
        let st = state.read(cx);
        let triple = st.selected_connection_id().and_then(|id| {
            let client = st.active_connection_client(id)?;
            let db = st.selected_database_name()?;
            Some((client, db, st.connection_manager()))
        });
        match triple {
            Some(t) => t,
            None => {
                state.update(cx, |s, cx| {
                    s.set_status_message(Some(crate::state::StatusMessage::error(
                        "No active connection or database",
                    )));
                    cx.notify();
                });
                return;
            }
        }
    };

    let now = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let safe_title: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let default_name = format!("{}_{}.xlsx", safe_title, now);

    let filters = vec![crate::components::file_picker::FileFilter::excel()];

    cx.spawn({
        let state = state.clone();
        async move |cx: &mut gpui::AsyncApp| {
            let path = crate::components::file_picker::open_file_dialog_async(
                crate::components::file_picker::FilePickerMode::Save,
                filters,
                Some(default_name),
            )
            .await;

            let Some(path) = path else {
                return;
            };

            let _ = cx.update(|cx| {
                state.update(cx, |s, cx| {
                    s.set_status_message(Some(crate::state::StatusMessage::info(
                        "Exporting report...",
                    )));
                    cx.notify();
                });

                let task = cx.background_spawn({
                    let client = client.clone();
                    let database = database.clone();
                    let sheets = sheets.clone();
                    let path = path.clone();
                    let manager = manager.clone();
                    async move {
                        manager.export_report_to_excel(&client, &database, &sheets, &path, |_| {})
                    }
                });

                cx.spawn({
                    let state = state.clone();
                    async move |cx: &mut gpui::AsyncApp| {
                        let result = task.await;
                        let _ = cx.update(|cx| {
                            state.update(cx, |s, cx| {
                                match result {
                                    Ok(r) => {
                                        let mut msg = format!(
                                            "Report exported: {} rows across {} sheets",
                                            r.total_rows, r.sheets_written
                                        );
                                        if !r.errors.is_empty() {
                                            msg.push_str(&format!(
                                                " ({} sheet(s) failed)",
                                                r.errors.len()
                                            ));
                                        }
                                        s.set_status_message(Some(
                                            crate::state::StatusMessage::info(msg),
                                        ));
                                    }
                                    Err(e) => {
                                        s.set_status_message(Some(
                                            crate::state::StatusMessage::error(format!(
                                                "Report export failed: {}",
                                                e
                                            )),
                                        ));
                                    }
                                }
                                cx.notify();
                            });
                        });
                    }
                })
                .detach();
            });
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn coalesce_stream_events_merges_adjacent_text_chunks() {
        let events = vec![
            StreamEvent::TextDelta("hel".to_string()),
            StreamEvent::TextDelta("lo".to_string()),
            StreamEvent::TextDelta("".to_string()),
            StreamEvent::TextDelta(" world".to_string()),
        ];

        let merged = coalesce_stream_events(events);
        assert_eq!(merged.len(), 1);
        match &merged[0] {
            StreamEvent::TextDelta(text) => assert_eq!(text, "hello world"),
            _ => panic!("expected text delta"),
        }
    }

    #[::core::prelude::v1::test]
    fn coalesce_stream_events_keeps_non_text_boundaries() {
        let events = vec![
            StreamEvent::TextDelta("a".to_string()),
            StreamEvent::ToolCallStart {
                name: "find_documents".to_string(),
                args_preview: "{}".to_string(),
                args_full: "{}".to_string(),
            },
            StreamEvent::TextDelta("b".to_string()),
            StreamEvent::TextDelta("c".to_string()),
        ];

        let merged = coalesce_stream_events(events);
        assert_eq!(merged.len(), 3);
        match &merged[0] {
            StreamEvent::TextDelta(text) => assert_eq!(text, "a"),
            _ => panic!("expected first item to be text delta"),
        }
        match &merged[1] {
            StreamEvent::ToolCallStart { name, .. } => assert_eq!(name, "find_documents"),
            _ => panic!("expected second item to be tool call"),
        }
        match &merged[2] {
            StreamEvent::TextDelta(text) => assert_eq!(text, "bc"),
            _ => panic!("expected third item to be text delta"),
        }
    }
}
