//! WebView wrapper for the Forge editor integration.
//!
//! This module provides a WebView component that embeds the Forge editor for the query shell.

use gpui::*;
use gpui_component::webview::WebView;
use gpui_component::wry;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::assets::EmbeddedAssets;

// ============================================================================
// IPC Message Types
// ============================================================================

/// Messages received from the Forge editor via IPC.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcMessage {
    /// Editor is ready
    EditorReady,
    /// Text content changed
    TextChange { text: String, tab_id: Option<String> },
    /// Editor focused (from WebView)
    EditorFocus,
    /// Completion request
    CompletionRequest {
        text: String,
        line: u32,
        column: u32,
    },
    /// Clipboard copy from editor
    ClipboardCopy { text: String },
    /// Clipboard paste request from editor
    ClipboardPaste,
    /// Execute query request (Cmd+Enter)
    ExecuteQuery { text: String },
    /// JavaScript error from the WebView
    JsError { message: String },
    /// Debug key event
    DebugKey {
        event: String,
        key: String,
        code: String,
    },
}

/// Suggestion sent to the Forge editor.
#[derive(Debug, Clone, Serialize)]
pub struct ForgeSuggestion {
    pub label: String,
    pub kind: String,
    pub insert_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

// ============================================================================
// ForgeWebView
// ============================================================================

/// WebView wrapper for the Forge editor with IPC communication.
pub struct ForgeWebView {
    webview: Entity<WebView>,
    /// Channel for receiving IPC messages from the editor
    message_rx: Arc<Mutex<Option<std::sync::mpsc::Receiver<IpcMessage>>>>,
}

impl ForgeWebView {
    /// Create a new ForgeWebView with bundled CodeMirror editor.
    pub fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        // Create channel for IPC messages
        let (tx, rx) = std::sync::mpsc::channel::<IpcMessage>();
        let tx = Arc::new(Mutex::new(tx));

        log::debug!("Creating ForgeWebView with bundled Forge editor (CM6)");

        // Create WebView with custom protocol to serve bundled files
        let builder = wry::WebViewBuilder::new()
            .with_asynchronous_custom_protocol("openmango".into(), move |_id, request, responder| {
                let path = request.uri().path();
                log::debug!("Custom protocol request: {}", path);

                let (content, content_type) = match path {
                    "/" | "/editor.html" => {
                        let data = EmbeddedAssets::get("forge/editor.html")
                            .map(|f| f.data.to_vec())
                            .unwrap_or_default();
                        (data, "text/html; charset=utf-8")
                    }
                    "/editor.css" => {
                        let data = EmbeddedAssets::get("forge/editor.css")
                            .map(|f| f.data.to_vec())
                            .unwrap_or_default();
                        (data, "text/css; charset=utf-8")
                    }
                    "/editor.js" => {
                        let data = EmbeddedAssets::get("forge/editor.js")
                            .map(|f| f.data.to_vec())
                            .unwrap_or_default();
                        (data, "application/javascript; charset=utf-8")
                    }
                    _ => {
                        let response = wry::http::Response::builder()
                            .status(404)
                            .body(b"Not Found".to_vec())
                            .unwrap();
                        responder.respond(response);
                        return;
                    }
                };

                let response = wry::http::Response::builder()
                    .status(200)
                    .header("Content-Type", content_type)
                    .header("Access-Control-Allow-Origin", "*")
                    .body(content)
                    .unwrap();
                responder.respond(response);
            })
            .with_url("openmango://localhost/editor.html")
            .with_initialization_script(
                r#"console.log('[ForgeEditor] WebView initialized');
                   window.onerror = function(msg, url, line, col, error) {
                       console.error('[ForgeEditor Error]', msg, 'at', url, ':', line);
                       if (window.ipc) {
                           window.ipc.postMessage(JSON.stringify({type: 'js_error', message: msg + ' at ' + url + ':' + line}));
                       }
                   };"#,
            )
            .with_ipc_handler({
                let tx = tx.clone();
                move |req| {
                    let body = req.body();
                    match serde_json::from_str::<IpcMessage>(body) {
                        Ok(msg) => {
                            if let Ok(tx) = tx.lock() {
                                let _ = tx.send(msg);
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to parse IPC message: {} - body: {}", e, body);
                        }
                    }
                }
            })
            .with_devtools(cfg!(debug_assertions))
            .with_focused(true)
            .with_accept_first_mouse(true);

        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        let webview = {
            use raw_window_handle::HasWindowHandle as _;
            let window_handle = window.window_handle().expect("No window handle");
            builder.build_as_child(&window_handle).unwrap()
        };

        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        let webview = {
            use gtk::prelude::*;
            use wry::WebViewBuilderExtUnix;
            let fixed = gtk::Fixed::builder().build();
            fixed.show_all();
            builder.build_gtk(&fixed).unwrap()
        };

        let webview_entity = cx.new(|cx| WebView::new(webview, window, cx));

        // Make sure the webview is visible
        webview_entity.update(cx, |wv, _| {
            wv.show();
        });

        log::debug!("ForgeWebView created successfully");

        cx.new(|_cx| Self {
            webview: webview_entity,
            message_rx: Arc::new(Mutex::new(Some(rx))),
        })
    }

