use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use super::{MongoContext, ToolError, parse_json_to_doc, resolve_collection};

pub struct CountDocumentsTool(MongoContext);

impl CountDocumentsTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct CountArgs {
    pub collection: Option<String>,
    pub filter: Option<String>,
}

impl Tool for CountDocumentsTool {
    const NAME: &'static str = "count_documents";
    type Error = ToolError;
    type Args = CountArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Count documents in a collection, optionally matching a filter. \
                Uses estimated_document_count when no filter is provided for speed."
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
                        "description": "MongoDB filter as a JSON string"
                    }
                }
            }),
        }
    }

    async fn call(&self, args: CountArgs) -> Result<serde_json::Value, ToolError> {
        let col = resolve_collection(&args.collection, &self.0)?;
        let collection =
            self.0.client.database(&self.0.database).collection::<bson::Document>(&col);

        let count = match &args.filter {
            Some(f) if !f.is_empty() => {
                let filter = parse_json_to_doc(f)?;
                collection.count_documents(filter).await?
            }
            _ => collection.estimated_document_count().await?,
        };

        Ok(serde_json::json!({
            "collection": col,
            "count": count,
        }))
    }
}
