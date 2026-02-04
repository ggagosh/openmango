//! Forge - MongoDB Query Shell
//!
//! A database-scoped query shell with a Forge editor for syntax highlighting,
//! autocomplete, and IDE-like experience.

mod mongosh;

use mongosh::{MongoshBridge, MongoshEvent};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use gpui::EntityInputHandler;
use gpui::*;
use gpui_component::input::{
    CompletionProvider, Input, InputEvent, InputState, Rope, RopeExt, TabSize,
};
use gpui_component::resizable::{resizable_panel, v_resizable};
use gpui_component::scroll::ScrollableElement;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{Icon, IconName, Sizable};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    InsertReplaceEdit, Range,
};
use mongodb::bson::{Bson, Document};
use tokio::sync::broadcast;

use crate::bson::DocumentKey;
use crate::components::Button;
use crate::keyboard::{
    CancelForgeRun, ClearForgeOutput, FindInForgeOutput, FocusForgeEditor, FocusForgeOutput,
    RunForgeAll, RunForgeSelectionOrStatement,
};
use crate::state::SessionDocument;
use crate::state::{AppEvent, AppState, View};
use crate::theme::{borders, colors, fonts, spacing};
use crate::views::documents::tree::lazy_row::compute_row_meta;
use crate::views::documents::tree::lazy_tree::{VisibleRow, build_visible_rows};

const MAX_OUTPUT_RUNS: usize = 50;
const MAX_OUTPUT_LINES: usize = 5000;
const SYSTEM_RUN_ID: u64 = 0;

// ============================================================================
// Suggestion Types (for Forge editor integration)
// ============================================================================

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub label: String,
    pub kind: SuggestionKind,
    pub insert_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    Collection,
    Method,
    Operator,
}

impl SuggestionKind {
    fn as_str(self) -> &'static str {
        match self {
            SuggestionKind::Collection => "Collection",
            SuggestionKind::Method => "Method",
            SuggestionKind::Operator => "Operator",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ForgeOutputTab {
    Results,
    Raw,
}

struct ForgeRunOutput {
    id: u64,
    started_at: DateTime<Utc>,
    code_preview: String,
    raw_lines: Vec<String>,
    error: Option<String>,
    last_print_line: Option<String>,
}

struct ResultPage {
    label: String,
    docs: Vec<Document>,
    pinned: bool,
}

fn format_result_tab_label(label: &str, idx: usize) -> String {
    let trimmed = label.trim();
    let base = if trimmed.is_empty() { format!("Result {}", idx + 1) } else { trimmed.to_string() };
    const MAX_LEN: usize = 32;
    if base.chars().count() <= MAX_LEN {
        return base;
    }
    let shortened: String = base.chars().take(MAX_LEN.saturating_sub(3)).collect();
    format!("{shortened}...")
}

fn statement_bounds(text: &str, cursor: usize) -> (usize, usize) {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut start = cursor.min(len);
    let mut end = cursor.min(len);

    // Walk backward to previous statement boundary.
    let mut i = start;
    while i > 0 {
        let b = bytes[i - 1];
        if b == b';' {
            start = i;
            break;
        }
        if b == b'\n' && i >= 2 && bytes[i - 2] == b'\n' {
            start = i;
            break;
        }
        i -= 1;
        start = i;
    }

    // Walk forward to next statement boundary.
    let mut j = end;
    while j < len {
        let b = bytes[j];
        if b == b';' {
            end = j;
            break;
        }
        if b == b'\n' && j + 1 < len && bytes[j + 1] == b'\n' {
            end = j;
            break;
        }
        j += 1;
        end = j;
    }

    (start.min(len), end.min(len))
}

fn filter_visible_rows(
    documents: &[SessionDocument],
    rows: Vec<VisibleRow>,
    query: &str,
) -> Vec<VisibleRow> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return rows;
    }

    let mut keep_ids: HashSet<String> = HashSet::new();
    for row in rows.iter() {
        let meta = compute_row_meta(row, documents);
        let haystack =
            format!("{} {} {}", meta.key_label, meta.value_label, meta.type_label).to_lowercase();
        if !haystack.contains(&needle) {
            continue;
        }

        let doc_key = &documents[row.doc_index].key;
        keep_ids.insert(crate::bson::doc_root_id(doc_key));
        for depth in 1..=row.path.len() {
            keep_ids.insert(crate::bson::path_to_id(doc_key, &row.path[..depth]));
        }
        keep_ids.insert(row.node_id.clone());
    }

    rows.into_iter().filter(|row| keep_ids.contains(&row.node_id)).collect()
}

// ============================================================================
// Forge Runtime + Completion Provider
// ============================================================================

struct ForgeRuntime {
    bridge: Mutex<Option<Arc<MongoshBridge>>>,
}

impl ForgeRuntime {
    fn new() -> Self {
        Self { bridge: Mutex::new(None) }
    }

    fn ensure_bridge(&self) -> Result<Arc<MongoshBridge>, crate::error::Error> {
        if let Ok(guard) = self.bridge.lock()
            && let Some(bridge) = guard.as_ref()
        {
            return Ok(bridge.clone());
        }

        let bridge = MongoshBridge::new()?;
        if let Ok(mut guard) = self.bridge.lock() {
            *guard = Some(bridge.clone());
        }
        Ok(bridge)
    }
}

struct ForgeCompletionProvider {
    state: Entity<AppState>,
    runtime: Arc<ForgeRuntime>,
}

impl ForgeCompletionProvider {
    fn new(state: Entity<AppState>, runtime: Arc<ForgeRuntime>) -> Self {
        Self { state, runtime }
    }
}

impl CompletionProvider for ForgeCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut Context<InputState>,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let (line_prefix, line_start) = line_prefix_for_offset(rope, offset);
        if should_skip_completion(&line_prefix) {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }

        let trimmed = line_prefix.trim_end();
        let context = detect_context(trimmed);
        let (token, token_start_in_line) = completion_token(&line_prefix, context);
        let replace_start = line_start.saturating_add(token_start_in_line);
        let replace_range = completion_range(rope, replace_start, offset);

        if context.is_none() && token.is_empty() && (trimmed.is_empty() || trimmed.ends_with('.')) {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }

        let local = match context {
            Some(ContextKind::Collections) => {
                ForgeView::build_collection_suggestions(self.state.read(cx))
            }
            Some(ContextKind::Methods) => ForgeView::build_method_suggestions(),
            Some(ContextKind::Operators) => ForgeView::build_operator_suggestions(),
            None => Vec::new(),
        };

        let completion_prefix = trimmed.to_string();
        let merged_local =
            merge_suggestions(local.clone(), Vec::new(), context, &completion_prefix, &token);
        let local_items = suggestions_to_completion_items(merged_local, &replace_range);

        let Some((session_id, uri, database)) = active_forge_session_info(self.state.read(cx))
        else {
            return Task::ready(Ok(CompletionResponse::Array(local_items)));
        };

        let runtime = self.runtime.clone();
        let runtime_handle = self.state.read(cx).connection_manager().runtime_handle();
        let token = token.clone();
        let context_for_merge = context;
        let completion_prefix = completion_prefix.clone();
        let request_text = completion_prefix.clone();
        cx.background_spawn(async move {
            let result = runtime_handle
                .spawn_blocking(move || {
                    let bridge = runtime.ensure_bridge()?;
                    bridge.ensure_session(session_id, &uri, &database)?;
                    bridge.complete(session_id, &request_text, Duration::from_millis(500))
                })
                .await;

            let completions = match result {
                Ok(Ok(list)) => list,
                Ok(Err(err)) => {
                    log::warn!("Forge completion error: {}", err);
                    Vec::new()
                }
                Err(err) => {
                    log::warn!("Forge completion join error: {}", err);
                    Vec::new()
                }
            };

            let merged = merge_suggestions(
                local,
                completions,
                context_for_merge,
                &completion_prefix,
                &token,
            );
            let items = suggestions_to_completion_items(merged, &replace_range);
            Ok(CompletionResponse::Array(items))
        })
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        if new_text.is_empty() {
            return false;
        }
        if new_text.chars().all(|c| c.is_whitespace()) {
            return false;
        }
        new_text.chars().any(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '$')
    }
}

// ============================================================================
// ForgeView
// ============================================================================

pub struct ForgeView {
    state: Entity<AppState>,
    editor_state: Option<Entity<InputState>>,
    editor_subscription: Option<Subscription>,
    completion_provider: Option<Rc<ForgeCompletionProvider>>,
    raw_output_state: Option<Entity<InputState>>,
    raw_output_subscription: Option<Subscription>,
    raw_output_text: String,
    raw_output_programmatic: bool,
    results_search_state: Option<Entity<InputState>>,
    results_search_subscription: Option<Subscription>,
    results_search_query: String,
    current_text: String,
    focus_handle: FocusHandle,
    editor_focus_requested: bool,
    runtime: Arc<ForgeRuntime>,
    mongosh_error: Option<String>,
    run_seq: u64,
    is_running: bool,
    output_runs: Vec<ForgeRunOutput>,
    output_tab: ForgeOutputTab,
    active_run_id: Option<u64>,
    output_events_started: bool,
    last_result: Option<String>,
    last_error: Option<String>,
    result_documents: Option<Arc<Vec<SessionDocument>>>,
    result_pages: Vec<ResultPage>,
    result_page_index: usize,
    result_signature: Option<u64>,
    result_expanded_nodes: HashSet<String>,
    result_scroll: UniformListScrollHandle,
    active_tab_id: Option<uuid::Uuid>,
    output_visible: bool,
    _subscriptions: Vec<Subscription>,
}

