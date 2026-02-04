use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use chrono::Utc;
use gpui::*;
use gpui_component::input::{InputEvent, InputState};
use mongodb::bson::Document;

use crate::bson::DocumentKey;
use crate::state::SessionDocument;
use crate::views::documents::tree::lazy_tree::VisibleRow;

use super::ForgeView;
use super::mongosh;
use super::types::{
    ForgeOutputTab, ForgeRunOutput, MAX_OUTPUT_LINES, MAX_OUTPUT_RUNS, ResultPage, SYSTEM_RUN_ID,
};

pub fn filter_visible_rows(
    documents: &[SessionDocument],
    rows: Vec<VisibleRow>,
    query: &str,
) -> Vec<VisibleRow> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return rows;
    }

    let mut keep_ids = HashSet::new();
    for row in rows.iter() {
        let meta = crate::views::documents::tree::lazy_row::compute_row_meta(row, documents);
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

fn results_signature(documents: &[SessionDocument]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    documents.len().hash(&mut hasher);
    for doc in documents {
        doc.key.hash(&mut hasher);
    }
    hasher.finish()
}

impl ForgeView {
    pub fn ensure_raw_output_state(
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

    pub fn ensure_results_search_state(
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

    pub fn begin_run(&mut self, run_id: u64, code: &str) {
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

    pub fn code_preview(code: &str) -> String {
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

    pub fn ensure_system_run(&mut self) -> u64 {
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

    pub fn append_output_lines(&mut self, run_id: u64, lines: Vec<String>) {
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

    pub fn append_eval_output(&mut self, run_id: u64, printable: &serde_json::Value) {
        let lines = Self::format_printable_lines(printable);
        if lines.is_empty() {
            return;
        }
        self.append_output_lines(run_id, lines);
    }

    pub fn append_error_output(&mut self, run_id: u64, message: &str) {
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

    pub fn format_printable_lines(printable: &serde_json::Value) -> Vec<String> {
        super::logic::format_printable_lines(printable)
    }

    pub fn format_payload_lines(payload: &[serde_json::Value]) -> Vec<String> {
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

    pub fn default_result_label_for_value(value: &serde_json::Value) -> String {
        super::logic::default_result_label_for_value(value)
    }

    pub fn build_raw_output_text(&self) -> String {
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

    pub fn clear_output_runs(&mut self) {
        self.output_runs.clear();
        self.active_run_id = None;
        self.clear_result_pages(false);
        self.last_result = None;
        self.last_error = None;
        self.raw_output_text.clear();
        self.results_search_query.clear();
        self.sync_output_tab();
    }

    pub fn trim_output_runs(&mut self) {
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

    pub fn trim_output_lines(&mut self) {
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

    pub fn format_result(&self, result: &mongosh::RuntimeEvaluationResult) -> String {
        if result.printable.is_string() {
            result.printable.as_str().unwrap_or("").to_string()
        } else if result.printable.is_null() {
            "null".to_string()
        } else {
            serde_json::to_string_pretty(&result.printable)
                .unwrap_or_else(|_| result.printable.to_string())
        }
    }

    pub fn is_trivial_printable(value: &serde_json::Value) -> bool {
        super::logic::is_trivial_printable(value)
    }

    pub fn result_documents(printable: &serde_json::Value) -> Option<Vec<Document>> {
        super::logic::result_documents(printable)
    }

    pub fn set_result_documents(&mut self, docs: Vec<Document>) {
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

    pub fn clear_result_pages(&mut self, keep_pinned: bool) {
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

    pub fn push_result_page(&mut self, label: String, docs: Vec<Document>) {
        self.result_pages.push(ResultPage { label, docs: docs.clone(), pinned: false });
        self.result_page_index = self.result_pages.len().saturating_sub(1);
        self.set_result_documents(docs);
        self.last_result = None;
        self.sync_output_tab();
    }

    pub fn select_result_page(&mut self, index: usize) {
        if index >= self.result_pages.len() {
            return;
        }
        self.result_page_index = index;
        let docs = self.result_pages[index].docs.clone();
        self.set_result_documents(docs);
        self.result_scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    pub fn toggle_result_pinned(&mut self, index: usize) {
        if let Some(page) = self.result_pages.get_mut(index) {
            page.pinned = !page.pinned;
        }
    }

    pub fn close_result_page(&mut self, index: usize) {
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

    pub fn clear_results(&mut self) {
        self.result_documents = None;
        self.result_signature = None;
        self.result_expanded_nodes.clear();
    }

    pub fn update_run_print_label(&mut self, run_id: u64, label: String) {
        if let Some(run) = self.output_runs.iter_mut().find(|run| run.id == run_id) {
            run.last_print_line = Some(label);
        }
    }

    pub fn take_run_print_label(&mut self, run_id: u64) -> Option<String> {
        self.output_runs
            .iter_mut()
            .find(|run| run.id == run_id)
            .and_then(|run| run.last_print_line.take())
    }

    pub fn run_label(&self, run_id: u64) -> Option<String> {
        self.output_runs
            .iter()
            .find(|run| run.id == run_id)
            .map(|run| run.code_preview.clone())
            .filter(|label| !label.trim().is_empty())
    }

    pub fn default_result_label(&self) -> String {
        format!("Result {}", self.result_pages.len() + 1)
    }

    pub fn has_results(&self) -> bool {
        self.result_documents.is_some()
            || self.last_result.is_some()
            || self.last_error.is_some()
            || !self.result_pages.is_empty()
    }

    pub fn sync_output_tab(&mut self) {
        if self.has_results() {
            if self.output_tab == ForgeOutputTab::Raw {
                self.output_tab = ForgeOutputTab::Results;
            }
        } else {
            self.output_tab = ForgeOutputTab::Raw;
        }
    }
}
