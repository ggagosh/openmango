pub mod aggregate;
pub mod collection_stats;
pub mod count;
pub mod create_index;
pub mod delete;
pub mod explain;
pub mod find;
pub mod generate_report;
pub mod indexes;
pub mod insert;
pub mod list_collections;
pub mod replace;
pub mod sample_values;
pub mod schema;

use mongodb::bson;
use rig::tool::ToolDyn;

use crate::ai::safety::{ConfirmationSender, OperationPreview, SafetyTier, classify_tool_call};
use crate::models::ConnectionWriteIdentity;

/// Shared context passed to all tools at construction time.
#[derive(Clone)]
pub struct MongoContext {
    pub client: mongodb::Client,
    pub database: String,
    pub collection: Option<String>,
    pub write_identity: ConnectionWriteIdentity,
    pub read_only: bool,
    pub operation_engine: Option<std::sync::Arc<crate::operations::OperationEngine>>,
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
    #[error("Reversible history error: {0}")]
    History(String),
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
    DocumentsChanged {
        connection_id: uuid::Uuid,
        database: String,
        collection: String,
    },
    ConfirmationRequired {
        tool_name: String,
        description: String,
        tier: SafetyTier,
        preview: OperationPreview,
        write_identity: ConnectionWriteIdentity,
        response_tx: ConfirmationSender,
    },
}

/// Build all available MongoDB tools for the given context.
pub fn build_tools(ctx: MongoContext) -> Vec<Box<dyn ToolDyn>> {
    let mut tools: Vec<Box<dyn ToolDyn>> = vec![
        Box::new(find::FindDocumentsTool::new(ctx.clone())),
        Box::new(aggregate::AggregateTool::new(ctx.clone())),
        Box::new(count::CountDocumentsTool::new(ctx.clone())),
        Box::new(list_collections::ListCollectionsTool::new(ctx.clone())),
        Box::new(collection_stats::CollectionStatsTool::new(ctx.clone())),
        Box::new(schema::CollectionSchemaTool::new(ctx.clone())),
        Box::new(indexes::ListIndexesTool::new(ctx.clone())),
        Box::new(explain::ExplainQueryTool::new(ctx.clone())),
        Box::new(sample_values::SampleFieldValuesTool::new(ctx.clone())),
        Box::new(generate_report::GenerateReportTool::new(ctx.clone())),
    ];

    if !ctx.read_only {
        tools.extend([
            Box::new(insert::InsertDocumentsTool::new(ctx.clone())) as Box<dyn ToolDyn>,
            Box::new(replace::ReplaceDocumentsTool::new(ctx.clone())),
            Box::new(delete::DeleteDocumentsTool::new(ctx.clone())),
            Box::new(create_index::CreateIndexTool::new(ctx.clone())),
            Box::new(self::drop_index::DropIndexTool::new(ctx)),
        ]);
    }

    tools
}

pub fn require_reversible_history(
    ctx: &MongoContext,
) -> Result<std::sync::Arc<crate::operations::OperationEngine>, ToolError> {
    if !ctx.write_identity.reversible_history {
        return Err(ToolError::Rejected(
            "Built-in AI document writes require Reversible history on this connection."
                .to_string(),
        ));
    }
    ctx.operation_engine
        .clone()
        .ok_or_else(|| ToolError::History("History is unavailable; no write was made.".to_string()))
}

pub fn reversible_target(
    ctx: &MongoContext,
    collection: &str,
    id: bson::Bson,
) -> crate::operations::DocumentTarget {
    crate::operations::DocumentTarget {
        connection_id: ctx.write_identity.id,
        connection_name: ctx.write_identity.name.clone(),
        database: ctx.database.clone(),
        collection: collection.to_string(),
        id,
    }
}

