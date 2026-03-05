//! Safety classification for AI write tools.
//!
//! Classifies tool calls into tiers that determine whether confirmation is
//! required before executing a mutation against MongoDB.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// How dangerous is a tool call?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafetyTier {
    /// Read tools — run immediately with no confirmation.
    AutoExecute,
    /// Insert, create_index — show preview card, require confirmation.
    ConfirmFirst,
    /// Update, delete, drop_index — red warning card, require confirmation.
    AlwaysConfirm,
    /// Destructive patterns (empty filter delete, $where) — blocked outright.
    Blocked,
}

/// Result of classifying a single tool invocation.
pub struct SafetyClassification {
    pub tier: SafetyTier,
    pub description: String,
    pub reason: Option<String>,
}

/// Clone + Send + Sync wrapper around a take-once oneshot sender.
#[derive(Debug, Clone)]
pub struct ConfirmationSender(Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<bool>>>>);

impl ConfirmationSender {
    pub fn new(tx: tokio::sync::oneshot::Sender<bool>) -> Self {
        Self(Arc::new(std::sync::Mutex::new(Some(tx))))
    }

    /// Send a response. Idempotent — subsequent calls are no-ops.
    pub fn respond(&self, approved: bool) {
        if let Ok(mut guard) = self.0.lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send(approved);
        }
    }
}

/// Preview data shown in the confirmation card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationPreview {
    pub collection: String,
    pub affected_count: u64,
    pub sample_docs: Vec<serde_json::Value>,
    /// Safety reason shown when the operation is blocked but overridable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Read-only tool names (auto-execute)
// ---------------------------------------------------------------------------

const AUTO_EXECUTE_TOOLS: &[&str] = &[
    "find_documents",
    "aggregate",
    "count_documents",
    "list_collections",
    "collection_stats",
    "collection_schema",
    "list_indexes",
    "explain_query",
];

const CONFIRM_FIRST_TOOLS: &[&str] = &["insert_documents", "create_index"];
const ALWAYS_CONFIRM_TOOLS: &[&str] = &["update_documents", "delete_documents", "drop_index"];

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Classify a tool call by name and argument JSON.
pub fn classify_tool_call(tool_name: &str, args_json: &str) -> SafetyClassification {
    if AUTO_EXECUTE_TOOLS.contains(&tool_name) {
        return SafetyClassification {
            tier: SafetyTier::AutoExecute,
            description: format!("{tool_name} (read-only)"),
            reason: None,
        };
    }

    // Check for blocked patterns on update/delete before tier assignment
    if (tool_name == "update_documents" || tool_name == "delete_documents")
        && let Some(blocked) = check_blocked_patterns(tool_name, args_json)
    {
        return blocked;
    }

    if CONFIRM_FIRST_TOOLS.contains(&tool_name) {
        let desc = match tool_name {
            "insert_documents" => "Insert documents",
            "create_index" => "Create index",
            _ => tool_name,
        };
        return SafetyClassification {
            tier: SafetyTier::ConfirmFirst,
            description: desc.to_string(),
            reason: None,
        };
    }

    if ALWAYS_CONFIRM_TOOLS.contains(&tool_name) {
        let desc = match tool_name {
            "update_documents" => "Update documents",
            "delete_documents" => "Delete documents",
            "drop_index" => "Drop index",
            _ => tool_name,
        };
        return SafetyClassification {
            tier: SafetyTier::AlwaysConfirm,
            description: desc.to_string(),
            reason: None,
        };
    }

    // Unknown tool — blocked
    SafetyClassification {
        tier: SafetyTier::Blocked,
        description: format!("Unknown tool: {tool_name}"),
        reason: Some(format!("Tool '{tool_name}' is not recognized")),
    }
}

