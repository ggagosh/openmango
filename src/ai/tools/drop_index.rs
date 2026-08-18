use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::ai::safety::OperationPreview;

use super::{
    MongoContext, ToolError, ensure_writable, execute_reversible_index_mutation,
    require_confirmation, require_reversible_history, resolve_collection, reversible_target,
};

pub struct DropIndexTool(MongoContext);

impl DropIndexTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct DropIndexArgs {
    pub collection: Option<String>,
    pub index_name: String,
}

impl Tool for DropIndexTool {
    const NAME: &'static str = "drop_index";
    type Error = ToolError;
    type Args = DropIndexArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Drop an index from a MongoDB collection by name. \
                The _id_ index cannot be dropped."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection name (optional if a default is set)"
                    },
                    "index_name": {
                        "type": "string",
                        "description": "Name of the index to drop"
                    }
                },
                "required": ["index_name"]
            }),
        }
    }

    async fn call(&self, args: DropIndexArgs) -> Result<serde_json::Value, ToolError> {
        ensure_writable(&self.0)?;
        require_reversible_history(&self.0)?;
        let col_name = resolve_collection(&args.collection, &self.0)?;

        // Block dropping the _id_ index
        if args.index_name == "_id_" {
            return Err(ToolError::InvalidInput("Cannot drop the _id_ index".to_string()));
        }

        // Preview: show the index name being dropped
        let preview = OperationPreview {
            collection: col_name.clone(),
            affected_count: 0,
            sample_docs: vec![serde_json::json!({ "index_name": &args.index_name })],
            reason: None,
        };

        let args_json = serde_json::to_string(&serde_json::json!({
            "index_name": args.index_name,
        }))
        .unwrap_or_default();
        require_confirmation(&self.0, Self::NAME, &args_json, preview).await?;

        execute_reversible_index_mutation(
            &self.0,
            &col_name,
            crate::operations::Mutation::DropIndex {
                target: reversible_target(&self.0, &col_name, args.index_name.clone().into()),
            },
        )
        .await?;

        Ok(serde_json::json!({ "dropped": args.index_name }))
    }
}
