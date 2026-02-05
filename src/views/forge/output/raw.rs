use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};

use super::super::ForgeView;
use crate::theme::{colors, fonts};

impl ForgeView {
    pub fn ensure_raw_output_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(state) = self.state.output.raw_output_state.as_ref() {
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
                    if this.state.output.raw_output_programmatic {
                        return;
                    }
                    let current = state.read(cx).value().to_string();
                    if current != this.state.output.raw_output_text {
                        this.state.output.raw_output_programmatic = true;
                        state.update(cx, |state, cx| {
                            state.set_value(this.state.output.raw_output_text.clone(), window, cx);
                        });
                        this.state.output.raw_output_programmatic = false;
                    }
                }
            });

        self.state.output.raw_output_subscription = Some(subscription);
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
            self.state.output.raw_output_programmatic = true;
            state.update(cx, |state, cx| {
                state.set_value(text, window, cx);
            });
            self.state.output.raw_output_programmatic = false;
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
