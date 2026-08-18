use futures::TryStreamExt;
use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::ai::safety::OperationPreview;

use super::{
    MongoContext, ToolError, doc_to_json, ensure_writable, execute_reversible_mutations,
    parse_json_to_doc, require_confirmation, require_reversible_history, resolve_collection,
    reversible_target,
};

pub struct DeleteDocumentsTool(MongoContext);

impl DeleteDocumentsTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct DeleteArgs {
    pub collection: Option<String>,
    pub filter: String,
}

impl Tool for DeleteDocumentsTool {
    const NAME: &'static str = "delete_documents";
    type Error = ToolError;
    type Args = DeleteArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Delete documents from a MongoDB collection matching a filter. \
                A non-empty filter is required — empty filters are blocked for safety."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection name (optional if a default is set)"
                    },
                    "filter": {
                        "type": "string",
                        "description": "MongoDB filter as JSON string, e.g. {\"status\": \"inactive\"}"
                    }
                },
                "required": ["filter"]
            }),
        }
    }

    async fn call(&self, args: DeleteArgs) -> Result<serde_json::Value, ToolError> {
        ensure_writable(&self.0)?;
        require_reversible_history(&self.0)?;
        let col_name = resolve_collection(&args.collection, &self.0)?;
        let filter = parse_json_to_doc(&args.filter)?;

        let collection =
            self.0.client.database(&self.0.database).collection::<bson::Document>(&col_name);

        // Build preview: count + sample docs matching filter
        let count = collection.count_documents(filter.clone()).await?;
        if count > crate::operations::MAX_REVERSIBLE_BULK_DOCUMENTS as u64 {
            return Err(ToolError::Rejected(format!(
                "Built-in AI writes are limited to {} documents. Narrow the filter.",
                crate::operations::MAX_REVERSIBLE_BULK_DOCUMENTS
            )));
        }
        let cursor = collection.find(filter.clone()).limit(3).await?;
        let sample_bson: Vec<bson::Document> = cursor.try_collect().await?;
        let sample_docs: Vec<serde_json::Value> = sample_bson.iter().map(doc_to_json).collect();

        let preview = OperationPreview {
            collection: col_name.clone(),
            affected_count: count,
            sample_docs,
            reason: None,
        };

        let args_json = serde_json::to_string(&serde_json::json!({
            "filter": args.filter,
        }))
        .unwrap_or_default();
        require_confirmation(&self.0, Self::NAME, &args_json, preview).await?;

        let cursor = collection
            .find(filter)
            .sort(bson::doc! { "_id": 1 })
            .limit((crate::operations::MAX_REVERSIBLE_BULK_DOCUMENTS + 1) as i64)
            .await?;
        let documents: Vec<bson::Document> = cursor.try_collect().await?;
        if documents.len() > crate::operations::MAX_REVERSIBLE_BULK_DOCUMENTS {
            return Err(ToolError::Rejected(
                "More documents matched after confirmation. Narrow the filter and retry."
                    .to_string(),
            ));
        }
        crate::operations::ensure_reversible_bulk_size(documents.iter())
            .map_err(|error| ToolError::History(error.user_message().to_string()))?;
        if documents.iter().any(|document| !document.contains_key("_id")) {
            return Err(ToolError::History(
                "A matched document has no _id; no writes were attempted.".to_string(),
            ));
        }
        let mutations = documents
            .into_iter()
            .map(|document| {
                let id = document.get("_id").cloned().expect("validated above");
                crate::operations::Mutation::DeleteDocument {
                    target: reversible_target(&self.0, &col_name, id),
                    editor_precondition: Some(document),
                }
            })
            .collect();
        let deleted_count = execute_reversible_mutations(&self.0, &col_name, mutations).await?;

        Ok(serde_json::json!({ "deleted_count": deleted_count }))
    }
}
