use futures::TryStreamExt;
use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::ai::safety::OperationPreview;

use super::{
    MongoContext, ToolError, doc_to_json, parse_json_to_doc, require_confirmation,
    resolve_collection,
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
        let col_name = resolve_collection(&args.collection, &self.0)?;
        let filter = parse_json_to_doc(&args.filter)?;

        let collection =
            self.0.client.database(&self.0.database).collection::<bson::Document>(&col_name);

        // Build preview: count + sample docs matching filter
        let count = collection.count_documents(filter.clone()).await?;
        let cursor = collection.find(filter.clone()).limit(3).await?;
        let sample_bson: Vec<bson::Document> = cursor.try_collect().await?;
        let sample_docs: Vec<serde_json::Value> = sample_bson.iter().map(doc_to_json).collect();

        let preview =
            OperationPreview { collection: col_name.clone(), affected_count: count, sample_docs };

        let args_json = serde_json::to_string(&serde_json::json!({
            "filter": args.filter,
        }))
        .unwrap_or_default();
        require_confirmation(&self.0, Self::NAME, &args_json, preview).await?;

        // Execute
        let result = collection.delete_many(filter).await?;

        Ok(serde_json::json!({
            "deleted_count": result.deleted_count,
        }))
    }
}
