mod format;
mod pipeline;
mod raw;
mod results;

pub use format::format_result_tab_label;
pub use pipeline::documents_from_printable;

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
            error: None,
            last_print_line: None,
        });
        self.state.output.active_run_id = Some(run_id);
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

        if let Some(run) = self.state.output.output_runs.iter_mut().find(|run| run.id == run_id) {
            run.raw_lines.extend(normalized);
        } else {
            self.state.output.output_runs.push(ForgeRunOutput {
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
        if let Some(run) = self.state.output.output_runs.iter_mut().find(|run| run.id == run_id) {
            run.error = Some(message.to_string());
            return;
        }

        self.state.output.output_runs.push(ForgeRunOutput {
            id: run_id,
            started_at: Utc::now(),
            code_preview: "Shell output".to_string(),
            raw_lines: Vec::new(),
            error: Some(message.to_string()),
            last_print_line: None,
        });
        self.trim_output_runs();
    }

    pub fn clear_output_runs(&mut self) {
        self.state.output.output_runs.clear();
        self.state.output.active_run_id = None;
        super::controller::ForgeController::clear_result_pages(self, false);
        self.state.output.last_result = None;
        self.state.output.last_error = None;
        self.state.output.raw_output_text.clear();
        self.state.output.results_search_query.clear();
        super::controller::ForgeController::sync_output_tab(self);
    }

    pub fn trim_output_runs(&mut self) {
        if self.state.output.output_runs.len() <= MAX_OUTPUT_RUNS {
            return;
        }
        let overflow = self.state.output.output_runs.len().saturating_sub(MAX_OUTPUT_RUNS);
        for _ in 0..overflow {
            self.state.output.output_runs.remove(0);
        }
        if let Some(active) = self.state.output.active_run_id
            && !self.state.output.output_runs.iter().any(|run| run.id == active)
        {
            self.state.output.active_run_id =
                self.state.output.output_runs.last().map(|run| run.id);
        }
    }

    pub fn trim_output_lines(&mut self) {
        let mut total: usize =
            self.state.output.output_runs.iter().map(|run| run.raw_lines.len()).sum();
        while total > MAX_OUTPUT_LINES && !self.state.output.output_runs.is_empty() {
            if self.state.output.output_runs[0].raw_lines.is_empty() {
                self.state.output.output_runs.remove(0);
                continue;
            }
            self.state.output.output_runs[0].raw_lines.remove(0);
            total = total.saturating_sub(1);
        }
    }
}
