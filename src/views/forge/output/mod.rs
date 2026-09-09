mod format;
mod raw;
mod results;

pub use super::logic::result_documents as documents_from_printable;
pub use format::format_result_tab_label;
pub use raw::RawOutputState;

use super::ForgeView;
use super::types::{ForgeRunOutput, MAX_OUTPUT_LINES, MAX_OUTPUT_RUNS, SYSTEM_RUN_ID};
use chrono::Utc;

impl ForgeView {
    pub fn begin_run(&mut self, run_id: u64, code: &str) {
        let preview = Self::code_preview(code);
        self.state.output.output_runs.push(ForgeRunOutput {
            id: run_id,
            started_at: Utc::now(),
            code_preview: preview,
            raw_lines: Vec::new(),
            evaluation_lines: Vec::new(),
            error: None,
            last_print_line: None,
        });
        self.state.output.active_run_id = Some(run_id);
        self.state.output.output_visible = true;
        self.state.output.output_tab = super::types::ForgeOutputTab::Raw;
        self.state.output.auto_select_results = true;
        self.state.output.raw.dirty = true;
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
        if !self.state.output.output_runs.iter().any(|run| run.id == SYSTEM_RUN_ID) {
            self.state.output.output_runs.push(ForgeRunOutput {
                id: SYSTEM_RUN_ID,
                started_at: Utc::now(),
                code_preview: "Shell output".to_string(),
                raw_lines: Vec::new(),
                evaluation_lines: Vec::new(),
                error: None,
                last_print_line: None,
            });
            self.trim_output_runs();
            self.state.output.raw.dirty = true;
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

        if let Some(run) = self.state.output.output_runs.iter_mut().find(|run| run.id == run_id) {
            run.raw_lines.extend(normalized);
        } else {
            self.state.output.output_runs.push(ForgeRunOutput {
                id: run_id,
                started_at: Utc::now(),
                code_preview: "Shell output".to_string(),
                raw_lines: normalized,
                evaluation_lines: Vec::new(),
                error: None,
                last_print_line: None,
            });
            self.trim_output_runs();
        }

        self.trim_output_lines();
        self.state.output.raw.dirty = true;
    }

    pub fn append_eval_output(&mut self, run_id: u64, printable: &serde_json::Value) {
        let lines = Self::format_printable_lines(printable);
        if lines.is_empty() {
            return;
        }
        if let Some(run) = self.state.output.output_runs.iter_mut().find(|run| run.id == run_id) {
            run.evaluation_lines = lines;
        } else {
            self.state.output.output_runs.push(ForgeRunOutput {
                id: run_id,
                started_at: Utc::now(),
                code_preview: "Shell output".to_string(),
                raw_lines: Vec::new(),
                evaluation_lines: lines,
                error: None,
                last_print_line: None,
            });
            self.trim_output_runs();
        }
        self.trim_output_lines();
        self.state.output.raw.dirty = true;
    }

    pub fn append_error_output(&mut self, run_id: u64, message: &str) {
        self.state.output.raw.dirty = true;
        if let Some(run) = self.state.output.output_runs.iter_mut().find(|run| run.id == run_id) {
            run.error = Some(message.to_string());
            return;
        }

        self.state.output.output_runs.push(ForgeRunOutput {
            id: run_id,
            started_at: Utc::now(),
            code_preview: "Shell output".to_string(),
            raw_lines: Vec::new(),
            evaluation_lines: Vec::new(),
            error: Some(message.to_string()),
            last_print_line: None,
        });
        self.trim_output_runs();
    }

    pub fn clear_output_runs(&mut self) {
        self.state.output.output_runs.clear();
        if !self.state.runtime.is_running {
            self.state.output.active_run_id = None;
        }
        super::controller::ForgeController::clear_result_pages(self, false);
        self.state.output.last_result = None;
        self.state.output.last_error = None;
        self.state.output.raw.clear();
        self.state.output.trimmed_output_lines = 0;
        self.state.output.skipped_output_events = 0;
        self.state.output.output_tab = super::types::ForgeOutputTab::Raw;
        self.state.output.auto_select_results = true;
        self.state.output.results_search_query.clear();
        super::controller::ForgeController::sync_output_tab(self);
    }

    pub fn trim_output_runs(&mut self) {
        if self.state.output.output_runs.len() <= MAX_OUTPUT_RUNS {
            return;
        }
        let overflow = self.state.output.output_runs.len().saturating_sub(MAX_OUTPUT_RUNS);
        self.state.output.trimmed_output_lines += self.state.output.output_runs[..overflow]
            .iter()
            .map(|run| run.raw_lines.len() + run.evaluation_lines.len())
            .sum::<usize>();
        self.state.output.output_runs.drain(..overflow);
        self.state.output.raw.reset_history = true;
        self.state.output.raw.dirty = true;
        if let Some(active) = self.state.output.active_run_id
            && !self.state.output.output_runs.iter().any(|run| run.id == active)
        {
            self.state.output.active_run_id =
                self.state.output.output_runs.last().map(|run| run.id);
        }
    }

    pub fn trim_output_lines(&mut self) {
        let total: usize = self
            .state
            .output
            .output_runs
            .iter()
            .map(|run| run.raw_lines.len() + run.evaluation_lines.len())
            .sum();
        let mut overflow = total.saturating_sub(MAX_OUTPUT_LINES);
        if overflow > 0 {
            self.state.output.trimmed_output_lines += overflow;
            self.state.output.raw.reset_history = true;
        }
        for run in &mut self.state.output.output_runs {
            let count = overflow.min(run.raw_lines.len());
            run.raw_lines.drain(..count);
            overflow -= count;
            let count = overflow.min(run.evaluation_lines.len());
            run.evaluation_lines.drain(..count);
            overflow -= count;
            if overflow == 0 {
                break;
            }
        }
    }
}
