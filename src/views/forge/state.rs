use gpui::{FocusHandle, UniformListScrollHandle};
use gpui_component::input::InputState;

use super::types::{ForgeOutputTab, ForgeRunOutput, ResultPage};

pub struct ForgeEditorState {
    pub editor_state: Option<gpui::Entity<InputState>>,
    pub editor_subscription: Option<gpui::Subscription>,
    pub completion_provider: Option<std::rc::Rc<super::completion::ForgeCompletionProvider>>,
    pub current_text: String,
    pub editor_focus_requested: bool,
    pub active_tab_id: Option<uuid::Uuid>,
}

pub struct ForgeRuntimeState {
    pub run_seq: u64,
    pub is_running: bool,
    pub mongosh_error: Option<String>,
}

pub struct ForgeOutputState {
    pub raw_output_state: Option<gpui::Entity<InputState>>,
    pub raw_output_subscription: Option<gpui::Subscription>,
    pub raw_output_text: String,
    pub raw_output_programmatic: bool,
    pub results_search_state: Option<gpui::Entity<InputState>>,
    pub results_search_subscription: Option<gpui::Subscription>,
    pub results_search_query: String,
    pub output_runs: Vec<ForgeRunOutput>,
    pub output_tab: ForgeOutputTab,
    pub active_run_id: Option<u64>,
    pub output_events_started: bool,
    pub last_result: Option<String>,
    pub last_error: Option<String>,
    pub result_pages: Vec<ResultPage>,
    pub result_page_index: usize,
    pub result_signature: Option<u64>,
    pub result_expanded_nodes: std::collections::HashSet<String>,
    pub result_scroll: UniformListScrollHandle,
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
                editor_subscription: None,
                completion_provider: None,
                current_text: String::new(),
                editor_focus_requested: false,
                active_tab_id: None,
            },
            output: ForgeOutputState {
                raw_output_state: None,
                raw_output_subscription: None,
                raw_output_text: String::new(),
                raw_output_programmatic: false,
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
                result_signature: None,
                result_expanded_nodes: std::collections::HashSet::new(),
                result_scroll: UniformListScrollHandle::new(),
                output_visible: true,
            },
            runtime: ForgeRuntimeState { run_seq: 0, is_running: false, mongosh_error: None },
            focus_handle,
        }
    }
}
