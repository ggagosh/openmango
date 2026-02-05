use std::sync::Arc;

use gpui::{Context, Window};

use super::ForgeView;
use super::runtime::ForgeRuntime;
use super::types::ForgeOutputTab;

pub struct ForgeController {
    pub runtime: Arc<ForgeRuntime>,
}

impl ForgeController {
    pub fn new() -> Self {
        Self { runtime: Arc::new(ForgeRuntime::new()) }
    }

    pub fn run_all(view: &mut ForgeView, cx: &mut Context<ForgeView>) {
        if let Some(editor_state) = &view.state.editor.editor_state {
            let text = editor_state.read(cx).value().to_string();
            view.handle_execute_query(&text, cx);
        }
    }

    pub fn run_selection_or_statement(
        view: &mut ForgeView,
        window: &mut Window,
        cx: &mut Context<ForgeView>,
    ) {
        view.handle_execute_selection_or_statement(window, cx);
    }

    pub fn cancel_run(view: &mut ForgeView, cx: &mut Context<ForgeView>) {
        view.cancel_running(cx);
    }

    pub fn clear_output(view: &mut ForgeView, window: &mut Window, cx: &mut Context<ForgeView>) {
        view.clear_output_runs();
        if let Some(raw_state) = &view.state.output.raw_output_state {
            view.state.output.raw_output_programmatic = true;
            raw_state.update(cx, |state, cx| {
                state.set_value(String::new(), window, cx);
            });
            view.state.output.raw_output_programmatic = false;
        }
        cx.notify();
    }

    pub fn focus_editor(view: &mut ForgeView, window: &mut Window, cx: &mut Context<ForgeView>) {
        if let Some(editor_state) = &view.state.editor.editor_state {
            editor_state.update(cx, |state, cx| {
                state.focus(window, cx);
            });
        }
    }

    pub fn focus_output(view: &mut ForgeView, window: &mut Window, cx: &mut Context<ForgeView>) {
        match view.state.output.output_tab {
            ForgeOutputTab::Raw => {
                let state = view.ensure_raw_output_state(window, cx);
                state.update(cx, |state, cx| {
                    state.focus(window, cx);
                });
            }
            ForgeOutputTab::Results => {
                let state = view.ensure_results_search_state(window, cx);
                state.update(cx, |state, cx| {
                    state.focus(window, cx);
                });
            }
        }
    }

    pub fn find_in_output(view: &mut ForgeView, window: &mut Window, cx: &mut Context<ForgeView>) {
        match view.state.output.output_tab {
            ForgeOutputTab::Raw => {
                let state = view.ensure_raw_output_state(window, cx);
                state.update(cx, |state, cx| {
                    state.focus(window, cx);
                });
                cx.dispatch_action(&gpui_component::input::Search);
            }
            ForgeOutputTab::Results => {
                let state = view.ensure_results_search_state(window, cx);
                state.update(cx, |state, cx| {
                    state.focus(window, cx);
                });
            }
        }
    }

    pub fn handle_mongosh_event(
        view: &mut ForgeView,
        event: super::mongosh::MongoshEvent,
        cx: &mut Context<ForgeView>,
    ) {
        let Some((session_id, _, _)) =
            super::runtime::active_forge_session_info(view.app_state.read(cx))
        else {
            return;
        };
        let event_session_id = match &event {
            super::mongosh::MongoshEvent::Print { session_id, .. } => session_id,
            super::mongosh::MongoshEvent::Clear { session_id } => session_id,
        };
        if *event_session_id != session_id.to_string() {
            return;
        }

        match event {
            super::mongosh::MongoshEvent::Print { run_id, lines, payload, .. } => {
                let resolved_run_id = run_id
                    .or(view.state.output.active_run_id)
                    .unwrap_or_else(|| view.ensure_system_run());
                let last_print_line = lines.iter().rev().find_map(|line| {
                    let trimmed = line.trim();
                    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
                });
                if let Some(values) = &payload {
                    let normalized = ForgeView::format_payload_lines(values);
                    if !normalized.is_empty() {
                        view.append_output_lines(resolved_run_id, normalized);
                    } else {
                        view.append_output_lines(resolved_run_id, lines);
                    }
                } else {
                    view.append_output_lines(resolved_run_id, lines);
                }

                if let Some(values) = payload {
                    if let Some(active_run) = view.state.output.active_run_id
                        && resolved_run_id == active_run
                    {
                        let label = Self::take_run_print_label(view, resolved_run_id)
                            .unwrap_or_else(|| {
                                values
                                    .first()
                                    .map(ForgeView::default_result_label_for_value)
                                    .unwrap_or_else(|| Self::default_result_label(view))
                            });
                        let total = values.len();
                        for (idx, value) in values.into_iter().enumerate() {
                            if let Some(docs) = ForgeView::result_documents(&value) {
                                let tab_label = if total > 1 {
                                    format!("{} ({}/{})", label, idx + 1, total)
                                } else {
                                    label.clone()
                                };
                                Self::push_result_page(view, tab_label, docs);
                            }
                        }
                        Self::sync_output_tab(view);
                    }
                } else if let Some(label) = last_print_line {
                    Self::update_run_print_label(view, resolved_run_id, label);
                }
            }
            super::mongosh::MongoshEvent::Clear { .. } => {
                view.clear_output_runs();
            }
        }

        cx.notify();
    }
}