impl ForgeView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let runtime = Arc::new(ForgeRuntime::new());

        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&state, |this, state, event, cx| {
                if matches!(event, AppEvent::ViewChanged) {
                    let visible = matches!(state.read(cx).current_view, View::Forge);
                    this.editor_focus_requested = visible;
                    cx.notify();
                }
            }),
        ];

        Self {
            state,
            editor_state: None,
            editor_subscription: None,
            completion_provider: None,
            raw_output_state: None,
            raw_output_subscription: None,
            raw_output_text: String::new(),
            raw_output_programmatic: false,
            results_search_state: None,
            results_search_subscription: None,
            results_search_query: String::new(),
            current_text: String::new(),
            focus_handle,
            editor_focus_requested: false,
            runtime,
            mongosh_error: None,
            run_seq: 0,
            is_running: false,
            output_runs: Vec::new(),
            output_tab: ForgeOutputTab::Raw,
            active_run_id: None,
            output_events_started: false,
            last_result: None,
            last_error: None,
            result_documents: None,
            result_pages: Vec::new(),
            result_page_index: 0,
            result_signature: None,
            result_expanded_nodes: HashSet::new(),
            result_scroll: UniformListScrollHandle::new(),
            active_tab_id: None,
            output_visible: true,
            _subscriptions: subscriptions,
        }
    }

    fn ensure_editor_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_state.is_some() {
            return;
        }

        let provider =
            Rc::new(ForgeCompletionProvider::new(self.state.clone(), self.runtime.clone()));

        let editor_state = cx.new(|cx| {
            let mut editor = InputState::new(window, cx)
                .code_editor("javascript")
                .line_number(true)
                .searchable(true)
                .tab_size(TabSize { tab_size: 2, hard_tabs: false })
                .placeholder("// MongoDB Shell");

            editor.lsp.completion_provider = Some(provider.clone());
            editor
        });

        let subscription = cx.subscribe_in(
            &editor_state,
            window,
            move |this, state, event, window, cx| match event {
                InputEvent::Change => {
                    let text = state.read(cx).value().to_string();
                    this.handle_editor_change(&text, cx);
                }
                InputEvent::PressEnter { .. } => {
                    let mut adjusted = false;
                    state.update(cx, |state, cx| {
                        adjusted = auto_indent_between_braces(state, window, cx);
                    });
                    if adjusted {
                        cx.notify();
                    }
                }
                _ => {}
            },
        );

        self.editor_state = Some(editor_state);
        self.editor_subscription = Some(subscription);
        self.completion_provider = Some(provider);
    }

    fn ensure_raw_output_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(state) = self.raw_output_state.as_ref() {
            return state.clone();
        }

        let raw_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("text")
                .line_number(false)
                .searchable(true)
                .placeholder("No output yet.")
        });

        let subscription =
            cx.subscribe_in(&raw_state, window, move |this, state, event, window, cx| {
                if let InputEvent::Change = event {
                    if this.raw_output_programmatic {
                        return;
                    }
                    let current = state.read(cx).value().to_string();
                    if current != this.raw_output_text {
                        this.raw_output_programmatic = true;
                        state.update(cx, |state, cx| {
                            state.set_value(this.raw_output_text.clone(), window, cx);
                        });
                        this.raw_output_programmatic = false;
                    }
                }
            });

        self.raw_output_subscription = Some(subscription);
        self.raw_output_state = Some(raw_state.clone());
        raw_state
    }

    fn ensure_results_search_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(state) = self.results_search_state.as_ref() {
            return state.clone();
        }

        let search_state = cx
            .new(|cx| InputState::new(window, cx).placeholder("Search results").clean_on_escape());

        let subscription =
            cx.subscribe_in(&search_state, window, move |this, state, event, _window, cx| {
                if let InputEvent::Change = event {
                    let value = state.read(cx).value().to_string();
                    if value != this.results_search_query {
                        this.results_search_query = value;
                        cx.notify();
                    }
                }
            });

        self.results_search_subscription = Some(subscription);
        self.results_search_state = Some(search_state.clone());
        search_state
    }

    fn save_current_content(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.active_tab_id else {
            return;
        };
        let Some(editor_state) = &self.editor_state else {
            return;
        };
        let text = editor_state.read(cx).value().to_string();
        self.current_text = text.clone();
        self.state.update(cx, |state, _cx| {
            state.set_forge_tab_content(tab_id, text);
        });
    }

    fn handle_execute_selection_or_statement(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = self.editor_selection_text(window, cx) {
            self.handle_execute_query(&selection, cx);
            return;
        }

        if let Some(statement) = self.editor_statement_at_cursor(cx) {
            self.handle_execute_query(&statement, cx);
        }
    }

    fn editor_selection_text(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<String> {
        let editor_state = self.editor_state.as_ref()?;
        editor_state.update(cx, |state, cx| {
            let selection = state.selected_text_range(true, window, cx)?;
            if selection.range.start == selection.range.end {
                return None;
            }
            let mut adjusted = None;
            let text = state.text_for_range(selection.range.clone(), &mut adjusted, window, cx)?;
            let trimmed = text.trim();
            if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
        })
    }

    fn editor_statement_at_cursor(&self, cx: &mut Context<Self>) -> Option<String> {
        let editor_state = self.editor_state.as_ref()?;
        let text = editor_state.read(cx).text().to_string();
        let cursor = editor_state.read(cx).cursor().min(text.len());
        let (start, end) = statement_bounds(&text, cursor);
        let snippet = text.get(start..end)?.trim();
        if snippet.is_empty() { None } else { Some(snippet.to_string()) }
    }

    fn sync_active_tab_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        force: bool,
    ) {
        let active_id = self.state.read(cx).active_forge_tab_id();
        if !force && active_id == self.active_tab_id {
            return;
        }
        self.save_current_content(cx);

        self.active_tab_id = active_id;
        let Some(active_id) = active_id else {
            return;
        };

        let content = self.state.read(cx).forge_tab_content(active_id).unwrap_or("").to_string();

        self.current_text = content.clone();
        if let Some(editor_state) = &self.editor_state {
            editor_state.update(cx, |editor, cx| {
                editor.set_value(content.clone(), window, cx);
            });
        }
    }

    fn handle_editor_change(&mut self, text: &str, cx: &mut Context<Self>) {
        self.current_text = text.to_string();
        if let Some(tab_id) = self.active_tab_id {
            let content = self.current_text.clone();
            self.state.update(cx, |state, _cx| {
                state.set_forge_tab_content(tab_id, content);
            });
        }
    }

    fn handle_execute_query(&mut self, text: &str, cx: &mut Context<Self>) {
        self.current_text = text.to_string();
        let (session_id, uri, database, runtime_handle) = {
            let state_ref = self.state.read(cx);
            let Some((session_id, uri, database)) = active_forge_session_info(state_ref) else {
                self.last_error = Some("No active Forge session".to_string());
                self.last_result = None;
                self.clear_result_pages(false);
                return;
            };
            (session_id, uri, database, state_ref.connection_manager().runtime_handle())
        };

        let Some(bridge) = self.ensure_mongosh() else {
            self.clear_result_pages(false);
            cx.notify();
            return;
        };

        self.run_seq = self.run_seq.wrapping_add(1);
        let seq = self.run_seq;
        self.is_running = true;
        self.last_error = None;
        self.last_result = None;
        self.sync_output_tab();
        cx.notify();

        let code = text.to_string();
        self.clear_result_pages(true);
        self.begin_run(seq, &code);
        self.ensure_output_listener(cx);
        let bridge = bridge.clone();

        cx.spawn(async move |view: WeakEntity<ForgeView>, cx: &mut AsyncApp| {
            let result = runtime_handle
                .spawn_blocking(move || {
                    bridge.ensure_session(session_id, &uri, &database)?;
                    let mut eval =
                        bridge.evaluate(session_id, &code, Some(seq), Duration::from_secs(60))?;
                    if ForgeView::should_auto_preview(eval.result_type.as_deref(), &code)
                        && let Some(preview_code) = ForgeView::build_preview_code(&code)
                        && let Ok(preview) = bridge.evaluate(
                            session_id,
                            &preview_code,
                            Some(seq),
                            Duration::from_secs(30),
                        )
                    {
                        eval = preview;
                    }
                    Ok::<mongosh::RuntimeEvaluationResult, crate::error::Error>(eval)
                })
                .await;

            let update_result = cx.update(|cx| {
                view.update(cx, |this, cx| {
                    if seq != this.run_seq {
                        return;
                    }

                    this.is_running = false;
                    match result {
                        Ok(Ok(eval)) => {
                            if let Some(docs) = this.result_documents(&eval.printable) {
                                let label = this.run_label(seq).unwrap_or_else(|| {
                                    Self::default_result_label_for_value(&eval.printable)
                                });
                                this.push_result_page(label, docs);
                                this.last_result = None;
                            } else if this.result_pages.is_empty() {
                                this.clear_results();
                                if Self::is_trivial_printable(&eval.printable) {
                                    this.last_result = None;
                                } else {
                                    this.last_result = Some(this.format_result(&eval));
                                }
                            } else {
                                this.last_result = None;
                            }
                            this.last_error = None;
                            this.sync_output_tab();
                            this.append_eval_output(seq, &eval.printable);
                        }
                        Ok(Err(err)) => {
                            this.clear_result_pages(true);
                            this.last_error = Some(err.to_string());
                            this.last_result = None;
                            this.sync_output_tab();
                            this.append_error_output(seq, &err.to_string());
                        }
                        Err(err) => {
                            this.clear_result_pages(true);
                            this.last_error = Some(err.to_string());
                            this.last_result = None;
                            this.sync_output_tab();
                            this.append_error_output(seq, &err.to_string());
                        }
                    }
                    cx.notify();
                })
            });

            if update_result.is_err() {
                log::debug!("ForgeView dropped before query result.");
            }
        })
        .detach();
    }

    fn restart_session(&mut self, cx: &mut Context<Self>) {
        let (session_id, uri, database, runtime_handle) = {
            let state_ref = self.state.read(cx);
            let Some((session_id, uri, database)) = active_forge_session_info(state_ref) else {
                self.last_error = Some("No active Forge session".to_string());
                self.last_result = None;
                self.clear_result_pages(false);
                cx.notify();
                return;
            };
            (session_id, uri, database, state_ref.connection_manager().runtime_handle())
        };

        let Some(bridge) = self.ensure_mongosh() else {
            self.clear_result_pages(false);
            cx.notify();
            return;
        };

        self.is_running = true;
        self.last_error = None;
        self.clear_result_pages(true);
        self.last_result = Some("Restarting shell...".to_string());
        self.sync_output_tab();
        cx.notify();

        let bridge = bridge.clone();

        cx.spawn(async move |view: WeakEntity<ForgeView>, cx: &mut AsyncApp| {
            let result = runtime_handle
                .spawn_blocking(move || {
                    let _ = bridge.dispose_session(session_id);
                    bridge.ensure_session(session_id, &uri, &database)
                })
                .await;

            let update_result = cx.update(|cx| {
                view.update(cx, |this, cx| {
                    this.is_running = false;
                    match result {
                        Ok(Ok(_)) => {
                            this.last_result = Some("Shell restarted.".to_string());
                            this.last_error = None;
                        }
                        Ok(Err(err)) => {
                            this.last_error = Some(err.to_string());
                            this.last_result = None;
                        }
                        Err(err) => {
                            this.last_error = Some(err.to_string());
                            this.last_result = None;
                        }
                    }
                    this.sync_output_tab();
                    cx.notify();
                })
            });

            if update_result.is_err() {
                log::debug!("ForgeView dropped before restart completed.");
            }
        })
        .detach();
    }

    fn cancel_running(&mut self, cx: &mut Context<Self>) {
        if !self.is_running {
            return;
        }

        let (session_id, uri, database, runtime_handle) = {
            let state_ref = self.state.read(cx);
            let Some((session_id, uri, database)) = active_forge_session_info(state_ref) else {
                return;
            };
            (session_id, uri, database, state_ref.connection_manager().runtime_handle())
        };

        let Some(bridge) = self.ensure_mongosh() else {
            return;
        };

        self.is_running = false;
        self.run_seq = self.run_seq.wrapping_add(1);
        let run_id = self.active_run_id.unwrap_or_else(|| self.ensure_system_run());
        self.append_error_output(run_id, "Cancelled");
        self.last_error = Some("Cancelled".to_string());
        self.last_result = None;
        self.sync_output_tab();
        cx.notify();

        let bridge = bridge.clone();
        cx.spawn(async move |view: WeakEntity<ForgeView>, cx: &mut AsyncApp| {
            let _ = runtime_handle
                .spawn_blocking(move || {
                    let _ = bridge.dispose_session(session_id);
                    bridge.ensure_session(session_id, &uri, &database)
                })
                .await;

            let _ = cx.update(|cx| {
                view.update(cx, |this, cx| {
                    this.is_running = false;
                    cx.notify();
                })
            });
        })
        .detach();
    }

    fn ensure_mongosh(&mut self) -> Option<Arc<MongoshBridge>> {
        match self.runtime.ensure_bridge() {
            Ok(bridge) => {
                self.mongosh_error = None;
                Some(bridge)
            }
            Err(err) => {
                let message = err.to_string();
                log::error!("Failed to start Forge sidecar: {}", message);
                self.mongosh_error = Some(message.clone());
                self.last_error = Some(message);
                None
            }
        }
    }

    fn ensure_output_listener(&mut self, cx: &mut Context<Self>) {
        if self.output_events_started {
            return;
        }

        let bridge = match self.runtime.ensure_bridge() {
            Ok(bridge) => bridge,
            Err(_) => return,
        };

        self.output_events_started = true;
        let mut rx = bridge.subscribe_events();
        cx.spawn(async move |view: WeakEntity<ForgeView>, cx: &mut AsyncApp| {
            loop {
                let event = match rx.recv().await {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                };

                let update_result = cx.update(|cx| {
                    view.update(cx, |this, cx| {
                        this.handle_mongosh_event(event, cx);
                    })
                });

                if update_result.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_mongosh_event(&mut self, event: MongoshEvent, cx: &mut Context<Self>) {
        let Some((session_id, _, _)) = active_forge_session_info(self.state.read(cx)) else {
            return;
        };
        let event_session_id = match &event {
            MongoshEvent::Print { session_id, .. } => session_id,
            MongoshEvent::Clear { session_id } => session_id,
        };
        if *event_session_id != session_id.to_string() {
            return;
        }

        match event {
            MongoshEvent::Print { run_id, lines, payload, .. } => {
                let resolved_run_id =
                    run_id.or(self.active_run_id).unwrap_or_else(|| self.ensure_system_run());
                let last_print_line = lines.iter().rev().find_map(|line| {
                    let trimmed = line.trim();
                    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
                });
                if let Some(values) = &payload {
                    let normalized = Self::format_payload_lines(values);
                    if !normalized.is_empty() {
                        self.append_output_lines(resolved_run_id, normalized);
                    } else {
                        self.append_output_lines(resolved_run_id, lines);
                    }
                } else {
                    self.append_output_lines(resolved_run_id, lines);
                }

                if let Some(values) = payload {
                    if let Some(active_run) = self.active_run_id
                        && resolved_run_id == active_run
                    {
                        let label =
                            self.take_run_print_label(resolved_run_id).unwrap_or_else(|| {
                                values
                                    .first()
                                    .map(Self::default_result_label_for_value)
                                    .unwrap_or_else(|| self.default_result_label())
                            });
                        let total = values.len();
                        for (idx, value) in values.into_iter().enumerate() {
                            if let Some(docs) = self.result_documents(&value) {
                                let tab_label = if total > 1 {
                                    format!("{} ({}/{})", label, idx + 1, total)
                                } else {
                                    label.clone()
                                };
                                self.push_result_page(tab_label, docs);
                            }
                        }
                        self.sync_output_tab();
                    }
                } else if let Some(label) = last_print_line {
                    self.update_run_print_label(resolved_run_id, label);
                }
            }
            MongoshEvent::Clear { .. } => {
                self.clear_output_runs();
            }
        }

        cx.notify();
    }

    fn begin_run(&mut self, run_id: u64, code: &str) {
        let preview = Self::code_preview(code);
        self.output_runs.push(ForgeRunOutput {
            id: run_id,
            started_at: Utc::now(),
            code_preview: preview,
            raw_lines: Vec::new(),
            error: None,
            last_print_line: None,
        });
        self.active_run_id = Some(run_id);
        self.trim_output_runs();
    }

    fn code_preview(code: &str) -> String {
        for line in code.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                if trimmed.chars().count() > 80 {
                    let shortened: String = trimmed.chars().take(77).collect();
                    return format!("{shortened}...");
                }
                return trimmed.to_string();
            }
        }
        "Shell output".to_string()
    }

    fn ensure_system_run(&mut self) -> u64 {
        if !self.output_runs.iter().any(|run| run.id == SYSTEM_RUN_ID) {
            self.output_runs.push(ForgeRunOutput {
                id: SYSTEM_RUN_ID,
                started_at: Utc::now(),
                code_preview: "Shell output".to_string(),
                raw_lines: Vec::new(),
                error: None,
                last_print_line: None,
            });
            self.trim_output_runs();
        }
        SYSTEM_RUN_ID
    }

    fn append_output_lines(&mut self, run_id: u64, lines: Vec<String>) {
        let mut normalized: Vec<String> = Vec::new();
        for line in lines {
            for part in line.split('\n') {
                normalized.push(part.to_string());
            }
        }

        if let Some(run) = self.output_runs.iter_mut().find(|run| run.id == run_id) {
            run.raw_lines.extend(normalized);
        } else {
            self.output_runs.push(ForgeRunOutput {
                id: run_id,
                started_at: Utc::now(),
                code_preview: "Shell output".to_string(),
                raw_lines: normalized,
                error: None,
                last_print_line: None,
            });
            self.trim_output_runs();
        }

        self.trim_output_lines();
    }

    fn append_eval_output(&mut self, run_id: u64, printable: &serde_json::Value) {
        let lines = Self::format_printable_lines(printable);
        if lines.is_empty() {
            return;
        }
        self.append_output_lines(run_id, lines);
    }

    fn append_error_output(&mut self, run_id: u64, message: &str) {
        if let Some(run) = self.output_runs.iter_mut().find(|run| run.id == run_id) {
            run.error = Some(message.to_string());
            return;
        }

        self.output_runs.push(ForgeRunOutput {
            id: run_id,
            started_at: Utc::now(),
            code_preview: "Shell output".to_string(),
            raw_lines: Vec::new(),
            error: Some(message.to_string()),
            last_print_line: None,
        });
        self.trim_output_runs();
    }

    fn format_printable_lines(printable: &serde_json::Value) -> Vec<String> {
        if printable.is_null() {
            return Vec::new();
        }
        if let Some(text) = printable.as_str() {
            if text.is_empty() {
                return Vec::new();
            }
            return text.split('\n').map(|line| line.to_string()).collect();
        }
        let text =
            serde_json::to_string_pretty(printable).unwrap_or_else(|_| printable.to_string());
        text.split('\n').map(|line| line.to_string()).collect()
    }

    fn format_payload_lines(payload: &[serde_json::Value]) -> Vec<String> {
        let mut lines = Vec::new();
        for (idx, value) in payload.iter().enumerate() {
            let mut formatted = Self::format_printable_lines(value);
            if !formatted.is_empty() {
                lines.append(&mut formatted);
            }
            if idx + 1 < payload.len() && !lines.last().is_some_and(|line| line.is_empty()) {
                lines.push(String::new());
            }
        }
        lines
    }

    fn default_result_label_for_value(value: &serde_json::Value) -> String {
        if value.is_array() {
            "Shell Output (Array)".to_string()
        } else {
            "Shell Output (Documents)".to_string()
        }
    }

    fn build_raw_output_text(&self) -> String {
        let mut out = String::new();
        for (idx, run) in self.output_runs.iter().enumerate() {
            let time = run.started_at.format("%H:%M:%S").to_string();
            let header = if run.id == SYSTEM_RUN_ID {
                format!("[{}] {}", time, run.code_preview)
            } else {
                format!("[{}] Run #{} - {}", time, run.id, run.code_preview)
            };
            out.push_str(&header);
            out.push('\n');
            for line in &run.raw_lines {
                out.push_str(line);
                out.push('\n');
            }
            if let Some(err) = &run.error {
                out.push_str(err);
                out.push('\n');
            }
            if idx + 1 < self.output_runs.len() {
                out.push('\n');
            }
        }
        out
    }

    fn clear_output_runs(&mut self) {
        self.output_runs.clear();
        self.active_run_id = None;
        self.clear_result_pages(false);
        self.last_result = None;
        self.last_error = None;
        self.raw_output_text.clear();
        self.results_search_query.clear();
        self.sync_output_tab();
    }

    fn trim_output_runs(&mut self) {
        if self.output_runs.len() <= MAX_OUTPUT_RUNS {
            return;
        }
        let overflow = self.output_runs.len().saturating_sub(MAX_OUTPUT_RUNS);
        for _ in 0..overflow {
            self.output_runs.remove(0);
        }
        if let Some(active) = self.active_run_id
            && !self.output_runs.iter().any(|run| run.id == active)
        {
            self.active_run_id = self.output_runs.last().map(|run| run.id);
        }
    }

    fn trim_output_lines(&mut self) {
        let mut total: usize = self.output_runs.iter().map(|run| run.raw_lines.len()).sum();
        while total > MAX_OUTPUT_LINES && !self.output_runs.is_empty() {
            if self.output_runs[0].raw_lines.is_empty() {
                self.output_runs.remove(0);
                continue;
            }
            self.output_runs[0].raw_lines.remove(0);
            total = total.saturating_sub(1);
        }
    }

    fn format_result(&self, result: &mongosh::RuntimeEvaluationResult) -> String {
        if result.printable.is_string() {
            result.printable.as_str().unwrap_or("").to_string()
        } else if result.printable.is_null() {
            "null".to_string()
        } else {
            serde_json::to_string_pretty(&result.printable)
                .unwrap_or_else(|_| result.printable.to_string())
        }
    }

    fn is_trivial_printable(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Null => true,
            serde_json::Value::String(text) => {
                let trimmed = text.trim();
                trimmed.is_empty() || trimmed.eq_ignore_ascii_case("undefined")
            }
            _ => false,
        }
    }

    fn result_documents(&self, printable: &serde_json::Value) -> Option<Vec<Document>> {
        if let Some(text) = printable.as_str() {
            let trimmed = text.trim();
            if (trimmed.starts_with('{') || trimmed.starts_with('['))
                && let Ok(docs) = crate::bson::parse_documents_from_json(trimmed)
            {
                return Some(docs);
            }
            return None;
        }

        if let Some(docs) = Self::cursor_documents(printable) {
            return Some(docs);
        }

        if !matches!(printable, serde_json::Value::Object(_) | serde_json::Value::Array(_)) {
            return None;
        }

        let bson =
            Bson::try_from(printable.clone()).unwrap_or_else(|_| Self::value_to_bson(printable));

        match bson {
            Bson::Document(doc) => Some(vec![doc]),
            Bson::Array(items) => {
                let mut docs = Vec::with_capacity(items.len());
                for item in items.iter() {
                    if let Bson::Document(doc) = item {
                        docs.push(doc.clone());
                    } else {
                        let mut doc = Document::new();
                        doc.insert("value", Bson::Array(items));
                        return Some(vec![doc]);
                    }
                }
                Some(docs)
            }
            other => {
                let mut doc = Document::new();
                doc.insert("value", other);
                Some(vec![doc])
            }
        }
    }

    fn cursor_documents(printable: &serde_json::Value) -> Option<Vec<Document>> {
        let obj = printable.as_object()?;
        let docs = obj.get("documents")?.as_array()?;
        if docs.is_empty() {
            return None;
        }

        let mut out = Vec::with_capacity(docs.len());
        for item in docs {
            match Self::value_to_bson(item) {
                Bson::Document(doc) => out.push(doc),
                other => {
                    let mut doc = Document::new();
                    doc.insert("value", other);
                    out.push(doc);
                }
            }
        }

        if out.is_empty() { None } else { Some(out) }
    }

    fn value_to_bson(value: &serde_json::Value) -> Bson {
        match value {
            serde_json::Value::Null => Bson::Null,
            serde_json::Value::Bool(val) => Bson::Boolean(*val),
            serde_json::Value::Number(num) => {
                if let Some(val) = num.as_i64() {
                    Bson::Int64(val)
                } else if let Some(val) = num.as_u64() {
                    if val <= i64::MAX as u64 {
                        Bson::Int64(val as i64)
                    } else if let Some(val) = num.as_f64() {
                        Bson::Double(val)
                    } else {
                        Bson::String(num.to_string())
                    }
                } else if let Some(val) = num.as_f64() {
                    Bson::Double(val)
                } else {
                    Bson::String(num.to_string())
                }
            }
            serde_json::Value::String(val) => Bson::String(val.clone()),
            serde_json::Value::Array(items) => {
                Bson::Array(items.iter().map(Self::value_to_bson).collect())
            }
            serde_json::Value::Object(map) => {
                let mut doc = Document::new();
                for (key, val) in map {
                    doc.insert(key, Self::value_to_bson(val));
                }
                Bson::Document(doc)
            }
        }
    }

    fn set_result_documents(&mut self, docs: Vec<Document>) {
        let documents: Arc<Vec<SessionDocument>> = Arc::new(
            docs.into_iter()
                .enumerate()
                .map(|(idx, doc)| SessionDocument {
                    key: DocumentKey::from_document(&doc, idx),
                    doc,
                })
                .collect(),
        );
        let signature = results_signature(&documents);
        if self.result_signature != Some(signature) {
            self.result_signature = Some(signature);
            self.result_expanded_nodes.clear();
        }
        self.result_documents = Some(documents);
    }

    fn clear_result_pages(&mut self, keep_pinned: bool) {
        if keep_pinned {
            self.result_pages.retain(|page| page.pinned);
        } else {
            self.result_pages.clear();
        }

        if self.result_pages.is_empty() {
            self.result_page_index = 0;
            self.clear_results();
        } else {
            self.result_page_index =
                self.result_page_index.min(self.result_pages.len().saturating_sub(1));
            let docs = self.result_pages[self.result_page_index].docs.clone();
            self.set_result_documents(docs);
        }

        self.sync_output_tab();
    }

    fn push_result_page(&mut self, label: String, docs: Vec<Document>) {
        self.result_pages.push(ResultPage { label, docs: docs.clone(), pinned: false });
        self.result_page_index = self.result_pages.len().saturating_sub(1);
        self.set_result_documents(docs);
        self.last_result = None;
        self.sync_output_tab();
    }

    fn select_result_page(&mut self, index: usize) {
        if index >= self.result_pages.len() {
            return;
        }
        self.result_page_index = index;
        let docs = self.result_pages[index].docs.clone();
        self.set_result_documents(docs);
        self.result_scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    fn toggle_result_pinned(&mut self, index: usize) {
        if let Some(page) = self.result_pages.get_mut(index) {
            page.pinned = !page.pinned;
        }
    }

    fn close_result_page(&mut self, index: usize) {
        if index >= self.result_pages.len() {
            return;
        }
        let was_active = index == self.result_page_index;
        self.result_pages.remove(index);

        if self.result_pages.is_empty() {
            self.result_page_index = 0;
            self.clear_results();
            if self.last_result.is_some()
                || self.last_error.is_some()
                || self.mongosh_error.is_some()
            {
                self.output_tab = ForgeOutputTab::Results;
            } else {
                self.output_tab = ForgeOutputTab::Raw;
            }
        } else {
            if was_active {
                self.result_page_index = index.min(self.result_pages.len().saturating_sub(1));
            } else if index < self.result_page_index {
                self.result_page_index = self.result_page_index.saturating_sub(1);
            }
            let docs = self.result_pages[self.result_page_index].docs.clone();
            self.set_result_documents(docs);
        }
        self.sync_output_tab();
    }

    fn clear_results(&mut self) {
        self.result_documents = None;
        self.result_signature = None;
        self.result_expanded_nodes.clear();
    }

    fn update_run_print_label(&mut self, run_id: u64, label: String) {
        if let Some(run) = self.output_runs.iter_mut().find(|run| run.id == run_id) {
            run.last_print_line = Some(label);
        }
    }

    fn take_run_print_label(&mut self, run_id: u64) -> Option<String> {
        self.output_runs
            .iter_mut()
            .find(|run| run.id == run_id)
            .and_then(|run| run.last_print_line.take())
    }

    fn run_label(&self, run_id: u64) -> Option<String> {
        self.output_runs
            .iter()
            .find(|run| run.id == run_id)
            .map(|run| run.code_preview.clone())
            .filter(|label| !label.trim().is_empty())
    }

    fn default_result_label(&self) -> String {
        format!("Result {}", self.result_pages.len() + 1)
    }

    fn has_results(&self) -> bool {
        self.result_documents.is_some()
            || self.last_result.is_some()
            || self.last_error.is_some()
            || !self.result_pages.is_empty()
    }

    fn sync_output_tab(&mut self) {
        if self.has_results() {
            if self.output_tab == ForgeOutputTab::Raw {
                self.output_tab = ForgeOutputTab::Results;
            }
        } else {
            self.output_tab = ForgeOutputTab::Raw;
        }
    }

    fn make_suggestion(label: &str, kind: SuggestionKind, insert_text: &str) -> Suggestion {
        Suggestion { label: label.to_string(), kind, insert_text: insert_text.to_string() }
    }

    fn db_method_template(name: &str) -> Option<&'static str> {
        match name {
            "stats" => Some("stats()"),
            "getCollection" => Some("getCollection(\"\")"),
            "getSiblingDB" => Some("getSiblingDB(\"\")"),
            "runCommand" => Some("runCommand({})"),
            "listCollections" => Some("listCollections({})"),
            "createCollection" => Some("createCollection(\"\")"),
            _ => None,
        }
    }

    fn collection_method_template(name: &str) -> Option<&'static str> {
        match name {
            "find" => Some("find({})"),
            "findOne" => Some("findOne({})"),
            "aggregate" => Some("aggregate([{}])"),
            "insertOne" => Some("insertOne({})"),
            "insertMany" => Some("insertMany([{}])"),
            "updateOne" => Some("updateOne({}, {})"),
            "updateMany" => Some("updateMany({}, {})"),
            "deleteOne" => Some("deleteOne({})"),
            "deleteMany" => Some("deleteMany({})"),
            "countDocuments" => Some("countDocuments({})"),
            "distinct" => Some("distinct(\"\")"),
            "createIndex" => Some("createIndex({})"),
            "dropIndex" => Some("dropIndex(\"\")"),
            "getIndexes" => Some("getIndexes()"),
            _ => None,
        }
    }

    fn should_auto_preview(result_type: Option<&str>, code: &str) -> bool {
        let Some(code) = Self::sanitize_preview_source(code) else {
            return false;
        };
        let Some(result_type) = result_type else {
            return false;
        };
        if !result_type.contains("Cursor") {
            return false;
        }
        if result_type.contains("ChangeStream") {
            return false;
        }

        let trimmed = code.trim();
        if trimmed.is_empty() {
            return false;
        }
        let trimmed_no_semicolon = trimmed.trim_end_matches(';');
        if trimmed_no_semicolon.contains(';') {
            return false;
        }

        let lowered = trimmed_no_semicolon.to_ascii_lowercase();
        for blocked in
            [".toarray", ".itcount", ".next(", ".foreach", ".hasnext", ".pretty", ".watch("]
        {
            if lowered.contains(blocked) {
                return false;
            }
        }

        true
    }

    fn build_preview_code(code: &str) -> Option<String> {
        let trimmed = Self::sanitize_preview_source(code)?;
        if trimmed.is_empty() {
            return None;
        }
        let cleaned = trimmed.trim_end_matches(';');
        if cleaned.is_empty() {
            return None;
        }
        let lowered = cleaned.to_ascii_lowercase();
        let needs_limit = !lowered.contains(".limit(");
        let preview = if needs_limit {
            format!("({}).limit(50).toArray()", cleaned)
        } else {
            format!("({}).toArray()", cleaned)
        };
        Some(preview)
    }

    fn sanitize_preview_source(code: &str) -> Option<String> {
        let mut out: Vec<&str> = Vec::new();
        let mut in_block_comment = false;
        for line in code.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if in_block_comment {
                if let Some(end) = trimmed.find("*/") {
                    in_block_comment = false;
                    let rest = trimmed[end + 2..].trim();
                    if !rest.is_empty() {
                        out.push(rest);
                    }
                }
                continue;
            }

            if trimmed.starts_with("/*") {
                if !trimmed.contains("*/") {
                    in_block_comment = true;
                }
                continue;
            }

            if trimmed.starts_with("//") {
                continue;
            }

            out.push(line);
        }

        let cleaned = out.join("\n").trim().to_string();
        if cleaned.is_empty() {
            return None;
        }
        if cleaned.contains("//") || cleaned.contains("/*") {
            return None;
        }
        Some(cleaned)
    }

    fn build_collection_suggestions(state: &AppState) -> Vec<Suggestion> {
        let mut suggestions = Vec::new();

        // Add DB-level methods
        for (label, template) in [
            ("stats()", "stats()"),
            ("getCollection(\"\")", "getCollection(\"\")"),
            ("getSiblingDB(\"\")", "getSiblingDB(\"\")"),
            ("runCommand({})", "runCommand({})"),
            ("listCollections({})", "listCollections({})"),
            ("createCollection(\"\")", "createCollection(\"\")"),
        ] {
            suggestions.push(Self::make_suggestion(label, SuggestionKind::Method, template));
        }

        // Get collections from active connection
        if let Some(key) = state.active_forge_tab_key()
            && let Some(conn) = state.active_connection_by_id(key.connection_id)
            && let Some(collections) = conn.collections.get(&key.database)
        {
            for coll in collections {
                suggestions.push(Suggestion {
                    label: coll.clone(),
                    kind: SuggestionKind::Collection,
                    insert_text: coll.clone(),
                });
            }
        }

        suggestions
    }

    fn build_method_suggestions() -> Vec<Suggestion> {
        let mut suggestions = Vec::new();

        for method in METHODS {
            if let Some(template) = Self::collection_method_template(method) {
                suggestions.push(Self::make_suggestion(template, SuggestionKind::Method, template));
            } else {
                let insert = format!("{}()", method);
                suggestions.push(Self::make_suggestion(&insert, SuggestionKind::Method, &insert));
            }
        }

        suggestions
    }

    fn build_operator_suggestions() -> Vec<Suggestion> {
        OPERATORS
            .iter()
            .map(|op| Suggestion {
                label: op.to_string(),
                kind: SuggestionKind::Operator,
                insert_text: format!("{}: ", op),
            })
            .collect()
    }

    fn render_header(&self, cx: &App) -> impl IntoElement {
        let (database, _connection_name) = {
            let state_ref = self.state.read(cx);
            let db = state_ref
                .active_forge_tab_key()
                .map(|k| k.database.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            let conn_name = state_ref
                .active_forge_tab_key()
                .and_then(|k| state_ref.active_connection_by_id(k.connection_id))
                .map(|c| c.config.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            (db, conn_name)
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .px(spacing::md())
            .py(spacing::sm())
            .border_b_1()
            .border_color(colors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(colors::text_primary())
                            .child("Forge"),
                    )
                    .child(
                        div()
                            .px(spacing::sm())
                            .py(px(2.0))
                            .rounded(borders::radius_sm())
                            .bg(colors::bg_sidebar())
                            .text_xs()
                            .text_color(colors::text_secondary())
                            .child(database),
                    ),
            )
    }

    fn render_output(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let forge_view = cx.entity();

        let clear_button =
            Button::new("forge-output-clear").compact().ghost().label("Clear").on_click({
                let forge_view = forge_view.clone();
                move |_, _window, cx| {
                    forge_view.update(cx, |this, _cx| {
                        this.clear_output_runs();
                        if let Some(raw_state) = &this.raw_output_state {
                            this.raw_output_programmatic = true;
                            raw_state.update(_cx, |state, cx| {
                                state.set_value(String::new(), _window, cx);
                            });
                            this.raw_output_programmatic = false;
                        }
                        _cx.notify();
                    });
                }
            });

        let selected_index = match self.output_tab {
            ForgeOutputTab::Raw => 0,
            ForgeOutputTab::Results => {
                if self.result_pages.is_empty() {
                    1
                } else {
                    self.result_page_index.min(self.result_pages.len().saturating_sub(1)) + 1
                }
            }
        };

        let has_inline_result =
            self.last_result.is_some() || self.last_error.is_some() || self.mongosh_error.is_some();

        let tab_bar = TabBar::new("forge-output-tabs")
            .underline()
            .small()
            .selected_index(selected_index)
            .on_click({
                let forge_view = forge_view.clone();
                move |index, _window, cx| {
                    let index = *index;
                    forge_view.update(cx, |this, _cx| {
                        if index == 0 {
                            this.output_tab = ForgeOutputTab::Raw;
                        } else {
                            this.output_tab = ForgeOutputTab::Results;
                            if !this.result_pages.is_empty() {
                                this.select_result_page(index - 1);
                            }
                        }
                    });
                }
            })
            .children(
                std::iter::once(Tab::new().label("Raw output"))
                    .chain(if self.result_pages.is_empty() && has_inline_result {
                        vec![Tab::new().label("Shell Output")].into_iter()
                    } else {
                        Vec::new().into_iter()
                    })
                    .chain(self.result_pages.iter().enumerate().map(|(index, page)| {
                        let label = format_result_tab_label(&page.label, index);
                        let view_entity = forge_view.clone();
                        let pin_icon = if page.pinned {
                            Icon::new(IconName::Star).xsmall().text_color(colors::accent())
                        } else {
                            Icon::new(IconName::StarOff).xsmall().text_color(colors::text_muted())
                        };
                        let pin_button = div()
                            .id(("forge-result-pin", index))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded(borders::radius_sm())
                            .cursor_pointer()
                            .hover(|s| s.bg(colors::bg_hover()))
                            .child(pin_icon)
                            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                cx.stop_propagation();
                                view_entity.update(cx, |this, _cx| {
                                    this.toggle_result_pinned(index);
                                });
                            });

                        let view_entity = forge_view.clone();
                        let close_button = div()
                            .id(("forge-result-close", index))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded(borders::radius_sm())
                            .cursor_pointer()
                            .hover(|s| s.bg(colors::bg_hover()))
                            .child(
                                Icon::new(IconName::Close)
                                    .xsmall()
                                    .text_color(colors::text_muted()),
                            )
                            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                cx.stop_propagation();
                                view_entity.update(cx, |this, _cx| {
                                    this.close_result_page(index);
                                });
                            });

                        Tab::new().label(label).prefix(pin_button).suffix(close_button)
                    })),
            );

        let body: AnyElement = match self.output_tab {
            ForgeOutputTab::Results => self.render_results_body(window, cx).into_any_element(),
            ForgeOutputTab::Raw => self.render_raw_output_body(window, cx).into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .size_full()
            .px(spacing::md())
            .py(spacing::sm())
            .border_t_1()
            .border_color(colors::border())
            .bg(colors::bg_sidebar())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(div().text_xs().text_color(colors::text_muted()).child("Output"))
                            .child(tab_bar),
                    )
                    .child(clear_button),
            )
            .child(body)
    }

    fn bind_root_actions(
        &mut self,
        root: Div,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        root.on_action(cx.listener(|this, _: &RunForgeAll, _window, cx| {
            if let Some(editor_state) = &this.editor_state {
                let text = editor_state.read(cx).value().to_string();
                this.handle_execute_query(&text, cx);
                cx.stop_propagation();
            }
        }))
        .on_action(cx.listener(|this, _: &RunForgeSelectionOrStatement, window, cx| {
            this.handle_execute_selection_or_statement(window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &CancelForgeRun, _window, cx| {
            this.cancel_running(cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &ClearForgeOutput, _window, cx| {
            this.clear_output_runs();
            if let Some(raw_state) = &this.raw_output_state {
                this.raw_output_programmatic = true;
                raw_state.update(cx, |state, cx| {
                    state.set_value(String::new(), _window, cx);
                });
                this.raw_output_programmatic = false;
            }
            cx.notify();
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &FocusForgeEditor, window, cx| {
            if let Some(editor_state) = &this.editor_state {
                editor_state.update(cx, |state, cx| {
                    state.focus(window, cx);
                });
            }
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &FocusForgeOutput, window, cx| {
            match this.output_tab {
                ForgeOutputTab::Raw => {
                    let state = this.ensure_raw_output_state(window, cx);
                    state.update(cx, |state, cx| {
                        state.focus(window, cx);
                    });
                }
                ForgeOutputTab::Results => {
                    let state = this.ensure_results_search_state(window, cx);
                    state.update(cx, |state, cx| {
                        state.focus(window, cx);
                    });
                }
            }
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &FindInForgeOutput, window, cx| {
            match this.output_tab {
                ForgeOutputTab::Raw => {
                    let state = this.ensure_raw_output_state(window, cx);
                    state.update(cx, |state, cx| {
                        state.focus(window, cx);
                    });
                    cx.dispatch_action(&gpui_component::input::Search);
                }
                ForgeOutputTab::Results => {
                    let state = this.ensure_results_search_state(window, cx);
                    state.update(cx, |state, cx| {
                        state.focus(window, cx);
                    });
                }
            }
            cx.stop_propagation();
        }))
    }

    fn render_results_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut body =
            div().flex().flex_col().flex_1().min_w(px(0.0)).min_h(px(0.0)).overflow_hidden();

        if let Some(err) = &self.mongosh_error {
            body = body.child(
                div()
                    .px(spacing::sm())
                    .py(spacing::xs())
                    .text_sm()
                    .text_color(colors::text_error())
                    .child(format!("Forge runtime error: {err}")),
            );
        } else if let Some(err) = &self.last_error {
            body = body.child(
                div()
                    .px(spacing::sm())
                    .py(spacing::xs())
                    .text_sm()
                    .text_color(colors::text_error())
                    .child(err.clone()),
            );
        }

        let search_state = self.ensure_results_search_state(window, cx);
        let current_search = search_state.read(cx).value().to_string();
        if current_search != self.results_search_query {
            search_state.update(cx, |state, cx| {
                state.set_value(self.results_search_query.clone(), window, cx);
            });
        }
        let search_input = Input::new(&search_state)
            .appearance(true)
            .bordered(true)
            .focus_bordered(true)
            .w(px(220.0));
        body = body.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .px(spacing::sm())
                .py(spacing::xs())
                .child(search_input),
        );

        if let Some(documents) = self.result_documents.clone() {
            let expanded_nodes = &self.result_expanded_nodes;
            let mut visible_rows = build_visible_rows(&documents, expanded_nodes);
            if !self.results_search_query.trim().is_empty() {
                visible_rows =
                    filter_visible_rows(&documents, visible_rows, &self.results_search_query);
            }
            let visible_rows = Arc::new(visible_rows);
            let row_count = visible_rows.len();
            let scroll_handle = self.result_scroll.clone();
            let view_entity = cx.entity();

            let header = div()
                .flex()
                .items_center()
                .px(spacing::lg())
                .py(spacing::xs())
                .bg(colors::bg_header())
                .border_b_1()
                .border_color(colors::border())
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .text_color(colors::text_muted())
                        .child("Key"),
                )
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .text_color(colors::text_muted())
                        .child("Value"),
                )
                .child(div().w(px(120.0)).text_xs().text_color(colors::text_muted()).child("Type"));

            if documents.is_empty() {
                body = body.child(div().flex().flex_1().items_center().justify_center().child(
                    div().text_sm().text_color(colors::text_muted()).child("No documents returned"),
                ));
            } else if row_count == 0 {
                body = body.child(div().flex().flex_1().items_center().justify_center().child(
                    div().text_sm().text_color(colors::text_muted()).child("No matching results"),
                ));
            } else {
                let list = div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(
                        uniform_list(
                            "forge-results-tree",
                            row_count,
                            cx.processor({
                                let documents = documents.clone();
                                let visible_rows = visible_rows.clone();
                                let view_entity = view_entity.clone();
                                move |_view, range: std::ops::Range<usize>, _window, _cx| {
                                    range
                                        .map(|ix| {
                                            let row = &visible_rows[ix];
                                            let meta = compute_row_meta(row, &documents);
                                            render_forge_row(ix, row, &meta, view_entity.clone())
                                        })
                                        .collect()
                                }
                            }),
                        )
                        .flex_1()
                        .track_scroll(scroll_handle),
                    );

                body = body.child(header).child(list);
            }
        } else {
            let (text, color) = if let Some(result) = &self.last_result {
                (result.clone(), colors::text_secondary())
            } else if self.is_running {
                ("Running...".to_string(), colors::text_secondary())
            } else {
                ("No output yet.".to_string(), colors::text_muted())
            };

            body = body.child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .text_xs()
                    .font_family(fonts::mono())
                    .text_color(color)
                    .child(text),
            );
        }

        body
    }

    fn render_raw_output_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.ensure_raw_output_state(window, cx);
        let text = self.build_raw_output_text();
        if text != self.raw_output_text {
            self.raw_output_text = text.clone();
        }
        let current = state.read(cx).value().to_string();
        if current != text {
            self.raw_output_programmatic = true;
            state.update(cx, |state, cx| {
                state.set_value(text, window, cx);
            });
            self.raw_output_programmatic = false;
        }

        Input::new(&state)
            .h_full()
            .appearance(false)
            .bordered(false)
            .focus_bordered(false)
            .font_family(fonts::mono())
            .text_xs()
            .text_color(colors::text_secondary())
    }
}

