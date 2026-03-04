use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use uuid::Uuid;

use crate::ai::safety::{ConfirmationSender, OperationPreview, SafetyTier};

// ---------------------------------------------------------------------------
// Content blocks — structured rendering primitives
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    /// Rendered as TextView::markdown
    Markdown { text: String },
    /// Rendered as native datatable (from tool result or LLM fenced block)
    DataTable { json: String },
    /// Rendered as native chart
    Chart { chart_type: ChartType, json: String },
    /// Rendered as native stats card
    Stats { json: String },
    /// Shown as spinner during streaming (not serialized to disk)
    #[serde(skip)]
    Pending { block_type: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartType {
    Bar,
    Pie,
    Line,
}

const KNOWN_LANGS: &[&str] = &["datatable", "barchart", "piechart", "linechart", "stats"];

/// Parse markdown content into structured content blocks.
/// Splits at custom fenced code blocks (datatable, barchart, etc.).
/// Unclosed fenced blocks become `ContentBlock::Pending`.
pub fn parse_content_to_blocks(content: &str) -> Vec<ContentBlock> {
    let mut blocks: Vec<ContentBlock> = Vec::new();
    let mut md_buf = String::new();
    let mut in_block = false;
    let mut block_lang = String::new();
    let mut block_code = String::new();

    for line in content.lines() {
        if in_block {
            let trimmed = line.trim();
            if is_closing_fence(trimmed) {
                blocks.push(fenced_to_block(
                    &std::mem::take(&mut block_lang),
                    &std::mem::take(&mut block_code),
                ));
                in_block = false;
            } else if let Some(last_content) = strip_trailing_fence(trimmed) {
                if !block_code.is_empty() {
                    block_code.push('\n');
                }
                block_code.push_str(last_content);
                blocks.push(fenced_to_block(
                    &std::mem::take(&mut block_lang),
                    &std::mem::take(&mut block_code),
                ));
                in_block = false;
            } else {
                if !block_code.is_empty() {
                    block_code.push('\n');
                }
                block_code.push_str(line);
            }
        } else {
            let trimmed = line.trim();
            let normalized = normalize_fences(trimmed);
            if let Some(rest) = normalized.strip_prefix("```") {
                let lang = rest.trim();
                if KNOWN_LANGS.contains(&lang) {
                    let text = std::mem::take(&mut md_buf);
                    if !text.is_empty() {
                        blocks.push(ContentBlock::Markdown { text });
                    }
                    block_lang = lang.to_string();
                    block_code.clear();
                    in_block = true;
                } else {
                    if !md_buf.is_empty() {
                        md_buf.push('\n');
                    }
                    md_buf.push_str(line);
                }
            } else {
                if !md_buf.is_empty() {
                    md_buf.push('\n');
                }
                md_buf.push_str(line);
            }
        }
    }

    if in_block {
        let text = std::mem::take(&mut md_buf);
        if !text.is_empty() {
            blocks.push(ContentBlock::Markdown { text });
        }
        blocks.push(ContentBlock::Pending { block_type: block_lang });
    } else {
        let text = std::mem::take(&mut md_buf);
        if !text.is_empty() {
            blocks.push(ContentBlock::Markdown { text });
        }
    }

    blocks
}

/// Convert a fenced code block lang + code into the appropriate ContentBlock.
fn fenced_to_block(lang: &str, code: &str) -> ContentBlock {
    match lang {
        "datatable" => ContentBlock::DataTable { json: code.to_string() },
        "barchart" => ContentBlock::Chart { chart_type: ChartType::Bar, json: code.to_string() },
        "piechart" => ContentBlock::Chart { chart_type: ChartType::Pie, json: code.to_string() },
        "linechart" => ContentBlock::Chart { chart_type: ChartType::Line, json: code.to_string() },
        "stats" => ContentBlock::Stats { json: code.to_string() },
        _ => ContentBlock::Markdown { text: format!("```{lang}\n{code}\n```") },
    }
}

/// Map a tool result JSON string to a ContentBlock for native rendering.
pub fn tool_result_to_block(tool_name: &str, json: &str) -> Option<ContentBlock> {
    let val: serde_json::Value = serde_json::from_str(json).ok()?;
    match tool_name {
        "find_documents" | "aggregate" => {
            let docs = val.get("documents").or(val.get("results"))?;
            Some(ContentBlock::DataTable { json: docs.to_string() })
        }
        "list_indexes" => {
            let indexes = val.get("indexes")?;
            Some(ContentBlock::DataTable { json: indexes.to_string() })
        }
        "collection_stats" => {
            let obj = val.as_object()?;
            let metrics: Vec<serde_json::Value> = obj
                .iter()
                .map(|(k, v)| serde_json::json!({"label": k, "value": format_stat_value(v)}))
                .collect();
            let stats = serde_json::json!({"title": "Collection Stats", "metrics": metrics});
            Some(ContentBlock::Stats { json: stats.to_string() })
        }
        "count_documents" => {
            let count_val = val.get("count").map(|v| v.to_string()).unwrap_or_default();
            let metrics = vec![serde_json::json!({"label": "Count", "value": count_val})];
            let stats = serde_json::json!({"title": "Document Count", "metrics": metrics});
            Some(ContentBlock::Stats { json: stats.to_string() })
        }
        "insert_documents" => {
            let n = val.get("inserted_count").map(|v| v.to_string()).unwrap_or_default();
            let metrics = vec![serde_json::json!({"label": "Inserted", "value": n})];
            let stats = serde_json::json!({"title": "Insert Result", "metrics": metrics});
            Some(ContentBlock::Stats { json: stats.to_string() })
        }
        "update_documents" => {
            let matched = val.get("matched_count").map(|v| v.to_string()).unwrap_or_default();
            let modified = val.get("modified_count").map(|v| v.to_string()).unwrap_or_default();
            let metrics = vec![
                serde_json::json!({"label": "Matched", "value": matched}),
                serde_json::json!({"label": "Modified", "value": modified}),
            ];
            let stats = serde_json::json!({"title": "Update Result", "metrics": metrics});
            Some(ContentBlock::Stats { json: stats.to_string() })
        }
        "delete_documents" => {
            let n = val.get("deleted_count").map(|v| v.to_string()).unwrap_or_default();
            let metrics = vec![serde_json::json!({"label": "Deleted", "value": n})];
            let stats = serde_json::json!({"title": "Delete Result", "metrics": metrics});
            Some(ContentBlock::Stats { json: stats.to_string() })
        }
        "create_index" => {
            let name = val.get("index_name").and_then(|v| v.as_str()).unwrap_or("(unknown)");
            let metrics = vec![serde_json::json!({"label": "Index Created", "value": name})];
            let stats = serde_json::json!({"title": "Create Index Result", "metrics": metrics});
            Some(ContentBlock::Stats { json: stats.to_string() })
        }
        "drop_index" => {
            let name = val.get("dropped").and_then(|v| v.as_str()).unwrap_or("(unknown)");
            let metrics = vec![serde_json::json!({"label": "Index Dropped", "value": name})];
            let stats = serde_json::json!({"title": "Drop Index Result", "metrics": metrics});
            Some(ContentBlock::Stats { json: stats.to_string() })
        }
        _ => None,
    }
}

fn format_stat_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    return format!("{}", f as i64);
                }
                return format!("{:.2}", f);
            }
            n.to_string()
        }
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

