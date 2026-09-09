use gpui_kit::FocusHandle;
use gpui_kit::component::input::{EditorState, InputState};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use super::types::{ForgeOutputTab, ForgeRunOutput, ResultPage};
use crate::helpers::auto_pair::AutoPairState;

pub struct ForgeEditorState {
    pub editor_state: Option<gpui_kit::Entity<EditorState>>,
    pub buffers: HashMap<uuid::Uuid, ForgeEditorBuffer>,
    pub completion_request_id: Arc<AtomicU64>,
    pub editor_focus_requested: bool,
    pub active_tab_id: Option<uuid::Uuid>,
}

pub struct ForgeEditorBuffer {
    pub editor_state: gpui_kit::Entity<EditorState>,
    pub completion_provider: std::rc::Rc<super::completion::ForgeCompletionProvider>,
    pub completion_menu: gpui_kit::Entity<super::completion_menu::ForgeCompletionMenu>,
    pub _subscription: gpui_kit::Subscription,
    pub content: String,
    pub auto_pair: AutoPairState,
}

pub struct ForgeRuntimeState {
    pub run_seq: u64,
    pub is_running: bool,
    pub mongosh_error: Option<String>,
}

pub struct ForgeOutputState {
    pub raw: super::output::RawOutputState,
    pub results_search_state: Option<gpui_kit::Entity<InputState>>,
    pub results_search_subscription: Option<gpui_kit::Subscription>,
    pub results_search_query: String,
    pub output_runs: Vec<ForgeRunOutput>,
    pub output_tab: ForgeOutputTab,
    pub active_run_id: Option<u64>,
    pub output_events_started: bool,
    pub last_result: Option<String>,
    pub last_error: Option<String>,
    pub result_pages: Vec<ResultPage>,
    pub result_page_index: usize,
    pub auto_select_results: bool,
    pub trimmed_output_lines: usize,
    pub skipped_output_events: u64,
    pub output_visible: bool,
}

pub struct ForgeState {
    pub editor: ForgeEditorState,
    pub output: ForgeOutputState,
    pub runtime: ForgeRuntimeState,
    pub focus_handle: FocusHandle,
}

impl ForgeState {
    pub fn new(focus_handle: FocusHandle) -> Self {
        Self {
            editor: ForgeEditorState {
                editor_state: None,
                buffers: HashMap::new(),
                completion_request_id: Arc::new(AtomicU64::new(0)),
                editor_focus_requested: false,
                active_tab_id: None,
            },
            output: ForgeOutputState {
                raw: super::output::RawOutputState::default(),
                results_search_state: None,
                results_search_subscription: None,
                results_search_query: String::new(),
                output_runs: Vec::new(),
                output_tab: ForgeOutputTab::Raw,
                active_run_id: None,
                output_events_started: false,
                last_result: None,
                last_error: None,
                result_pages: Vec::new(),
                result_page_index: 0,
                auto_select_results: true,
                trimmed_output_lines: 0,
                skipped_output_events: 0,
                output_visible: true,
            },
            runtime: ForgeRuntimeState { run_seq: 0, is_running: false, mongosh_error: None },
            focus_handle,
        }
    }
}