impl Render for ForgeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.clone();

        // Check if we have an active Forge tab
        let has_forge_tab = state.read(cx).active_forge_tab_id().is_some();
        log::debug!("ForgeView::render - has_forge_tab: {}", has_forge_tab);

        // If no Forge tab, return empty placeholder
        if !has_forge_tab {
            return div().size_full().into_any_element();
        }

        self.ensure_editor_state(window, cx);
        self.sync_active_tab_content(window, cx, false);
        let Some(editor_state) = &self.editor_state else {
            return div().size_full().into_any_element();
        };
        if self.editor_focus_requested {
            self.editor_focus_requested = false;
            let focus = editor_state.read(cx).focus_handle(cx);
            window.focus(&focus);
        };
        let forge_view = cx.entity();
        let editor_child: AnyElement = Input::new(editor_state)
            .appearance(false)
            .font_family(fonts::mono())
            .text_sm()
            .text_color(colors::text_primary())
            .h_full()
            .w_full()
            .into_any_element();
        let editor_for_focus = editor_state.downgrade();
        let forge_focus_handle = self.focus_handle.clone();
        let status_text = if self.mongosh_error.is_some() {
            "Shell error"
        } else if self.is_running {
            "Running..."
        } else {
            "Ready"
        };

        let editor_panel = {
            let mut panel = div()
                .id("forge-editor-container")
                .relative()
                .flex()
                .min_h(px(0.0))
                .p(spacing::md())
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .h_full()
                        .border_1()
                        .border_color(colors::border())
                        .rounded(borders::radius_sm())
                        .bg(colors::bg_app())
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            window.focus(&forge_focus_handle);
                            if let Some(editor) = editor_for_focus.upgrade() {
                                let focus = editor.read(cx).focus_handle(cx);
                                window.focus(&focus);
                            }
                        })
                        .child(editor_child),
                );

            if self.output_visible {
                panel = panel.flex_1().min_h(px(0.0));
            } else {
                panel = panel.flex_1();
            }
            panel
        };

        let output_panel = if self.output_visible {
            Some(self.render_output(window, cx).into_any_element())
        } else {
            None
        };

        let show_output_button = if self.output_visible {
            None
        } else {
            Some(
                Button::new("forge-output-show")
                    .ghost()
                    .compact()
                    .icon(Icon::new(IconName::ChevronDown).xsmall())
                    .label("Show output")
                    .on_click({
                        let forge_view = forge_view.clone();
                        move |_, _window, cx| {
                            forge_view.update(cx, |this, _cx| {
                                this.output_visible = true;
                            });
                        }
                    })
                    .into_any_element(),
            )
        };

        let split_panel = if self.output_visible {
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
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(colors::bg_app())
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
                    .border_t_1()
                    .border_color(colors::border())
                    .bg(colors::bg_sidebar())
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors::text_muted())
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
                                div().text_xs().text_color(colors::text_muted()).child(status_text),
                            )
                            .child(
                                Button::new("forge-restart")
                                    .ghost()
                                    .compact()
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
            );

        self.bind_root_actions(root, window, cx).into_any_element()
    }
}

