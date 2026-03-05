use mongodb::IndexModel;
use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::ai::safety::OperationPreview;

use super::{MongoContext, ToolError, parse_json_to_doc, require_confirmation, resolve_collection};

pub struct CreateIndexTool(MongoContext);

impl CreateIndexTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct CreateIndexArgs {
    pub collection: Option<String>,
    pub keys: String,
    pub unique: Option<bool>,
    pub name: Option<String>,
}

impl Tool for CreateIndexTool {
    const NAME: &'static str = "create_index";
    type Error = ToolError;
    type Args = CreateIndexArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Create an index on a MongoDB collection. Specify the index key \
                definition as a JSON object."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection name (optional if a default is set)"
                    },
                    "keys": {
                        "type": "string",
                        "description": "Index key definition as JSON, e.g. {\"email\": 1} or {\"location\": \"2dsphere\"}"
                    },
                    "unique": {
                        "type": "boolean",
                        "description": "Whether the index should enforce uniqueness"
                    },
                    "name": {
                        "type": "string",
                        "description": "Optional custom name for the index"
                    }
                },
                "required": ["keys"]
            }),
        }
    }

    async fn call(&self, args: CreateIndexArgs) -> Result<serde_json::Value, ToolError> {
        let col_name = resolve_collection(&args.collection, &self.0)?;
        let keys = parse_json_to_doc(&args.keys)?;

        // Preview: show the index definition (no docs affected)
        let index_def = serde_json::json!({
            "keys": args.keys,
            "unique": args.unique.unwrap_or(false),
            "name": args.name.as_deref().unwrap_or("(auto)"),
        });
        let preview = OperationPreview {
            collection: col_name.clone(),
            affected_count: 0,
            sample_docs: vec![index_def],
            reason: None,
        };

        let args_json = serde_json::to_string(&serde_json::json!({
            "keys": args.keys,
        }))
        .unwrap_or_default();
        require_confirmation(&self.0, Self::NAME, &args_json, preview).await?;

        // Build index model
        let opts =
            mongodb::options::IndexOptions::builder().unique(args.unique).name(args.name).build();

        let model = IndexModel::builder().keys(keys).options(opts).build();

        let collection =
            self.0.client.database(&self.0.database).collection::<bson::Document>(&col_name);
        let result = collection.create_index(model).await?;

        Ok(serde_json::json!({
            "index_name": result.index_name,
        }))
    }
}
