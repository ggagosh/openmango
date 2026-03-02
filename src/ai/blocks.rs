use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

impl ChatRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "You",
            Self::Assistant => "Assistant",
            Self::System => "System",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: Uuid,
    pub role: ChatRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

impl ChatMessage {
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self { id: Uuid::new_v4(), role, content: content.into(), created_at: Utc::now() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiTurn {
    pub id: Uuid,
    pub user_message: ChatMessage,
    pub assistant_message: Option<ChatMessage>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolActivityStatus {
    Running,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolActivity {
    pub id: Uuid,
    pub tool_name: String,
    pub status: ToolActivityStatus,
    pub args_preview: String,
    pub result_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AiChatEntry {
    Turn(AiTurn),
    ToolActivity(ToolActivity),
    SystemMessage(ChatMessage),
    // Legacy variant for backwards-compatible deserialization of old workspaces.
    #[serde(alias = "Message")]
    LegacyMessage(ChatMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiChatState {
    pub panel_open: bool,
    pub is_loading: bool,
    pub draft_input: String,
    #[serde(default)]
    pub entries: Vec<AiChatEntry>,
    pub last_error: Option<String>,
    #[serde(skip)]
    pub current_turn_id: Option<Uuid>,
    #[serde(skip)]
    pub cancel_flag: Option<Arc<AtomicBool>>,
}

impl AiChatState {
    const TIMELINE_LIMIT: usize = 200;

    pub fn begin_turn(&mut self, content: impl Into<String>) -> Uuid {
        let user_message = ChatMessage::new(ChatRole::User, content);
        let turn_id = Uuid::new_v4();
        let turn =
            AiTurn { id: turn_id, user_message, assistant_message: None, created_at: Utc::now() };
        self.entries.push(AiChatEntry::Turn(turn));
        self.current_turn_id = Some(turn_id);
        self.trim_entries();
        self.clear_error();
        turn_id
    }

    pub fn push_system_message(&mut self, content: impl Into<String>) {
        self.entries.push(AiChatEntry::SystemMessage(ChatMessage::new(ChatRole::System, content)));
        self.trim_entries();
    }

    pub fn set_turn_assistant_message(&mut self, turn_id: Uuid, content: String) {
        if let Some(turn) = self.find_turn_mut(turn_id) {
            match &mut turn.assistant_message {
                Some(msg) => msg.content = content,
                None => {
                    turn.assistant_message = Some(ChatMessage::new(ChatRole::Assistant, content));
                }
            }
        }
    }

    pub fn begin_turn_streaming_response(&mut self) -> Option<Uuid> {
        let turn = self.current_turn_mut()?;
        let msg = ChatMessage::new(ChatRole::Assistant, String::new());
        let msg_id = msg.id;
        turn.assistant_message = Some(msg);
        Some(msg_id)
    }

    pub fn append_turn_delta(&mut self, message_id: Uuid, delta: &str) {
        if delta.is_empty() {
            return;
        }
        if let Some(turn) = self.current_turn_mut()
            && let Some(msg) = &mut turn.assistant_message
            && msg.id == message_id
        {
            msg.content.push_str(delta);
        }
    }

    pub fn finalize_turn_response(&mut self, message_id: Uuid, final_content: String) {
        if let Some(turn) = self.current_turn_mut()
            && let Some(msg) = &mut turn.assistant_message
            && msg.id == message_id
        {
            msg.content = final_content;
        }
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    pub fn find_turn_mut(&mut self, turn_id: Uuid) -> Option<&mut AiTurn> {
        self.entries.iter_mut().find_map(|entry| match entry {
            AiChatEntry::Turn(turn) if turn.id == turn_id => Some(turn),
            _ => None,
        })
    }

    pub fn current_turn_mut(&mut self) -> Option<&mut AiTurn> {
        let turn_id = self.current_turn_id?;
        self.find_turn_mut(turn_id)
    }

    pub fn messages(&self) -> Vec<ChatMessage> {
        let mut msgs = Vec::new();
        for entry in &self.entries {
            match entry {
                AiChatEntry::Turn(turn) => {
                    msgs.push(turn.user_message.clone());
                    if let Some(assistant_msg) = &turn.assistant_message {
                        msgs.push(assistant_msg.clone());
                    }
                }
                AiChatEntry::SystemMessage(msg) | AiChatEntry::LegacyMessage(msg) => {
                    msgs.push(msg.clone());
                }
                AiChatEntry::ToolActivity(_) => {}
            }
        }
        msgs
    }

    pub fn last_user_prompt(&self) -> Option<String> {
        self.entries.iter().rev().find_map(|entry| match entry {
            AiChatEntry::Turn(turn) => Some(turn.user_message.content.clone()),
            _ => None,
        })
    }

    pub fn push_tool_start(&mut self, name: String, args: String) -> Uuid {
        let id = Uuid::new_v4();
        self.entries.push(AiChatEntry::ToolActivity(ToolActivity {
            id,
            tool_name: name,
            status: ToolActivityStatus::Running,
            args_preview: args,
            result_preview: None,
        }));
        self.trim_entries();
        id
    }

    pub fn complete_tool(&mut self, name: &str, result: String) {
        // Find the most recent Running tool activity with matching name
        for entry in self.entries.iter_mut().rev() {
            if let AiChatEntry::ToolActivity(activity) = entry
                && activity.tool_name == name
                && matches!(activity.status, ToolActivityStatus::Running)
            {
                activity.status = ToolActivityStatus::Completed;
                activity.result_preview = Some(result);
                return;
            }
        }
    }

    pub fn trim_entries(&mut self) {
        if self.entries.len() > Self::TIMELINE_LIMIT {
            let extra = self.entries.len().saturating_sub(Self::TIMELINE_LIMIT);
            self.entries.drain(0..extra);
        }
    }
}