/// Check for dangerous patterns in update/delete args.
fn check_blocked_patterns(tool_name: &str, args_json: &str) -> Option<SafetyClassification> {
    let args: serde_json::Value = serde_json::from_str(args_json).ok()?;

    let filter = args.get("filter");

    // Missing filter
    if filter.is_none() {
        return Some(SafetyClassification {
            tier: SafetyTier::Blocked,
            description: format!("{tool_name} without filter"),
            reason: Some("A filter is required for update/delete operations".to_string()),
        });
    }

    let filter_str = match filter {
        Some(serde_json::Value::String(s)) => s.as_str(),
        _ => return None,
    };

    // Empty filter "{}" or whitespace-only
    let trimmed = filter_str.trim();
    if trimmed == "{}" || trimmed.is_empty() {
        return Some(SafetyClassification {
            tier: SafetyTier::Blocked,
            description: format!("{tool_name} with empty filter"),
            reason: Some(
                "Empty filter would affect all documents. Use a specific filter.".to_string(),
            ),
        });
    }

    // $where clause
    if trimmed.contains("$where") {
        return Some(SafetyClassification {
            tier: SafetyTier::Blocked,
            description: format!("{tool_name} with $where"),
            reason: Some("$where clauses are blocked for safety".to_string()),
        });
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_tools_are_auto_execute() {
        for name in AUTO_EXECUTE_TOOLS {
            let c = classify_tool_call(name, "{}");
            assert_eq!(c.tier, SafetyTier::AutoExecute, "expected AutoExecute for {name}");
        }
    }

    #[test]
    fn insert_is_confirm_first() {
        let c = classify_tool_call("insert_documents", r#"{"documents": "[]"}"#);
        assert_eq!(c.tier, SafetyTier::ConfirmFirst);
    }

    #[test]
    fn create_index_is_confirm_first() {
        let c = classify_tool_call("create_index", r#"{"keys": "{\"name\": 1}"}"#);
        assert_eq!(c.tier, SafetyTier::ConfirmFirst);
    }

    #[test]
    fn update_is_always_confirm() {
        let c = classify_tool_call(
            "update_documents",
            r#"{"filter": "{\"status\": \"active\"}", "update": "{\"$set\": {\"x\": 1}}"}"#,
        );
        assert_eq!(c.tier, SafetyTier::AlwaysConfirm);
    }

    #[test]
    fn delete_is_always_confirm() {
        let c =
            classify_tool_call("delete_documents", r#"{"filter": "{\"status\": \"inactive\"}"}"#);
        assert_eq!(c.tier, SafetyTier::AlwaysConfirm);
    }

    #[test]
    fn drop_index_is_always_confirm() {
        let c = classify_tool_call("drop_index", r#"{"index_name": "my_idx"}"#);
        assert_eq!(c.tier, SafetyTier::AlwaysConfirm);
    }

    #[test]
    fn unknown_tool_is_blocked() {
        let c = classify_tool_call("drop_database", "{}");
        assert_eq!(c.tier, SafetyTier::Blocked);
    }

    #[test]
    fn empty_filter_delete_is_blocked() {
        let c = classify_tool_call("delete_documents", r#"{"filter": "{}"}"#);
        assert_eq!(c.tier, SafetyTier::Blocked);
    }

    #[test]
    fn empty_filter_update_is_blocked() {
        let c = classify_tool_call("update_documents", r#"{"filter": "{}"}"#);
        assert_eq!(c.tier, SafetyTier::Blocked);
    }

    #[test]
    fn missing_filter_delete_is_blocked() {
        let c = classify_tool_call("delete_documents", r#"{}"#);
        assert_eq!(c.tier, SafetyTier::Blocked);
    }

    #[test]
    fn where_clause_is_blocked() {
        let c =
            classify_tool_call("delete_documents", r#"{"filter": "{\"$where\": \"this.x > 0\"}"}"#);
        assert_eq!(c.tier, SafetyTier::Blocked);
    }

    #[test]
    fn confirmation_sender_is_idempotent() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let sender = ConfirmationSender::new(tx);
        sender.respond(true);
        sender.respond(false); // no-op
        assert!(rx.blocking_recv().unwrap());
    }
}
