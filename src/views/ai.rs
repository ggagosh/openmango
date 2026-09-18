use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::bubble::{Bubble, BubbleVariant};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{
    Editor, EditorState, InputEvent, TextDecoration, TextDecorationCollection,
};
use gpui_kit::component::message::{
    Message, MessageAlignment, MessageContent, MessageFooter, MessageHeader,
};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::shimmer::ShimmerText;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::*;

use uuid::Uuid;

use crate::ai::bridge::AiBridge;
use crate::ai::context::build_ai_context;
use crate::ai::model_registry;
use crate::ai::provider::{AiGenerationRequest, generate_text_streaming};
use crate::ai::safety::SafetyTier;
use crate::ai::telemetry::AiRequestSpan;
use crate::ai::tools::{MongoContext, StreamEvent};
use crate::ai::{
    AiChatEntry, AiTurn, ChatMessage, ChatMessageTone, ChatRole, ContentBlock, ToolActivity,
    ToolActivityStatus, TurnUsage,
};
use crate::components::Button;
use crate::state::{AiProvider, AppCommands, AppState};
use crate::theme::{borders, islands, spacing};
use gpui_kit::component::{Icon, IconName, Size};

pub struct AiView {
    state: Entity<AppState>,
    input_state: Option<Entity<EditorState>>,
    mention_decorations: Option<TextDecorationCollection>,
    input_subscription: Option<Subscription>,
    /// The scroller owns scrolling, virtualization and following the tail.
    scroller: Option<Entity<MessageScrollerState>>,
    /// The rows it renders, rebuilt only when the conversation actually changed.
    timeline: Rc<Vec<TimelineRow>>,
    timeline_revision: u64,
    /// User's manual expand/collapse overrides for tool groups.
    /// Key = id of the first ToolActivity in the group.
    /// Absent = auto (expanded while running, collapsed when done).
    tool_group_overrides: HashMap<Uuid, bool>,
    last_seen_provider: AiProvider,
    /// The model picker keeps its own search state, so the view holds it across frames.
    model_selector: Option<crate::components::model_select::ModelSelector>,
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
            mention_decorations: None,
            input_subscription: None,
            scroller: None,
            timeline: Rc::new(Vec::new()),
            timeline_revision: 0,
            tool_group_overrides: HashMap::new(),
            last_seen_provider,
            model_selector: None,
            _subscriptions: subscriptions,
            mention_query: None,
            mention_filtered: Vec::new(),
            mention_selected_index: 0,
        }
    }

    fn ensure_scroller(
        &mut self,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<MessageScrollerState> {
        if let Some(scroller) = &self.scroller {
            return scroller.clone();
        }
        let scroller = cx.new(|cx| MessageScrollerState::new(count, cx));
        cx.observe(&scroller, |_, _, cx| cx.notify()).detach();
        self.scroller = Some(scroller.clone());
        let _ = window;
        scroller
    }

    /// Sending a message means the user wants to watch the answer arrive.
    fn follow_tail(&mut self, cx: &mut Context<Self>) {
        if let Some(scroller) = &self.scroller {
            scroller.update(cx, |scroller, cx| scroller.scroll_to_end(cx));
        }
    }

    /// Rebuild the rows when the conversation changed, and tell the scroller what moved:
    /// new rows to follow, or the last row growing as tokens arrive.
    fn sync_timeline(
        &mut self,
        entries: &[AiChatEntry],
        turn_working: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Expanding a group changes no entry, so the overrides belong in the key too — without
        // them a click rebuilt nothing and the group looked dead.
        let mut revision =
            timeline_revision(entries).wrapping_mul(131).wrapping_add(u64::from(turn_working));
        for (key, expanded) in &self.tool_group_overrides {
            revision = revision.wrapping_add(key.as_u128() as u64).wrapping_mul(if *expanded {
                3
            } else {
                5
            });
        }
        if revision == self.timeline_revision && !self.timeline.is_empty() {
            return;
        }
        self.timeline_revision = revision;
        let previous = self.timeline.len();
        let rows = build_timeline(entries, &self.tool_group_overrides, turn_working);
        let count = rows.len();
        self.timeline = Rc::new(rows);

        let Some(scroller) = self.scroller.clone() else { return };
        scroller.update(cx, |scroller, cx| match count.cmp(&previous) {
            std::cmp::Ordering::Greater => {
                let _ = scroller.append(count - previous, cx);
            }
            // Same rows, changed content: the streaming row grew.
            std::cmp::Ordering::Equal => {
                let _ = scroller.remeasure_items(previous.saturating_sub(1)..count, cx);
            }
            std::cmp::Ordering::Less => scroller.reset(count, cx),
        });
        let _ = window;
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
    fn update_mention_highlights(&self, text: &str, cx: &mut Context<Self>) {
        let Some(decorations) = &self.mention_decorations else {
            return;
        };
        let mentioned = self.state.read(cx).ai_chat.mentioned_collections.clone();
        if mentioned.is_empty() {
            decorations.clear(cx);
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
                    highlights.push(TextDecoration::new(abs..end, style));
                }
                start = abs + 1;
            }
        }
        highlights.sort_by_key(|decoration| decoration.range.start);
        decorations.set(highlights, cx);
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

    /// Send a question queued with `AppState::ask_ai`, or leave it in the input when it can't be
    /// sent yet (no collection selected, or a reply still streaming).
    fn send_pending_prompt(
        &mut self,
        input: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.state.update(cx, |state, _| state.ai_chat.pending_prompt.take())
        else {
            return;
        };
        let can_submit = {
            let state = self.state.read(cx);
            state.settings.ai.enabled
                && !state.ai_chat.is_loading
                && state.current_ai_session_key().is_some()
        };
        if can_submit {
            self.send_message_with_mentions(prompt, Vec::new(), cx);
        } else {
            self.state.update(cx, |state, _| state.ai_chat.draft_input = prompt.clone());
            input.update(cx, |input, cx| {
                input.set_value(prompt, window, cx);
                input.focus(window, cx);
            });
        }
    }

    pub fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.ensure_input_state(window, cx);
        input.update(cx, |state, cx| state.focus(window, cx));
    }

    fn ensure_input_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<EditorState> {
        if self.input_state.is_none() {
            let input_state = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("text")
                    .auto_close(false)
                    .smart_indent(false)
                    .soft_wrap(true)
                    .line_number(false)
                    // Without this the fold gutter reserves space and the caret starts a long
                    // way in from the border.
                    .folding(false)
                    .scroll_beyond_last_line(Some(0))
                    .cursor_surrounding_lines(Some(0))
                    .submit_on_enter(true)
                    .clean_on_escape()
                    .placeholder("Ask about your data…")
            });

            self.mention_decorations = Some(
                input_state
                    .update(cx, |input, cx| input.create_decorations_collection(Vec::new(), cx)),
            );
            let state = self.state.clone();
            let sub =
                cx.subscribe_in(&input_state, window, move |view, entity, event, window, cx| {
                    match event {
                        InputEvent::Change => {
                            let text = entity.read(cx).value().to_string();
                            state.update(cx, |state, _| {
                                state.ai_chat.draft_input = text.clone();
                            });
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
                                            gpui_kit::component::input::Position::new(
                                                line, character,
                                            ),
                                            window,
                                            cx,
                                        );
                                    });
                                    view.confirm_mention(cx);
                                    view.update_mention_highlights(&new_text, cx);
                                    return;
                                }
                            }

                            view.detect_mention_trigger(&text, cursor, cx);
                            view.sync_mentions_with_text(&text, cx);
                            view.update_mention_highlights(&text, cx);
                        }
                        InputEvent::Blur => {
                            let raw = entity.read(cx).value().to_string();
                            state.update(cx, |s, _| {
                                s.ai_chat.draft_input = raw;
                            });
                        }
                        InputEvent::PressEnter { secondary: false, shift: false } => {
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
        self.follow_tail(cx);

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
        // The prompt must describe the same tools `build_agent` hands over.
        let writable = {
            let state = self.state.read(cx);
            state.selected_connection_id().is_some_and(|id| !state.connection_read_only(id))
        };
        let system_prompt = build_ai_context(self.state.read(cx), &mentioned, writable);
        log::debug!(
            "[ai-chat] system_prompt len={} history_msgs={}",
            system_prompt.len(),
            history.len()
        );

        let tool_ctx = {
            let s = self.state.read(cx);
            s.selected_connection_id().and_then(|id| {
                let client = s.active_connection_client(id)?;
                let db = s.selected_database_name()?;
                let col = s.selected_collection_name();
                let write_identity =
                    crate::models::ConnectionWriteIdentity::from(s.connection_by_id(id)?);
                Some(MongoContext {
                    client,
                    database: db,
                    collection: col,
                    write_identity,
                    read_only: s.connection_read_only(id),
                    event_tx: None,
                })
            })
        };

        // What rig sent and received last turn, so tool results survive into this one, and how
        // much of it the chosen model can actually hold.
        let (transcript, context_tokens) = {
            let state = self.state.read(cx);
            let settings = &state.settings.ai;
            let context = state
                .ai_chat
                .catalog()
                .model(settings.provider, &settings.model)
                .and_then(|model| model.limit.context)
                .map(|context| context as usize)
                // Ollama serves whatever is installed and the catalogue does not list it, so
                // assume the small end rather than overrun a local model's window.
                .or((settings.provider == AiProvider::Ollama).then_some(32_000));
            (state.ai_chat.transcript.clone(), context)
        };
        let request = AiGenerationRequest {
            system_prompt,
            history,
            user_prompt: prompt,
            transcript,
            context_tokens,
        };

        let provider_label = ai_settings.provider.label().to_string();
        let model_label = ai_settings.model.clone();
        let session_label = turn_id.to_string();

        // Channel for streaming deltas
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();

        // The same flag the hook reads, so Stop ends the run inside rig's loop.
        let cancel_for_run = cancel_flag.clone();
        let task = cx.background_spawn(async move {
            AiBridge::block_on(async move {
                generate_text_streaming(&ai_settings, request, tool_ctx, cancel_for_run, tx).await
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
                cx.update(|cx| {
                    let changed = state.update(cx, |s, cx| {
                        let changed = merged
                            .into_iter()
                            .filter_map(|event| handle_stream_event(s, message_id, event))
                            .collect::<Vec<_>>();
                        cx.notify();
                        changed
                    });
                    for (session_key, indexes_changed) in changed {
                        if indexes_changed {
                            AppCommands::load_collection_indexes(
                                state.clone(),
                                session_key.clone(),
                                true,
                                cx,
                            );
                        } else {
                            AppCommands::load_documents_for_session(
                                state.clone(),
                                session_key.clone(),
                                cx,
                            );
                        }
                        AppCommands::collection_history_changed(state.clone(), session_key, cx);
                    }
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
                        cx.background_executor().timer(std::time::Duration::from_millis(16)).await;
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
                cx.update(|cx| {
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

            cx.update(|cx| {
                state.update(cx, |s, cx| {
                    match result {
                        Ok(outcome) => {
                            s.ai_chat.transcript = outcome.transcript;
                            s.ai_chat.set_turn_usage(turn_id, outcome.usage);
                            s.ai_chat.finalize_turn_response(message_id, outcome.text.clone());
                            span.finish_ok(outcome.text.len());
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
        let model_select = {
            let state = self.state.clone();
            let selector = self.model_selector.get_or_insert_with(|| {
                crate::components::model_select::ModelSelector::new(&state, window, cx)
            });
            selector.sync(&state, window, cx);
            selector.entity()
        };
        let input_state = self.ensure_input_state(window, cx);
        self.send_pending_prompt(&input_state, window, cx);

        let state = self.state.clone();
        let app_state = self.state.read(cx);
        let appearance = app_state.settings.appearance.clone();
        let ai_chat = &app_state.ai_chat;
        let ai_enabled = app_state.settings.ai.enabled;
        let is_loading = ai_chat.is_loading;
        let session_key = app_state.current_ai_session_key();
        let streaming_turn_id = ai_chat.current_turn_id;
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
                        .xsmall()
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
                        .xsmall()
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

        let entries = ai_chat.entries.clone();
        let scroller = self.ensure_scroller(entries.len(), window, cx);
        self.sync_timeline(&entries, is_loading, window, cx);

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

        // The kit's scroller owns virtualization and stick-to-bottom, including its own
        // jump-to-latest button, so none of that is re-implemented here.
        let message_list = div()
            .size_full()
            .overflow_hidden()
            .relative()
            .bg(islands::ai_shell_bg(&appearance, cx).opacity(0.72))
            .child(if entries.is_empty() {
                div().size_full().child(empty_state).into_any_element()
            } else {
                let rows = self.timeline.clone();
                let row_ctx = RowContext {
                    view: view_entity.clone(),
                    state: state.clone(),
                    appearance: appearance.clone(),
                    streaming_turn_id,
                };
                MessageScroller::new("ai-chat", scroller.clone(), move |index, window, cx| {
                    match rows.get(index) {
                        Some(row) => render_timeline_row(row, &row_ctx, window, cx),
                        None => div().into_any_element(),
                    }
                })
                .with_list_style(StyleRefinement::default().p(spacing::md()).gap(spacing::md()))
                .with_bottom_fade(islands::ai_shell_bg(&appearance, cx))
                .with_jump_button_label("Jump to latest")
                .size_full()
                .into_any_element()
            });

        // Model selector — presets first, then every model, searchable
        let model_selector =
            crate::components::model_select::model_select(&model_select, Size::Small, px(168.0))
                .disabled(is_loading);

        // Send/Stop icon button
        let send_or_stop_button = if is_loading {
            let stop_view = cx.entity();
            Button::new("send-stop")
                .danger()
                .xsmall()
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
                .xsmall()
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
                        .rounded(crate::theme::borders::radius_sm())
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
                        gpui_kit::transparent_black()
                    };
                    div()
                        .id(ElementId::Name(format!("mention-item-{i}").into()))
                        .flex()
                        .items_center()
                        .gap(spacing::sm())
                        .px(spacing::sm())
                        .py(spacing::xs())
                        .cursor_pointer()
                        .rounded(crate::theme::borders::radius_sm())
                        .bg(bg)
                        .hover(|s: gpui_kit::StyleRefinement| {
                            s.bg(cx.theme().secondary.opacity(0.2))
                        })
                        .child(
                            Icon::new(crate::assets::AppIcon::Braces)
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

        // Editor area panel
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

        // The box grows with what is typed instead of opening as a block of dead space.
        let composer_rows = input_state.read(cx).value().lines().count().max(1).clamp(2, 8);
        input_area = input_area
            .child(div().px(px(2.0)).py(px(2.0)).child(
                Editor::new(&input_state).text_xs().appearance(false).w_full().h(window.rem_size()
                    * 0.75
                    * 1.55
                    * composer_rows as f32
                    + px(4.0)),
            ))
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
        .rounded(crate::theme::borders::radius_sm())
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

/// One row of the chat as the scroller sees it: owned, because the scroller renders rows by
/// index on demand rather than from a borrow of the entry list.
pub(crate) enum TimelineRow {
    Turn { turn: AiTurn, tools: Vec<ToolActivity>, expanded: bool },
    ToolGroup { tools: Vec<ToolActivity>, expanded: bool },
    Other(AiChatEntry),
}

/// What every row needs besides its own data.
struct RowContext {
    view: Entity<AiView>,
    state: Entity<AppState>,
    appearance: crate::state::AppearanceSettings,
    streaming_turn_id: Option<Uuid>,
}

/// Whether a tool group shows its detail.
///
/// The user's own toggle always wins — reopening a group they just closed is what made it feel
/// unclickable. Left alone, a group stays open for as long as the turn is working, rather than
/// opening and closing as each tool starts and finishes.
fn group_expanded(
    tools: &[ToolActivity],
    overrides: &HashMap<Uuid, bool>,
    turn_working: bool,
) -> bool {
    if let Some(&expanded) = overrides.get(&tools[0].id) {
        return expanded;
    }
    turn_working
        || tools.iter().any(|tool| {
            matches!(
                tool.status,
                ToolActivityStatus::Running | ToolActivityStatus::AwaitingConfirmation { .. }
            )
        })
}

/// Group the flat entry list into rows. Tool activity folds into the turn it belongs to.
fn build_timeline(
    entries: &[AiChatEntry],
    overrides: &HashMap<Uuid, bool>,
    turn_working: bool,
) -> Vec<TimelineRow> {
    let mut rows: Vec<TimelineRow> = Vec::new();
    let mut pending: Vec<ToolActivity> = Vec::new();

    let flush = |pending: &mut Vec<ToolActivity>, rows: &mut Vec<TimelineRow>| {
        if pending.is_empty() {
            return;
        }
        let tools = std::mem::take(pending);
        match rows.last_mut() {
            Some(TimelineRow::Turn { tools: existing, .. }) => existing.extend(tools),
            _ => rows.push(TimelineRow::ToolGroup { tools, expanded: false }),
        }
    };

    for entry in entries {
        if let AiChatEntry::ToolActivity(activity) = entry {
            pending.push(activity.clone());
            continue;
        }
        flush(&mut pending, &mut rows);
        match entry {
            AiChatEntry::Turn(turn) => {
                rows.push(TimelineRow::Turn {
                    turn: turn.clone(),
                    tools: Vec::new(),
                    expanded: false,
                });
            }
            _ => rows.push(TimelineRow::Other(entry.clone())),
        }
    }
    flush(&mut pending, &mut rows);

    for row in &mut rows {
        match row {
            TimelineRow::Turn { tools, expanded, .. } if !tools.is_empty() => {
                *expanded = group_expanded(tools, overrides, turn_working);
            }
            TimelineRow::ToolGroup { tools, expanded } => {
                *expanded = group_expanded(tools, overrides, turn_working);
            }
            _ => {}
        }
    }
    rows
}

fn render_timeline_row(
    row: &TimelineRow,
    ctx: &RowContext,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match row {
        TimelineRow::Turn { turn, tools, expanded } => {
            let tool_refs: Vec<&ToolActivity> = tools.iter().collect();
            let tool_section = (!tools.is_empty()).then(|| {
                render_tool_group(
                    &tool_refs,
                    *expanded,
                    tools[0].id,
                    ctx.view.clone(),
                    ctx.state.clone(),
                    &ctx.appearance,
                    window,
                    cx,
                )
            });
            let reports: Vec<(String, Vec<crate::ai::ReportSheet>)> = tools
                .iter()
                .filter_map(|tool| match tool.result_block.as_deref() {
                    Some(ContentBlock::Report { title, sheets }) => {
                        Some((title.clone(), sheets.clone()))
                    }
                    _ => None,
                })
                .collect();
            render_turn(
                turn,
                tool_section,
                TurnReportContext { reports, state: ctx.state.clone() },
                ctx.streaming_turn_id == Some(turn.id),
                &ctx.appearance,
                window,
                cx,
            )
        }
        TimelineRow::ToolGroup { tools, expanded } => {
            let tool_refs: Vec<&ToolActivity> = tools.iter().collect();
            render_tool_group(
                &tool_refs,
                *expanded,
                tools[0].id,
                ctx.view.clone(),
                ctx.state.clone(),
                &ctx.appearance,
                window,
                cx,
            )
        }
        TimelineRow::Other(entry) => match entry {
            AiChatEntry::SystemMessage(msg) => render_status_message(msg, &ctx.appearance, cx),
            AiChatEntry::LegacyMessage(msg) => {
                let color = match msg.role {
                    ChatRole::User => cx.theme().foreground,
                    ChatRole::Assistant => cx.theme().primary,
                    ChatRole::System => cx.theme().muted_foreground,
                };
                div()
                    .px(spacing::md())
                    .py(spacing::sm())
                    .bg(islands::ai_surface_muted_bg(&ctx.appearance, cx))
                    .rounded(islands::radius_sm(&ctx.appearance))
                    .border_1()
                    .border_color(islands::panel_border(&ctx.appearance, cx))
                    .text_sm()
                    .text_color(color)
                    .child(format!("{}: {}", msg.role.label(), msg.content))
                    .into_any_element()
            }
            _ => div().into_any_element(),
        },
    }
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
    // An answer is mostly prose with the occasional query in it, so the code block has room to
    // breathe and the table reads as data rather than as more paragraphs.
    let code_block_style = gpui_kit::StyleRefinement::default()
        .mt(spacing::sm())
        .mb(spacing::sm())
        .p(spacing::sm())
        .rounded(borders::radius_sm())
        .bg(cx.theme().secondary.opacity(0.35))
        .border_1()
        .border_color(cx.theme().border.opacity(0.82));
    let table_style = gpui_kit::StyleRefinement::default()
        .mt(spacing::sm())
        .mb(spacing::sm())
        .rounded(borders::radius_sm())
        .border_1()
        .border_color(cx.theme().border.opacity(0.82));
    let table_head_style = gpui_kit::StyleRefinement::default()
        .bg(cx.theme().secondary.opacity(0.35))
        .text_color(cx.theme().muted_foreground);
    let table_cell_style = gpui_kit::StyleRefinement::default().px(spacing::sm()).py(spacing::xs());

    TextViewStyle {
        // Wider than the old 0.72: paragraphs that touch read as one block of text.
        paragraph_gap: rems(0.9),
        heading_base_font_size: px(13.0),
        highlight_theme: cx.theme().highlight_theme.clone(),
        is_dark: cx.theme().mode.is_dark(),
        code_block: code_block_style,
        table: table_style,
        table_head: table_head_style,
        table_cell: table_cell_style,
        ..TextViewStyle::default()
    }
    .heading_font_size(|level, base| {
        let scale = match level {
            1 => 1.34,
            2 => 1.2,
            3 => 1.08,
            _ => 1.0,
        };
        base * scale
    })
}

/// The assistant's side of a turn. The kit's Message owns the row and the Bubble the surface;
/// the island colours are kept as overrides so the panel still looks like the rest of the app.
fn assistant_message(
    label: &'static str,
    label_color: Hsla,
    body: impl IntoElement,
    surface: Hsla,
    border: Hsla,
    usage: Option<TurnUsage>,
    appearance: &crate::state::AppearanceSettings,
) -> Message {
    let message = Message::new()
        .alignment(MessageAlignment::Start)
        .with_stack_style(StyleRefinement::default().w_full().min_w(px(0.0)))
        .header(MessageHeader::new().child(
            div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(label_color).child(label),
        ))
        .content(
            MessageContent::new().w_full().bubble(
                Bubble::new()
                    .with_variant(BubbleVariant::Ghost)
                    .px(spacing::md())
                    .py(spacing::sm())
                    .bg(surface)
                    .border_1()
                    .border_color(border)
                    .rounded(islands::radius_sm(appearance))
                    .w_full()
                    .min_w(px(0.0))
                    .child(body),
            ),
        );
    match usage.filter(|usage| !usage.is_empty()) {
        Some(usage) => message.footer(
            MessageFooter::new()
                .child(div().text_xs().text_color(label_color.opacity(0.7)).child(usage.label())),
        ),
        None => message,
    }
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
    let assistant_bg = islands::ai_surface_bg(appearance, cx).opacity(0.88);

    // The kit's Message handles the row, its alignment and the bubble; what the user typed is
    // the only thing this has to supply.
    let user_msg = Message::new()
        .alignment(MessageAlignment::End)
        .with_stack_style(
            StyleRefinement::default().max_w(px(820.0)).min_w(px(0.0)).w_full().ml_auto(),
        )
        .header(
            MessageHeader::new().child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().primary)
                    .child("You"),
            ),
        )
        .content(
            MessageContent::new().bubble(
                Bubble::new().with_variant(BubbleVariant::Tinted).child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .min_w(px(0.0))
                        .child(turn.user_message.content.clone()),
                ),
            ),
        );

    let assistant_section = match &turn.assistant_message {
        Some(msg) if msg.tone == ChatMessageTone::Error => {
            let mut body = div().flex().flex_col().gap(ai_block_gap());
            if let Some(ts) = tool_section {
                body = body.child(ts);
            }
            body = body.child(
                div()
                    .text_sm()
                    .min_w(px(0.0))
                    .child(render_plain_text_lines(&msg.content, cx.theme().foreground)),
            );

            Some(assistant_message(
                "Error",
                cx.theme().danger,
                body,
                cx.theme().danger.opacity(0.1),
                cx.theme().danger.opacity(0.42),
                None,
                appearance,
            ))
        }
        Some(msg) if is_streaming && !msg.content.is_empty() => {
            let mut body = div().flex().flex_col().gap(ai_block_gap());
            if let Some(ts) = tool_section {
                body = body.child(ts);
            }
            body = body
                .child(
                    div().min_w(px(0.0)).text_color(cx.theme().foreground).child(
                        TextView::markdown(
                            ElementId::Name(format!("ai-stream-{}", msg.id).into()),
                            msg.content.clone(),
                        )
                        .selectable(true)
                        .style(ai_markdown_style(cx)),
                    ),
                )
                .child(
                    div().text_xs().child(
                        ShimmerText::new("Streaming…")
                            .id("ai-streaming")
                            .text_color(cx.theme().muted_foreground),
                    ),
                );

            Some(assistant_message(
                "Assistant",
                cx.theme().primary,
                body,
                assistant_bg,
                border,
                turn.usage,
                appearance,
            ))
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

            let mut body = div().flex().flex_col().gap(ai_block_gap());
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

            Some(assistant_message(
                "Assistant",
                cx.theme().primary,
                body,
                assistant_bg,
                border,
                turn.usage,
                appearance,
            ))
        }
        Some(_) if is_streaming => {
            let mut body = div().flex().flex_col().gap(ai_block_gap());
            if let Some(ts) = tool_section {
                body = body.child(ts);
            }
            body = body.child(
                div().text_xs().child(
                    ShimmerText::new("Thinking…")
                        .id("ai-thinking")
                        .text_color(cx.theme().muted_foreground),
                ),
            );

            Some(assistant_message(
                "Assistant",
                cx.theme().primary,
                body,
                assistant_bg,
                border,
                turn.usage,
                appearance,
            ))
        }
        // Tool work with nothing said about it yet: no bubble, just the activity.
        _ => tool_section.map(|ts| {
            Message::new()
                .alignment(MessageAlignment::Start)
                .with_stack_style(StyleRefinement::default().w_full().min_w(px(0.0)))
                .content(
                    MessageContent::new()
                        .w_full()
                        .child(div().px(spacing::md()).py(spacing::sm()).child(ts)),
                )
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
                .min_w(px(0.0))
                .child(header_icon)
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .flex_shrink_0()
                // Which tools ran, at a glance, without opening the group.
                .children(distinct_tool_icons(tools).into_iter().map(|icon| {
                    icon.xsmall().text_color(cx.theme().muted_foreground.opacity(0.75))
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{}", tools.len())),
                ),
        )
        .on_mouse_down(MouseButton::Left, {
            move |_, _, cx| {
                cx.stop_propagation();
                view.update(cx, |this, cx| {
                    this.tool_group_overrides.insert(group_key, !expanded);
                    cx.notify();
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
        if let Some(block) = t.result_block.as_deref() {
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
                            .xsmall()
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
                            .xsmall()
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
                            .xsmall()
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
                    (Spinner::new().xsmall().into_any_element(), "running")
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
                                        .child(format!("{display_name} failed")),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().foreground)
                                .child(crate::error::sentence(reason)),
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
                .child(
                    tool_icon(&activity.tool_name).xsmall().text_color(cx.theme().muted_foreground),
                )
                .child(
                    div().text_xs().text_color(cx.theme().foreground).child(display_name.clone()),
                )
                .children(activity.collection.clone().map(|collection| {
                    div().text_xs().text_color(cx.theme().muted_foreground).child(collection)
                }))
                .child(div().flex_1())
                .child(icon_el)
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(suffix))
                .into_any_element()
        }
    }
}

fn handle_stream_event(
    state: &mut AppState,
    message_id: Uuid,
    event: StreamEvent,
) -> Option<(crate::state::SessionKey, bool)> {
    match event {
        StreamEvent::TextDelta(delta) => {
            state.ai_chat.append_turn_delta(message_id, &delta);
            None
        }
        StreamEvent::ToolCallStart { call_id, name, args_preview, args_full } => {
            state.ai_chat.push_tool_start(call_id, name, args_preview, args_full);
            None
        }
        StreamEvent::ToolCallEnd { call_id, name, result_preview, result_json } => {
            state.ai_chat.complete_tool(&call_id, &name, result_preview, result_json);
            None
        }
        StreamEvent::ToolCallFailed { call_id, name, reason } => {
            state.ai_chat.fail_tool(&call_id, &name, reason);
            None
        }
        StreamEvent::DocumentsChanged { connection_id, database, collection } => {
            Some((crate::state::SessionKey::new(connection_id, database, collection), false))
        }
        StreamEvent::IndexesChanged { connection_id, database, collection } => {
            Some((crate::state::SessionKey::new(connection_id, database, collection), true))
        }
        StreamEvent::ConfirmationRequired {
            tool_name,
            description,
            tier,
            preview,
            write_identity,
            response_tx,
        } => {
            state.ai_chat.set_tool_awaiting_confirmation(
                &tool_name,
                description,
                tier,
                preview,
                write_identity,
                response_tx,
            );
            None
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
        "replace_documents" => "Replace",
        "delete_documents" => "Delete",
        "create_index" => "Create Index",
        "drop_index" => "Drop Index",
        _ => "Approve",
    }
}

fn is_danger_tool(tool_name: &str) -> bool {
    matches!(tool_name, "replace_documents" | "delete_documents" | "drop_index")
}

fn ai_write_identity_is_current(
    state: &AppState,
    write_identity: &crate::models::ConnectionWriteIdentity,
) -> bool {
    state
        .connection_by_id(write_identity.id)
        .is_some_and(|connection| write_identity.matches(connection))
        && !state.connection_read_only(write_identity.id)
}

fn approve_ai_tool_confirmation(
    state: &Entity<AppState>,
    write_identity: &crate::models::ConnectionWriteIdentity,
    response_tx: &crate::ai::safety::ConfirmationSender,
    activity_id: Uuid,
    cx: &mut App,
) {
    let allowed = state.read(cx).is_connected(write_identity.id)
        && ai_write_identity_is_current(state.read(cx), write_identity);
    response_tx.respond(allowed);
    state.update(cx, |state, cx| {
        if allowed {
            state.ai_chat.approve_tool_confirmation(activity_id);
        } else {
            state.ai_chat.reject_tool_confirmation(activity_id);
            state.set_status_message(Some(crate::state::StatusMessage::error(
                "AI write blocked because the connection identity changed. Review it again.",
            )));
        }
        cx.notify();
    });
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
        ref write_identity,
        ref response_tx,
    } = activity.status
    else {
        unreachable!();
    };
    let activity_id = activity.id;
    let identity = crate::components::ConnectionIdentity::from(write_identity);
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
    let approve_identity = write_identity.clone();

    let confirm_id: SharedString = format!("confirm-{activity_id}").into();
    let cancel_id: SharedString = format!("cancel-{activity_id}").into();

    let confirm_button = if danger {
        Button::new(confirm_id).danger().xsmall().label(confirm_label).on_click(move |_, _, cx| {
            approve_ai_tool_confirmation(
                &approve_state,
                &approve_identity,
                &approve_tx,
                activity_id,
                cx,
            );
        })
    } else {
        Button::new(confirm_id).primary().xsmall().label(confirm_label).on_click({
            let approve_identity = approve_identity.clone();
            move |_, _, cx| {
                approve_ai_tool_confirmation(
                    &approve_state,
                    &approve_identity,
                    &approve_tx,
                    activity_id,
                    cx,
                );
            }
        })
    };

    let cancel_button =
        Button::new(cancel_id).ghost().xsmall().label("Cancel").on_click(move |_, _, cx| {
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
        .child(
            div()
                .mt(spacing::xs())
                .child(crate::components::connection_identity_badge(&identity, true, cx)),
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

/// One icon per distinct tool in a group, in the order they ran, capped so a long run does not
/// turn the header into a strip of icons.
fn distinct_tool_icons(tools: &[&ToolActivity]) -> Vec<Icon> {
    let mut seen: Vec<&str> = Vec::new();
    for tool in tools {
        if !seen.contains(&tool.tool_name.as_str()) {
            seen.push(tool.tool_name.as_str());
        }
    }
    seen.into_iter().take(4).map(tool_icon).collect()
}

/// The icon for a tool, so a run reads as a sequence of actions rather than a wall of names.
fn tool_icon(name: &str) -> Icon {
    use crate::assets::AppIcon;
    match name {
        "find_documents" => Icon::new(IconName::Search),
        "aggregate" => Icon::new(AppIcon::Workflow),
        "count_documents" => Icon::new(IconName::Asterisk),
        "list_collections" => Icon::new(AppIcon::Table2),
        "collection_stats" => Icon::new(IconName::ChartPie),
        "collection_schema" => Icon::new(AppIcon::Braces),
        "list_indexes" => Icon::new(IconName::LayoutDashboard),
        "explain_query" => Icon::new(IconName::Cpu),
        "sample_field_values" => Icon::new(AppIcon::Filter),
        "generate_report" => Icon::new(AppIcon::FileSpreadsheet),
        "insert_documents" => Icon::new(IconName::Plus),
        "replace_documents" => Icon::new(IconName::Replace),
        "delete_documents" => Icon::new(AppIcon::Trash),
        "create_index" => Icon::new(IconName::Plus),
        "drop_index" => Icon::new(IconName::Minus),
        _ => Icon::new(IconName::SquareTerminal),
    }
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
                .xsmall()
                .icon(Icon::new(crate::assets::AppIcon::Download).xsmall())
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
        async move |cx: &mut gpui_kit::AsyncApp| {
            let path = crate::components::file_picker::open_file_dialog_async(
                crate::components::file_picker::FilePickerMode::Save,
                filters,
                Some(default_name),
            )
            .await;

            let Some(path) = path else {
                return;
            };

            cx.update(|cx| {
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
                    async move |cx: &mut gpui_kit::AsyncApp| {
                        let result = task.await;
                        cx.update(|cx| {
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

    fn tool(status: ToolActivityStatus) -> AiChatEntry {
        AiChatEntry::ToolActivity(ToolActivity {
            id: Uuid::new_v4(),
            call_id: None,
            tool_name: "find_documents".to_string(),
            status,
            args_preview: String::new(),
            result_preview: None,
            result_block: None,
            collection: None,
            args_full: None,
        })
    }

    fn turn() -> AiChatEntry {
        AiChatEntry::Turn(AiTurn {
            id: Uuid::new_v4(),
            usage: None,
            user_message: ChatMessage::new(ChatRole::User, "how many orders?"),
            assistant_message: None,
            created_at: chrono::Utc::now(),
        })
    }

    #[::core::prelude::v1::test]
    fn tool_activity_folds_into_the_turn_that_caused_it() {
        let entries =
            vec![turn(), tool(ToolActivityStatus::Completed), tool(ToolActivityStatus::Completed)];
        let rows = build_timeline(&entries, &HashMap::new(), false);

        assert_eq!(rows.len(), 1, "one turn, not three rows");
        match &rows[0] {
            TimelineRow::Turn { tools, expanded, .. } => {
                assert_eq!(tools.len(), 2);
                assert!(!expanded, "finished tools stay collapsed");
            }
            _ => panic!("expected a turn row"),
        }
    }

    #[::core::prelude::v1::test]
    fn a_running_group_opens_itself_but_the_user_can_close_it() {
        let entries = vec![turn(), tool(ToolActivityStatus::Running)];
        let key = match &entries[1] {
            AiChatEntry::ToolActivity(activity) => activity.id,
            _ => unreachable!(),
        };

        let rows = build_timeline(&entries, &HashMap::new(), true);
        match &rows[0] {
            TimelineRow::Turn { expanded, .. } => assert!(*expanded, "running work opens itself"),
            _ => panic!("expected a turn row"),
        }

        // Closing it has to stick, even while the turn keeps working.
        let mut overrides = HashMap::new();
        overrides.insert(key, false);
        let rows = build_timeline(&entries, &overrides, true);
        match &rows[0] {
            TimelineRow::Turn { expanded, .. } => assert!(!*expanded, "the user's choice wins"),
            _ => panic!("expected a turn row"),
        }
    }

    /// Tools finish one at a time; the group must not blink shut between them.
    #[::core::prelude::v1::test]
    fn a_group_stays_open_between_two_tool_calls() {
        let entries = vec![turn(), tool(ToolActivityStatus::Completed)];
        let rows = build_timeline(&entries, &HashMap::new(), true);
        match &rows[0] {
            TimelineRow::Turn { expanded, .. } => {
                assert!(*expanded, "the turn is still working, so its tools stay visible")
            }
            _ => panic!("expected a turn row"),
        }
    }

    #[::core::prelude::v1::test]
    fn tools_without_a_turn_stand_on_their_own() {
        let entries = vec![tool(ToolActivityStatus::Completed)];
        let rows = build_timeline(&entries, &HashMap::new(), false);
        assert!(matches!(rows.as_slice(), [TimelineRow::ToolGroup { .. }]));
    }

    #[::core::prelude::v1::test]
    fn ai_confirmation_uses_captured_connection_identity() {
        let mut state = AppState::new();
        state.connections.clear();
        let connection = crate::models::SavedConnection::new(
            "Production".into(),
            "mongodb://localhost/app".into(),
        );
        let identity = crate::models::ConnectionWriteIdentity::from(&connection);
        state.connections.push(connection);
        assert!(ai_write_identity_is_current(&state, &identity));

        state.connections[0].name = "Other".into();
        assert!(!ai_write_identity_is_current(&state, &identity));
    }

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
                call_id: "call-1".to_string(),
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