// ============================================================================
// Forge Results Tree Rendering (Aggregation-style)
// ============================================================================

fn render_forge_row(
    ix: usize,
    row: &VisibleRow,
    meta: &crate::views::documents::tree::lazy_row::LazyRowMeta,
    view_entity: Entity<ForgeView>,
) -> AnyElement {
    let node_id = row.node_id.clone();
    let depth = row.depth;
    let is_folder = row.is_folder;
    let is_expanded = row.is_expanded;

    let key_label = meta.key_label.clone();
    let value_label = meta.value_label.clone();
    let value_color = meta.value_color;
    let type_label = meta.type_label.clone();

    let leading = if is_folder {
        let toggle_node_id = node_id.clone();
        let toggle_view = view_entity.clone();
        div()
            .id(("forge-row-chevron", ix))
            .w(px(14.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                if event.click_count == 1 {
                    cx.stop_propagation();
                    toggle_view.update(cx, |this, cx| {
                        if this.result_expanded_nodes.contains(&toggle_node_id) {
                            this.result_expanded_nodes.remove(&toggle_node_id);
                        } else {
                            this.result_expanded_nodes.insert(toggle_node_id.clone());
                        }
                        cx.notify();
                    });
                }
            })
            .child(
                Icon::new(if is_expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                    .xsmall()
                    .text_color(colors::text_muted()),
            )
            .into_any_element()
    } else {
        div().w(px(14.0)).into_any_element()
    };

    div()
        .id(("forge-result-row", ix))
        .flex()
        .items_center()
        .w_full()
        .px(spacing::lg())
        .py(spacing::xs())
        .hover(|s| s.bg(colors::list_hover()))
        .on_mouse_down(MouseButton::Left, {
            let node_id = node_id.clone();
            let row_view = view_entity.clone();
            move |event, _window, cx| {
                if event.click_count == 2 && is_folder {
                    row_view.update(cx, |this, cx| {
                        if this.result_expanded_nodes.contains(&node_id) {
                            this.result_expanded_nodes.remove(&node_id);
                        } else {
                            this.result_expanded_nodes.insert(node_id.clone());
                        }
                        cx.notify();
                    });
                }
            }
        })
        .child(render_forge_key_column(depth, leading, &key_label))
        .child(render_forge_value_column(&value_label, value_color))
        .child(
            div()
                .w(px(120.0))
                .text_sm()
                .text_color(colors::text_muted())
                .overflow_hidden()
                .text_ellipsis()
                .child(type_label),
        )
        .into_any_element()
}

fn render_forge_key_column(depth: usize, leading: AnyElement, key_label: &str) -> impl IntoElement {
    let key_label = key_label.to_string();

    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .flex_1()
        .min_w(px(0.0))
        .overflow_hidden()
        .pl(px(14.0 * depth as f32))
        .child(leading)
        .child(
            div()
                .text_sm()
                .text_color(colors::syntax_key())
                .overflow_hidden()
                .text_ellipsis()
                .child(key_label),
        )
}

fn render_forge_value_column(value_label: &str, value_color: Rgba) -> impl IntoElement {
    div().flex_1().min_w(px(0.0)).overflow_hidden().child(
        div()
            .text_sm()
            .text_color(value_color)
            .overflow_hidden()
            .text_ellipsis()
            .child(value_label.to_string()),
    )
}

fn results_signature(documents: &[SessionDocument]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    documents.len().hash(&mut hasher);
    for doc in documents {
        doc.key.hash(&mut hasher);
    }
    hasher.finish()
}

// ============================================================================
// Completion Helpers
// ============================================================================

fn active_forge_session_info(state: &AppState) -> Option<(uuid::Uuid, String, String)> {
    let key = state.active_forge_tab_key()?.clone();
    let uri = state.connection_uri(key.connection_id)?;
    Some((key.id, uri, key.database))
}

fn line_prefix_for_offset(rope: &Rope, offset: usize) -> (String, usize) {
    let offset = offset.min(rope.len());
    let position = rope.offset_to_position(offset);
    let line_start = rope.line_start_offset(position.line as usize);
    let line = rope.slice_line(position.line as usize).to_string();
    let prefix_len = offset.saturating_sub(line_start).min(line.len());
    let prefix = line.get(..prefix_len).unwrap_or("").to_string();
    (prefix, line_start)
}

fn completion_range(rope: &Rope, start: usize, end: usize) -> Range {
    let start = start.min(rope.len());
    let end = end.min(rope.len());
    let start_pos = rope.offset_to_position(start);
    let end_pos = rope.offset_to_position(end);
    Range::new(start_pos, end_pos)
}

fn completion_token(line_prefix: &str, context: Option<ContextKind>) -> (String, usize) {
    match context {
        Some(ContextKind::Collections) => {
            if let Some(db_pos) = line_prefix.rfind("db.") {
                let start = db_pos + 3;
                return (line_prefix[start..].to_string(), start);
            }
        }
        Some(ContextKind::Methods) => {
            if let Some(dot_pos) = line_prefix.rfind('.') {
                let start = dot_pos + 1;
                return (line_prefix[start..].to_string(), start);
            }
        }
        Some(ContextKind::Operators) => {
            if let Some(dollar_pos) = line_prefix.rfind('$') {
                let start = dollar_pos;
                return (line_prefix[start..].to_string(), start);
            }
        }
        None => {}
    }
    (String::new(), line_prefix.len())
}

fn matches_token(candidate: &str, token: &str, context: Option<ContextKind>) -> bool {
    if token.is_empty() {
        return true;
    }
    let candidate = if matches!(context, Some(ContextKind::Methods)) {
        candidate.split_once('(').map(|(base, _)| base).unwrap_or(candidate)
    } else {
        candidate
    };
    candidate.starts_with(token)
}

fn merge_suggestions(
    local: Vec<Suggestion>,
    mongosh: Vec<String>,
    context: Option<ContextKind>,
    completion_prefix: &str,
    token: &str,
) -> Vec<Suggestion> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for suggestion in local {
        if !matches_token(&suggestion.label, token, context) {
            continue;
        }
        let label = suggestion.label.clone();
        if seen.insert(label.clone()) {
            out.push(suggestion);
        }
        if let Some((base, _)) = label.split_once('(')
            && !base.is_empty()
        {
            seen.insert(base.to_string());
        }
    }

    for completion in mongosh {
        let suffix = strip_completion_prefix(&completion, completion_prefix);
        let normalized = normalize_completion(&suffix, context);
        if normalized.is_empty() {
            continue;
        }
        if !matches_token(&normalized, token, context) {
            continue;
        }
        let looks_like_operator = normalized.starts_with('$');
        let suggestion = match context {
            Some(ContextKind::Collections) => ForgeView::db_method_template(&normalized)
                .map(|template| {
                    ForgeView::make_suggestion(template, SuggestionKind::Method, template)
                })
                .unwrap_or_else(|| Suggestion {
                    label: normalized.clone(),
                    kind: SuggestionKind::Collection,
                    insert_text: normalized,
                }),
            Some(ContextKind::Methods) => ForgeView::collection_method_template(&normalized)
                .map(|template| {
                    ForgeView::make_suggestion(template, SuggestionKind::Method, template)
                })
                .unwrap_or_else(|| Suggestion {
                    label: normalized.clone(),
                    kind: SuggestionKind::Method,
                    insert_text: normalized,
                }),
            Some(ContextKind::Operators) => Suggestion {
                label: normalized.clone(),
                kind: SuggestionKind::Operator,
                insert_text: format!("{}: ", normalized),
            },
            None => Suggestion {
                label: normalized.clone(),
                kind: if looks_like_operator {
                    SuggestionKind::Operator
                } else {
                    SuggestionKind::Method
                },
                insert_text: if looks_like_operator {
                    format!("{}: ", normalized)
                } else {
                    normalized
                },
            },
        };

        if seen.insert(suggestion.label.clone()) {
            out.push(suggestion);
        }
    }

    out
}

