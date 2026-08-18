use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::ai::safety::OperationPreview;

use super::{
    MongoContext, ToolError, ensure_writable, execute_reversible_index_mutation, parse_json_to_doc,
    require_confirmation, require_reversible_history, resolve_collection, reversible_target,
};

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
                "required": ["keys", "name"]
            }),
        }
    }

    async fn call(&self, args: CreateIndexArgs) -> Result<serde_json::Value, ToolError> {
        ensure_writable(&self.0)?;
        require_reversible_history(&self.0)?;
        let col_name = resolve_collection(&args.collection, &self.0)?;
        let keys = parse_json_to_doc(&args.keys)?;
        let name = args.name.clone().filter(|name| !name.trim().is_empty()).ok_or_else(|| {
            ToolError::InvalidInput(
                "Reversible index creation requires an explicit name".to_string(),
            )
        })?;

        // Preview: show the index definition (no docs affected)
        let index_def = serde_json::json!({
            "keys": args.keys,
            "unique": args.unique.unwrap_or(false),
            "name": name,
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

        let mut definition = bson::doc! { "key": keys, "name": name.clone() };
        if let Some(unique) = args.unique {
            definition.insert("unique", unique);
        }
        execute_reversible_index_mutation(
            &self.0,
            &col_name,
            crate::operations::Mutation::CreateIndex {
                target: reversible_target(&self.0, &col_name, name.clone().into()),
                definition,
            },
        )
        .await?;

        Ok(serde_json::json!({ "index_name": name }))
    }
}
