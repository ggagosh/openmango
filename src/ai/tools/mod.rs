pub mod aggregate;
pub mod collection_stats;
pub mod count;
pub mod explain;
pub mod find;
pub mod indexes;
pub mod list_collections;
pub mod schema;

use mongodb::bson;
use rig::tool::ToolDyn;

/// Shared context passed to all tools at construction time.
#[derive(Clone)]
pub struct MongoContext {
    pub client: mongodb::Client,
    pub database: String,
    pub collection: Option<String>,
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
}

/// Stream events emitted by the provider during generation.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta(String),
    ToolCallStart { name: String, args_preview: String },
    ToolCallEnd { name: String, result_preview: String },
}

/// Build all available MongoDB tools for the given context.
pub fn build_tools(ctx: MongoContext) -> Vec<Box<dyn ToolDyn>> {
    vec![
        Box::new(find::FindDocumentsTool::new(ctx.clone())),
        Box::new(aggregate::AggregateTool::new(ctx.clone())),
        Box::new(count::CountDocumentsTool::new(ctx.clone())),
        Box::new(list_collections::ListCollectionsTool::new(ctx.clone())),
        Box::new(collection_stats::CollectionStatsTool::new(ctx.clone())),
        Box::new(schema::CollectionSchemaTool::new(ctx.clone())),
        Box::new(indexes::ListIndexesTool::new(ctx.clone())),
        Box::new(explain::ExplainQueryTool::new(ctx)),
    ]
}

/// Truncate a JSON value's serialized form to `max_bytes`.
pub fn truncate_output(value: serde_json::Value, max_bytes: usize) -> serde_json::Value {
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() <= max_bytes {
        return value;
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
const MAX_FIND_LIMIT: i64 = 50;