    /// Take the message receiver (can only be called once).
    pub fn take_message_receiver(&self) -> Option<std::sync::mpsc::Receiver<IpcMessage>> {
        self.message_rx.lock().ok()?.take()
    }

    /// Send suggestions to the Forge editor.
    pub fn send_suggestions(&self, suggestions: Vec<ForgeSuggestion>, cx: &mut App) {
        let json = match serde_json::to_string(&suggestions) {
            Ok(j) => j,
            Err(e) => {
                log::error!("Failed to serialize suggestions: {}", e);
                return;
            }
        };

        self.webview.update(cx, |wv, _| {
            if let Err(e) = wv.evaluate_script(&format!(
                "if (window.receiveSuggestions) {{ window.receiveSuggestions({}); }}",
                json
            )) {
                log::warn!("Failed to send suggestions to editor: {}", e);
            }
        });
    }

    /// Switch the active tab in the editor and optionally set its content.
    pub fn set_active_tab(&self, tab_id: &str, content: &str, cx: &mut App) {
        let escaped = content.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        self.webview.update(cx, |wv, _| {
            if let Err(e) = wv.evaluate_script(&format!(
                "if (window.setActiveTab) {{ window.setActiveTab(\"{}\", \"{}\"); }}",
                tab_id, escaped
            )) {
                log::warn!("Failed to set active tab in editor: {}", e);
            }
        });
    }

    /// Flush editor content back to host state.
    pub fn flush_content(&self, tab_id: Option<&str>, cx: &mut App) {
        let script = if let Some(tab_id) = tab_id {
            format!(
                "if (window.flushContentToHost) {{ window.flushContentToHost(\"{}\"); }}",
                tab_id
            )
        } else {
            "if (window.flushContentToHost) { window.flushContentToHost(); }".to_string()
        };
        self.webview.update(cx, |wv, _| {
            if let Err(e) = wv.evaluate_script(&script) {
                log::warn!("Failed to flush editor content: {}", e);
            }
        });
    }

    /// Send query result to the Forge editor.
    #[allow(dead_code)]
    pub fn send_result(&self, result: &str, cx: &mut App) {
        let escaped = result.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        self.webview.update(cx, |wv, _| {
            if let Err(e) = wv.evaluate_script(&format!("window.receiveResult(\"{}\")", escaped)) {
                log::warn!("Failed to send result to editor: {}", e);
            }
        });
    }

    /// Set editor content.
    #[allow(dead_code)]
    pub fn set_content(&self, content: &str, cx: &mut App) {
        let escaped = content.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        self.webview.update(cx, |wv, _| {
            if let Err(e) = wv.evaluate_script(&format!(
                "if (window.setContent) {{ window.setContent(\"{}\"); }}",
                escaped
            )) {
                log::warn!("Failed to set editor content: {}", e);
            }
        });
    }

    /// Send pasted text to the editor.
    pub fn send_paste(&self, text: &str, cx: &mut App) {
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        self.webview.update(cx, |wv, _| {
            if let Err(e) = wv.evaluate_script(&format!(
                "if (window.receivePaste) {{ window.receivePaste(\"{}\"); }}",
                escaped
            )) {
                log::warn!("Failed to send paste text: {}", e);
            }
        });
    }

    /// Focus the editor.
    #[allow(dead_code)]
    pub fn focus_editor(&self, cx: &mut App) {
        self.webview.update(cx, |wv, _| {
            if let Err(e) = wv.evaluate_script("if (window.focusEditor) { window.focusEditor(); }") {
                log::warn!("Failed to focus editor: {}", e);
            }
        });
    }

    /// Focus the WebView itself (native focus).
    pub fn focus_webview(&self, cx: &mut App) {
        self.webview.update(cx, |wv, _| {
            if let Err(e) = wv.raw().focus() {
                log::warn!("Failed to focus WebView: {}", e);
            }
        });
    }

    /// Get the inner gpui-component WebView entity.
    pub fn inner_webview(&self) -> &Entity<WebView> {
        &self.webview
    }

}

impl Render for ForgeWebView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.webview.clone())
    }
}
