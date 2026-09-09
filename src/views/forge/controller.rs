use std::sync::Arc;

use gpui_kit::{Context, Window};

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
        view.sync_raw_output(window, cx);
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
        view.state.output.auto_select_results = false;
        match view.state.output.output_tab {
            ForgeOutputTab::Raw => {
                let state = view.ensure_raw_output_state(window, cx);
                view.sync_raw_output(window, cx);
                state.update(cx, |state, cx| {
                    state.focus(window, cx);
                });
            }
            ForgeOutputTab::Results => {
                if view.current_result_documents().is_none() {
                    window.focus(&view.state.focus_handle, cx);
                    cx.notify();
                    return;
                }
                let state = view.ensure_results_search_state(window, cx);
                state.update(cx, |state, cx| {
                    state.focus(window, cx);
                });
            }
        }
    }

    pub fn find_in_output(view: &mut ForgeView, window: &mut Window, cx: &mut Context<ForgeView>) {
        view.state.output.auto_select_results = false;
        match view.state.output.output_tab {
            ForgeOutputTab::Raw => {
                let state = view.ensure_raw_output_state(window, cx);
                view.sync_raw_output(window, cx);
                state.update(cx, |state, cx| {
                    state.focus(window, cx);
                });
                cx.dispatch_action(&gpui_kit::component::input::Search);
            }
            ForgeOutputTab::Results => {
                if view.current_result_documents().is_none() {
                    view.state.output.output_tab = ForgeOutputTab::Raw;
                    Self::find_in_output(view, window, cx);
                    cx.notify();
                    return;
                }
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
        let Ok(Some((session_id, _, _))) =
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
                // Console text is exactly what the shell printed; payloads are
                // only for the structured result view.
                view.append_output_lines(resolved_run_id, lines);

                if let Some(values) = payload {
                    if let Some(active_run) = view.state.output.active_run_id
                        && resolved_run_id == active_run
                    {
                        let label = Self::take_run_print_label(view, resolved_run_id)
                            .unwrap_or_else(|| "Printed documents".to_string());
                        let total = values.len();
                        for (idx, value) in values.into_iter().enumerate() {
                            if let Some(docs) = ForgeView::result_documents(&value) {
                                let tab_label = if total > 1 {
                                    format!("{} ({}/{})", label, idx + 1, total)
                                } else {
                                    label.clone()
                                };
                                Self::push_printed_result_page(
                                    view,
                                    resolved_run_id,
                                    tab_label,
                                    docs,
                                );
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
