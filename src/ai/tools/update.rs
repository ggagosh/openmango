use futures::TryStreamExt;
use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::ai::safety::OperationPreview;

use super::{
    MongoContext, ToolError, doc_to_json, ensure_writable, parse_json_to_doc, require_confirmation,
    resolve_collection,
};

pub struct UpdateDocumentsTool(MongoContext);

impl UpdateDocumentsTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct UpdateArgs {
    pub collection: Option<String>,
    pub filter: String,
    pub update: String,
    pub many: Option<bool>,
}

impl Tool for UpdateDocumentsTool {
    const NAME: &'static str = "update_documents";
    type Error = ToolError;
    type Args = UpdateArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Update documents in a MongoDB collection matching a filter. \
                Uses update_many by default. Set many=false for update_one."
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
                    },
                    "update": {
                        "type": "string",
                        "description": "MongoDB update expression as JSON string, e.g. {\"$set\": {\"status\": \"active\"}}"
                    },
                    "many": {
                        "type": "boolean",
                        "description": "If true (default), update all matching docs. If false, update only the first match."
                    }
                },
                "required": ["filter", "update"]
            }),
        }
    }

    async fn call(&self, args: UpdateArgs) -> Result<serde_json::Value, ToolError> {
        ensure_writable(&self.0)?;
        let col_name = resolve_collection(&args.collection, &self.0)?;
        let filter = parse_json_to_doc(&args.filter)?;
        let update = parse_json_to_doc(&args.update)?;

        let collection =
            self.0.client.database(&self.0.database).collection::<bson::Document>(&col_name);

        // Build preview: count + sample docs matching filter
        let many = args.many.unwrap_or(true);
        let matching_count = collection.count_documents(filter.clone()).await?;
        let affected_count = if many { matching_count } else { matching_count.min(1) };
        let cursor = collection.find(filter.clone()).limit(if many { 3 } else { 1 }).await?;
        let sample_bson: Vec<bson::Document> = cursor.try_collect().await?;
        let sample_docs: Vec<serde_json::Value> = sample_bson.iter().map(doc_to_json).collect();

        let preview = OperationPreview {
            collection: col_name.clone(),
            affected_count,
            sample_docs,
            reason: None,
        };

        let args_json = serde_json::to_string(&serde_json::json!({
            "filter": args.filter,
            "update": args.update,
            "many": many,
        }))
        .unwrap_or_default();
        require_confirmation(&self.0, Self::NAME, &args_json, preview).await?;

        // Execute
        let result = if many {
            let r = collection.update_many(filter, update).await?;
            serde_json::json!({
                "matched_count": r.matched_count,
                "modified_count": r.modified_count,
            })
        } else {
            let r = collection.update_one(filter, update).await?;
            serde_json::json!({
                "matched_count": r.matched_count,
                "modified_count": r.modified_count,
            })
        };

        Ok(result)
    }
}
