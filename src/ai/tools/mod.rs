pub mod aggregate;
pub mod collection_stats;
pub mod count;
pub mod create_index;
pub mod delete;
pub mod explain;
pub mod find;
pub mod indexes;
pub mod insert;
pub mod list_collections;
pub mod schema;
pub mod update;

use mongodb::bson;
use rig::tool::ToolDyn;

use crate::ai::safety::{ConfirmationSender, OperationPreview, SafetyTier, classify_tool_call};

/// Shared context passed to all tools at construction time.
#[derive(Clone)]
pub struct MongoContext {
    pub client: mongodb::Client,
    pub database: String,
    pub collection: Option<String>,
    pub event_tx: Option<tokio::sync::mpsc::UnboundedSender<StreamEvent>>,
}

/// Errors that tools can return — rig converts these into text for the LLM.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("MongoDB error: {0}")]
    Mongo(#[from] mongodb::error::Error),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Rejected(String),
}

/// Stream events emitted by the provider during generation.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta(String),
    ToolCallStart {
        name: String,
        args_preview: String,
        args_full: String,
    },
    ToolCallEnd {
        name: String,
        result_preview: String,
        result_json: Option<String>,
    },
    ConfirmationRequired {
        tool_name: String,
        description: String,
        tier: SafetyTier,
        preview: OperationPreview,
        response_tx: ConfirmationSender,
    },
}

/// Build all available MongoDB tools for the given context.
pub fn build_tools(ctx: MongoContext) -> Vec<Box<dyn ToolDyn>> {
    vec![
        // Read tools
        Box::new(find::FindDocumentsTool::new(ctx.clone())),
        Box::new(aggregate::AggregateTool::new(ctx.clone())),
        Box::new(count::CountDocumentsTool::new(ctx.clone())),
        Box::new(list_collections::ListCollectionsTool::new(ctx.clone())),
        Box::new(collection_stats::CollectionStatsTool::new(ctx.clone())),
        Box::new(schema::CollectionSchemaTool::new(ctx.clone())),
        Box::new(indexes::ListIndexesTool::new(ctx.clone())),
        Box::new(explain::ExplainQueryTool::new(ctx.clone())),
        // Write tools
        Box::new(insert::InsertDocumentsTool::new(ctx.clone())),
        Box::new(update::UpdateDocumentsTool::new(ctx.clone())),
        Box::new(delete::DeleteDocumentsTool::new(ctx.clone())),
        Box::new(create_index::CreateIndexTool::new(ctx.clone())),
        Box::new(self::drop_index::DropIndexTool::new(ctx)),
    ]
}

/// Request user confirmation for a write operation via the event channel.
///
/// Returns `Ok(())` if the operation should proceed, or an appropriate error
/// if it was blocked or rejected.
pub async fn require_confirmation(
    ctx: &MongoContext,
    tool_name: &str,
    args_json: &str,
    preview: OperationPreview,
) -> Result<(), ToolError> {
    let classification = classify_tool_call(tool_name, args_json);
    match classification.tier {
        SafetyTier::Blocked => Err(ToolError::InvalidInput(
            classification.reason.unwrap_or_else(|| "Operation blocked".to_string()),
        )),
        SafetyTier::AutoExecute => Ok(()),
        SafetyTier::ConfirmFirst | SafetyTier::AlwaysConfirm => {
            if let Some(tx) = &ctx.event_tx {
                let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                let _ = tx.send(StreamEvent::ConfirmationRequired {
                    tool_name: tool_name.to_string(),
                    description: classification.description,
                    tier: classification.tier,
                    preview,
                    response_tx: ConfirmationSender::new(resp_tx),
                });
                match resp_rx.await {
                    Ok(true) => Ok(()),
                    Ok(false) => Err(ToolError::Rejected("User rejected the operation".into())),
                    Err(_) => Err(ToolError::Rejected("Operation cancelled".into())),
                }
            } else {
                // No event channel (non-streaming path) — skip confirmation
                Ok(())
            }
        }
    }
}

/// Truncate a JSON value's serialized form to `max_bytes`.
///
/// For objects containing a large array (e.g. `"documents"`, `"results"`),
/// elements are removed from the end so the output stays valid JSON.
/// Falls back to raw string truncation only for non-object values.
pub fn truncate_output(value: serde_json::Value, max_bytes: usize) -> serde_json::Value {
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() <= max_bytes {
        return value;
    }

    // For objects with a known array key, drop elements to fit.
    if let serde_json::Value::Object(mut map) = value {
        for key in ["documents", "results", "indexes"] {
            if let Some(serde_json::Value::Array(arr)) = map.remove(key) {
                let total = arr.len();
                // Binary search: find max element count that fits.
                let (mut lo, mut hi) = (0usize, arr.len());
                while lo < hi {
                    let mid = (lo + hi).div_ceil(2);
                    let mut candidate = map.clone();
                    candidate
                        .insert(key.to_string(), serde_json::Value::Array(arr[..mid].to_vec()));
                    let len = serde_json::to_string(&candidate).map_or(usize::MAX, |s| s.len());
                    if len <= max_bytes {
                        lo = mid;
                    } else {
                        hi = mid - 1;
                    }
                }
                map.insert(key.to_string(), serde_json::Value::Array(arr[..lo].to_vec()));
                if lo < total {
                    map.insert(
                        "truncated_from".to_string(),
                        serde_json::Value::Number(total.into()),
                    );
                }
                return serde_json::Value::Object(map);
            }
        }
        // No known array key — fall through to string truncation.
        let serialized = serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or_default();
        let end = serialized.floor_char_boundary(max_bytes.saturating_sub(40));
        return serde_json::Value::String(format!(
            "{}... [truncated, {} bytes total]",
            &serialized[..end],
            serialized.len()
        ));
    }

    let end = serialized.floor_char_boundary(max_bytes.saturating_sub(40));
    serde_json::Value::String(format!(
        "{}... [truncated, {} bytes total]",
        &serialized[..end],
        serialized.len()
    ))
}

/// Truncate a string to at most `max` bytes on a char boundary.
pub fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let end = s.floor_char_boundary(max);
    &s[..end]
}

/// Resolve a collection name from the tool args or fall back to context default.
pub fn resolve_collection(arg: &Option<String>, ctx: &MongoContext) -> Result<String, ToolError> {
    arg.as_deref()
        .or(ctx.collection.as_deref())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| {
            ToolError::InvalidInput(
                "No collection specified and no default collection in context".to_string(),
            )
        })
}

/// Parse a JSON string into a BSON Document.
pub fn parse_json_to_doc(json_str: &str) -> Result<bson::Document, ToolError> {
    let value: serde_json::Value = serde_json::from_str(json_str)?;
    let bson_val = bson::to_bson(&value).map_err(|e| ToolError::InvalidInput(e.to_string()))?;
    match bson_val {
        bson::Bson::Document(doc) => Ok(doc),
        _ => Err(ToolError::InvalidInput("Expected a JSON object".to_string())),
    }
}

/// Convert a BSON document to a relaxed JSON value.
pub fn doc_to_json(doc: &bson::Document) -> serde_json::Value {
    // Use Bson's extended JSON serialization for clean output
    let bson_val = bson::Bson::Document(doc.clone());
    serde_json::to_value(bson_val).unwrap_or(serde_json::Value::Null)
}

const MAX_OUTPUT_BYTES: usize = 32 * 1024;
const MAX_FIND_LIMIT: i64 = 10;

pub mod drop_index;
