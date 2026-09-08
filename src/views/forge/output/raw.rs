use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::*;

use super::super::ForgeView;
use crate::theme::fonts;

impl ForgeView {
    pub fn ensure_raw_output_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        if let Some(state) = self.state.output.raw_output_state.as_ref() {
            return state.clone();
        }

        let raw_state = cx.new(|cx| {
            TextareaState::new(window, cx).searchable(true).placeholder("No output yet.")
        });

        self.state.output.raw_output_state = Some(raw_state.clone());
        raw_state
    }

    pub fn build_raw_output_text(&self) -> String {
        let mut out = String::new();
        for (idx, run) in self.state.output.output_runs.iter().enumerate() {
            let time = run.started_at.format("%H:%M:%S").to_string();
            let header = if run.id == super::super::types::SYSTEM_RUN_ID {
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
            if idx + 1 < self.state.output.output_runs.len() {
                out.push('\n');
            }
        }
        out
    }

    pub fn render_raw_output_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.ensure_raw_output_state(window, cx);
        let text = self.build_raw_output_text();
        if text != self.state.output.raw_output_text {
            self.state.output.raw_output_text = text.clone();
        }
        let current = state.read(cx).value().to_string();
        if current != text {
            state.update(cx, |state, cx| {
                state.set_value(text, window, cx);
            });
        }

        Textarea::new(&state)
            .readonly(true)
            .h_full()
            .appearance(false)
            .bordered(false)
            .font_family(fonts::mono())
            .text_xs()
            .text_color(cx.theme().secondary_foreground)
    }
}