fn normalize_completion(completion: &str, context: Option<ContextKind>) -> String {
    let completion = completion.trim();
    if completion.is_empty() {
        return String::new();
    }

    match context {
        Some(ContextKind::Collections) => {
            completion.strip_prefix("db.").unwrap_or(completion).to_string()
        }
        Some(ContextKind::Methods) => {
            completion.rsplit('.').next().unwrap_or(completion).to_string()
        }
        Some(ContextKind::Operators) | None => completion.to_string(),
    }
}

fn strip_completion_prefix(completion: &str, prefix: &str) -> String {
    if completion.is_empty() {
        return String::new();
    }

    if let Some(stripped) = completion.strip_prefix(prefix) {
        return stripped.trim_start_matches(['.', ' ', '\t']).to_string();
    }

    let trimmed_prefix = prefix.trim_end();
    if trimmed_prefix.len() != prefix.len()
        && let Some(stripped) = completion.strip_prefix(trimmed_prefix)
    {
        return stripped.trim_start_matches(['.', ' ', '\t']).to_string();
    }

    completion.trim_start_matches(['.', ' ', '\t']).to_string()
}

fn suggestions_to_completion_items(
    suggestions: Vec<Suggestion>,
    replace_range: &Range,
) -> Vec<CompletionItem> {
    suggestions
        .into_iter()
        .map(|suggestion| {
            let kind = match suggestion.kind {
                SuggestionKind::Collection => CompletionItemKind::FIELD,
                SuggestionKind::Method => CompletionItemKind::METHOD,
                SuggestionKind::Operator => CompletionItemKind::OPERATOR,
            };
            CompletionItem {
                label: suggestion.label.clone(),
                kind: Some(kind),
                detail: Some(suggestion.kind.as_str().to_string()),
                text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                    new_text: suggestion.insert_text,
                    insert: *replace_range,
                    replace: *replace_range,
                })),
                ..Default::default()
            }
        })
        .collect()
}