/// Normalize escaped backticks: `\`` → `` ` ``.
fn normalize_fences(s: &str) -> String {
    s.replace("\\`", "`")
}

/// Check if `trimmed` is a closing fence.
fn is_closing_fence(trimmed: &str) -> bool {
    trimmed == "```" || trimmed == "\\`\\`\\`"
}

/// Check if `trimmed` ends with a closing fence.
fn strip_trailing_fence(trimmed: &str) -> Option<&str> {
    if trimmed.len() > 3 {
        if let Some(rest) = trimmed.strip_suffix("```") {
            return Some(rest);
        }
        if let Some(rest) = trimmed.strip_suffix("\\`\\`\\`") {
            return Some(rest);
        }
    }
    None
}

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
    /// Raw markdown content (kept for LLM history / backward compat)
    pub content: String,
    /// Structured blocks for rendering. Derived from content + tool results.
    /// If empty (e.g. old workspace data), falls back to parsing `content`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<ContentBlock>,
    pub created_at: DateTime<Utc>,
}

impl ChatMessage {
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        let content = content.into();
        let blocks =
            if content.is_empty() { Vec::new() } else { parse_content_to_blocks(&content) };
        Self { id: Uuid::new_v4(), role, content, blocks, created_at: Utc::now() }
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
    #[serde(skip)]
    AwaitingConfirmation {
        description: String,
        tier: SafetyTier,
        preview: OperationPreview,
        response_tx: ConfirmationSender,
    },
    Completed,
    Failed(String),
    #[serde(skip)]
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolActivity {
    pub id: Uuid,
    pub tool_name: String,
    pub status: ToolActivityStatus,
    pub args_preview: String,
    pub result_preview: Option<String>,
    /// Full structured result for native rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_block: Option<ContentBlock>,
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
    #[serde(skip)]
    pub cached_models: crate::ai::model_registry::ModelCache,
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
            msg.blocks = parse_content_to_blocks(&msg.content);
        }
    }

    pub fn finalize_turn_response(&mut self, message_id: Uuid, final_content: String) {
        if let Some(turn) = self.current_turn_mut()
            && let Some(msg) = &mut turn.assistant_message
            && msg.id == message_id
        {
            msg.blocks = parse_content_to_blocks(&final_content);
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
            result_block: None,
        }));
        self.trim_entries();
        id
    }

    pub fn complete_tool(
        &mut self,
        name: &str,
        result_preview: String,
        result_json: Option<String>,
    ) {
        // Find the most recent Running tool activity with matching name
        let block = result_json.as_deref().and_then(|json| tool_result_to_block(name, json));
        for entry in self.entries.iter_mut().rev() {
            if let AiChatEntry::ToolActivity(activity) = entry
                && activity.tool_name == name
                && matches!(activity.status, ToolActivityStatus::Running)
            {
                activity.status = ToolActivityStatus::Completed;
                activity.result_preview = Some(result_preview);
                activity.result_block = block;
                return;
            }
        }
    }

    /// Transition a tool from AwaitingConfirmation back to Running (approved).
    pub fn approve_tool_confirmation(&mut self, activity_id: Uuid) {
        for entry in self.entries.iter_mut().rev() {
            if let AiChatEntry::ToolActivity(activity) = entry
                && activity.id == activity_id
                && matches!(activity.status, ToolActivityStatus::AwaitingConfirmation { .. })
            {
                activity.status = ToolActivityStatus::Running;
                return;
            }
        }
    }

    /// Transition a tool from AwaitingConfirmation to Rejected.
    pub fn reject_tool_confirmation(&mut self, activity_id: Uuid) {
        for entry in self.entries.iter_mut().rev() {
            if let AiChatEntry::ToolActivity(activity) = entry
                && activity.id == activity_id
                && matches!(activity.status, ToolActivityStatus::AwaitingConfirmation { .. })
            {
                activity.status = ToolActivityStatus::Rejected;
                return;
            }
        }
    }

    pub fn set_tool_awaiting_confirmation(
        &mut self,
        tool_name: &str,
        description: String,
        tier: SafetyTier,
        preview: OperationPreview,
        response_tx: ConfirmationSender,
    ) {
        for entry in self.entries.iter_mut().rev() {
            if let AiChatEntry::ToolActivity(activity) = entry
                && activity.tool_name == tool_name
                && matches!(activity.status, ToolActivityStatus::Running)
            {
                activity.status = ToolActivityStatus::AwaitingConfirmation {
                    description,
                    tier,
                    preview,
                    response_tx,
                };
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
