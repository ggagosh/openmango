use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::*;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::state::AppState;

use super::ForgeView;
use super::mongosh::{self, MongoshBridge, MongoshEvent};

pub struct ForgeRuntime {
    bridge: Mutex<Option<Arc<MongoshBridge>>>,
}

impl ForgeRuntime {
    pub fn new() -> Self {
        Self { bridge: Mutex::new(None) }
    }

    pub fn ensure_bridge(&self) -> Result<Arc<MongoshBridge>, crate::error::Error> {
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

pub fn active_forge_session_info(state: &AppState) -> Option<(Uuid, String, String)> {
    let key = state.active_forge_tab_key()?.clone();
    let uri = state.connection_uri(key.connection_id)?;
    Some((key.id, uri, key.database))
}

impl ForgeView {
    pub fn handle_execute_query(&mut self, text: &str, cx: &mut Context<Self>) {
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
                            if let Some(docs) = Self::result_documents(&eval.printable) {
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

    pub fn restart_session(&mut self, cx: &mut Context<Self>) {
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

    pub fn cancel_running(&mut self, cx: &mut Context<Self>) {
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

    pub fn ensure_mongosh(&mut self) -> Option<Arc<MongoshBridge>> {
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

    pub fn ensure_output_listener(&mut self, cx: &mut Context<Self>) {
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

    pub fn handle_mongosh_event(&mut self, event: MongoshEvent, cx: &mut Context<Self>) {
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
                            if let Some(docs) = Self::result_documents(&value) {
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

    pub fn should_auto_preview(result_type: Option<&str>, code: &str) -> bool {
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

    pub fn build_preview_code(code: &str) -> Option<String> {
        let trimmed = Self::sanitize_preview_source(code)?;
        let trimmed = trimmed.trim_end_matches(';');
        Some(format!("{}.limit(50).toArray()", trimmed))
    }

    pub fn sanitize_preview_source(code: &str) -> Option<String> {
        let trimmed = code.trim();
        if trimmed.is_empty() {
            return None;
        }
        let trimmed = trimmed.trim_end_matches(';').trim();
        if trimmed.contains(';') {
            return None;
        }
        Some(trimmed.to_string())
    }
}