// ============================================================================
// Context Detection
// ============================================================================

#[derive(Clone, Copy)]
enum ContextKind {
    /// After "db." - show collections
    Collections,
    /// After "db.collectionName." - show methods
    Methods,
    /// After "$" - show operators
    Operators,
}

/// Detects the completion context from the text.
fn detect_context(text: &str) -> Option<ContextKind> {
    // Check if we're typing an operator (after $)
    if let Some(dollar_pos) = text.rfind('$') {
        let after_dollar = &text[dollar_pos + 1..];
        // If we're still typing the operator name (no space or colon yet)
        if !after_dollar.contains(':') && !after_dollar.contains(' ') && !after_dollar.contains('}')
        {
            return Some(ContextKind::Operators);
        }
    }

    // Check for method chaining: only after collection access
    if let Some(last_dot) = text.rfind('.') {
        let after_dot = &text[last_dot + 1..];

        // Only trigger if we're typing after the dot (alphanumeric chars only)
        if after_dot.chars().all(|c| c.is_alphanumeric() || c == '_') {
            let before_dot = &text[..last_dot];

            if looks_like_collection_access(before_dot) {
                return Some(ContextKind::Methods);
            }
        }
    }

    // Look for "db." at the end (collection completion)
    if let Some(db_pos) = text.rfind("db.") {
        let after_db = &text[db_pos + 3..];
        // Only show collections if typing right after "db." with no dots
        if !after_db.contains('.') && after_db.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Some(ContextKind::Collections);
        }
    }

    None
}