pub async fn execute_reversible_mutations(
    ctx: &MongoContext,
    collection: &str,
    mutations: Vec<crate::operations::Mutation>,
) -> Result<usize, ToolError> {
    let engine = require_reversible_history(ctx)?;
    let total = mutations.len();
    let task_result = tokio::task::spawn_blocking(move || {
        for (completed, mutation) in mutations.into_iter().enumerate() {
            engine.execute(crate::operations::OperationContext::built_in_ai(), mutation).map_err(
                |error| {
                    (
                        completed,
                        format!(
                            "write stopped after {completed} of {total} documents: {}",
                            error.user_message()
                        ),
                    )
                },
            )?;
        }
        Ok(total)
    })
    .await;
    if total > 0
        && let Some(tx) = &ctx.event_tx
    {
        let _ = tx.send(StreamEvent::DocumentsChanged {
            connection_id: ctx.write_identity.id,
            database: ctx.database.clone(),
            collection: collection.to_string(),
        });
    }
    task_result
        .map_err(|error| ToolError::History(format!("History task failed: {error}")))?
        .map_err(|(_, error)| ToolError::History(error))
}

pub fn ensure_writable(ctx: &MongoContext) -> Result<(), ToolError> {
    if ctx.read_only {
        Err(ToolError::Rejected(
            "This connection is read-only; AI write operations are disabled.".to_string(),
        ))
    } else {
        Ok(())
    }
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
    ensure_writable(ctx)?;
    let classification = classify_tool_call(tool_name, args_json);
    match classification.tier {
        SafetyTier::AutoExecute => Ok(()),
        SafetyTier::Blocked | SafetyTier::ConfirmFirst | SafetyTier::AlwaysConfirm => {
            if let Some(tx) = &ctx.event_tx {
                let mut preview = preview;
                if classification.tier == SafetyTier::Blocked {
                    preview.reason = classification.reason.clone();
                }
                let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                let _ = tx.send(StreamEvent::ConfirmationRequired {
                    tool_name: tool_name.to_string(),
                    description: classification.description,
                    tier: classification.tier,
                    preview,
                    write_identity: ctx.write_identity.clone(),
                    response_tx: ConfirmationSender::new(resp_tx),
                });
                match resp_rx.await {
                    Ok(true) => Ok(()),
                    Ok(false) => Err(ToolError::Rejected("User rejected the operation".into())),
                    Err(_) => Err(ToolError::Rejected("Operation cancelled".into())),
                }
            } else {
                Err(ToolError::Rejected(
                    "Write confirmation is unavailable; operation cancelled.".to_string(),
                ))
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
        for key in ["documents", "results", "indexes", "sheets"] {
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
///
/// Supports MongoDB extended JSON (`{"$oid": "..."}`, `{"$date": "..."}`) and
/// shell syntax (`ObjectId("...")`, `ISODate("...")`) so that LLM-generated
/// filters with ObjectId references are correctly converted to BSON types.
pub fn parse_json_to_doc(json_str: &str) -> Result<bson::Document, ToolError> {
    crate::bson::parse_document_from_json(json_str).map_err(ToolError::InvalidInput)
}

/// Convert a BSON document to a relaxed JSON value.
pub fn doc_to_json(doc: &bson::Document) -> serde_json::Value {
    // Use Bson's extended JSON serialization for clean output
    let bson_val = bson::Bson::Document(doc.clone());
    serde_json::to_value(bson_val).unwrap_or(serde_json::Value::Null)
}

const MAX_OUTPUT_BYTES: usize = 32 * 1024;
const MAX_FIND_LIMIT: i64 = 50;

pub mod drop_index;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mongodb::bson::doc;
    use tempfile::TempDir;

    use super::{
        MongoContext, StreamEvent, ToolError, execute_reversible_mutations,
        require_reversible_history, reversible_target,
    };

    #[tokio::test]
    async fn ai_document_writes_fail_closed_without_reversible_history() {
        let connection = crate::models::SavedConnection::new(
            "Local".to_string(),
            "mongodb://localhost:27017".to_string(),
        );
        let context = MongoContext {
            client: mongodb::Client::with_uri_str("mongodb://localhost:27017").await.unwrap(),
            database: "app".to_string(),
            collection: Some("users".to_string()),
            write_identity: crate::models::ConnectionWriteIdentity::from(&connection),
            read_only: false,
            operation_engine: None,
            event_tx: None,
        };

        assert!(matches!(require_reversible_history(&context), Err(ToolError::Rejected(_))));

        let mut enabled = context;
        enabled.write_identity.reversible_history = true;
        assert!(matches!(require_reversible_history(&enabled), Err(ToolError::History(_))));
    }

    #[tokio::test]
    async fn ai_mutations_use_built_in_origin_and_keep_partial_progress_revertible() {
        let directory = TempDir::new().unwrap();
        let backend = Arc::new(crate::operations::InMemoryMutationBackend::default());
        let engine = Arc::new(
            crate::operations::OperationEngine::open(
                directory.path().join("history.sqlite3"),
                [31; 32],
                backend.clone(),
            )
            .unwrap(),
        );
        let mut connection = crate::models::SavedConnection::new(
            "Local".to_string(),
            "mongodb://localhost:27017".to_string(),
        );
        connection.reversible_history = true;
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let context = MongoContext {
            client: mongodb::Client::with_uri_str("mongodb://localhost:27017").await.unwrap(),
            database: "app".to_string(),
            collection: Some("users".to_string()),
            write_identity: crate::models::ConnectionWriteIdentity::from(&connection),
            read_only: false,
            operation_engine: Some(engine.clone()),
            event_tx: Some(event_tx),
        };
        let first = reversible_target(&context, "users", 1.into());
        let second = reversible_target(&context, "users", 2.into());
        let inserted = execute_reversible_mutations(
            &context,
            "users",
            vec![
                crate::operations::Mutation::InsertDocument {
                    target: first,
                    document: doc! { "_id": 1, "value": "first" },
                },
                crate::operations::Mutation::InsertDocument {
                    target: second,
                    document: doc! { "_id": 2, "value": "second" },
                },
            ],
        )
        .await
        .unwrap();

        assert_eq!(inserted, 2);
        assert!(matches!(event_rx.recv().await, Some(StreamEvent::DocumentsChanged { .. })));
        assert!(
            engine
                .list(crate::operations::OperationQuery::default())
                .unwrap()
                .items
                .iter()
                .all(|operation| operation.origin == crate::operations::OperationOrigin::BuiltInAi)
        );

        let completed = reversible_target(&context, "users", 3.into());
        let conflict = reversible_target(&context, "users", 4.into());
        backend.set_document(&conflict, doc! { "_id": 4, "value": "existing" });
        let error = execute_reversible_mutations(
            &context,
            "users",
            vec![
                crate::operations::Mutation::InsertDocument {
                    target: completed.clone(),
                    document: doc! { "_id": 3, "value": "completed" },
                },
                crate::operations::Mutation::InsertDocument {
                    target: conflict.clone(),
                    document: doc! { "_id": 4, "value": "blocked" },
                },
            ],
        )
        .await
        .unwrap_err();

        assert!(matches!(error, ToolError::History(_)));
        assert!(matches!(event_rx.recv().await, Some(StreamEvent::DocumentsChanged { .. })));
        assert_eq!(backend.document(&completed), Some(doc! { "_id": 3, "value": "completed" }));
        assert_eq!(backend.document(&conflict), Some(doc! { "_id": 4, "value": "existing" }));

        execute_reversible_mutations(
            &context,
            "users",
            vec![crate::operations::Mutation::InsertDocument {
                target: conflict,
                document: doc! { "_id": 4, "value": "still blocked" },
            }],
        )
        .await
        .unwrap_err();
        assert!(matches!(event_rx.recv().await, Some(StreamEvent::DocumentsChanged { .. })));
    }
}
