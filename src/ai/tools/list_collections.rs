use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use super::{MongoContext, ToolError};

pub struct ListCollectionsTool(MongoContext);

impl ListCollectionsTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct ListCollectionsArgs {}

impl Tool for ListCollectionsTool {
    const NAME: &'static str = "list_collections";
    type Error = ToolError;
    type Args = ListCollectionsArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "List all collections in the current database.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn call(&self, _args: ListCollectionsArgs) -> Result<serde_json::Value, ToolError> {
        let db = self.0.client.database(&self.0.database);
        let names: Vec<String> = db.list_collection_names().await?;

        Ok(serde_json::json!({
            "database": self.0.database,
            "collections": names,
            "count": names.len(),
        }))
    }
}