fn looks_like_collection_access(text: &str) -> bool {
    let trimmed = text.trim_end();
    let Some(rest) = trimmed.strip_prefix("db.") else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }

    if rest.starts_with('[') {
        return rest.ends_with(']');
    }

    if let Some((name, _args)) = rest.split_once('(') {
        let name = name.trim_end();
        if name == "getCollection" {
            return rest.ends_with(')');
        }
        return false;
    }

    rest.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

fn should_skip_completion(line_prefix: &str) -> bool {
    let trimmed = line_prefix.trim_start();
    if trimmed.starts_with("//") {
        return true;
    }
    if let Some(start) = trimmed.find("/*") {
        let rest = &trimmed[start + 2..];
        if !rest.contains("*/") {
            return true;
        }
    }
    false
}

fn auto_indent_between_braces(
    state: &mut InputState,
    window: &mut Window,
    cx: &mut Context<InputState>,
) -> bool {
    let text = state.value().to_string();
    let cursor = state.cursor();
    if cursor >= text.len() {
        return false;
    }

    let bytes = text.as_bytes();
    let close = bytes[cursor];
    let open = match close {
        b'}' => b'{',
        b']' => b'[',
        _ => return false,
    };

    let mut line_start = cursor;
    while line_start > 0 {
        if bytes[line_start - 1] == b'\n' {
            break;
        }
        line_start -= 1;
    }

    let current_indent = &text[line_start..cursor];
    if !current_indent.chars().all(|c| c.is_whitespace()) {
        return false;
    }

    let mut idx = line_start;
    let mut prev_non_ws = None;
    while idx > 0 {
        let b = bytes[idx - 1];
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            idx -= 1;
            continue;
        }
        prev_non_ws = Some(b);
        break;
    }

    if prev_non_ws != Some(open) {
        return false;
    }

    let extra_indent = " ".repeat(TabSize::default().tab_size);
    if extra_indent.is_empty() {
        return false;
    }

    let insert = format!("{extra_indent}\n{current_indent}");
    let mut new_text = String::with_capacity(text.len() + insert.len());
    new_text.push_str(&text[..cursor]);
    new_text.push_str(&insert);
    new_text.push_str(&text[cursor..]);

    state.set_value(new_text, window, cx);
    let new_cursor_offset = cursor + extra_indent.len();
    let position = state.text().offset_to_position(new_cursor_offset);
    state.set_cursor_position(position, window, cx);
    true
}

// ============================================================================
// Constants
// ============================================================================

const METHODS: &[&str] = &[
    "find",
    "findOne",
    "aggregate",
    "insertOne",
    "insertMany",
    "updateOne",
    "updateMany",
    "deleteOne",
    "deleteMany",
    "countDocuments",
    "distinct",
    "createIndex",
    "dropIndex",
    "getIndexes",
];

const OPERATORS: &[&str] = &[
    "$match",
    "$project",
    "$group",
    "$sort",
    "$limit",
    "$skip",
    "$unwind",
    "$lookup",
    "$addFields",
    "$set",
    "$unset",
    "$replaceRoot",
    "$replaceWith",
    "$bucket",
    "$bucketAuto",
    "$count",
    "$facet",
    "$out",
    "$merge",
    "$sample",
    "$unionWith",
    "$redact",
    "$graphLookup",
];
