use futures::TryStreamExt;
use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use super::{MongoContext, ToolError, doc_to_json, resolve_collection};

pub struct ListIndexesTool(MongoContext);

impl ListIndexesTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct ListIndexesArgs {
    pub collection: Option<String>,
}

impl Tool for ListIndexesTool {
    const NAME: &'static str = "list_indexes";
    type Error = ToolError;
    type Args = ListIndexesArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "List all indexes on a collection, including key definitions and options."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection name (optional if a default is set)"
                    }
                }
            }),
        }
    }

    async fn call(&self, args: ListIndexesArgs) -> Result<serde_json::Value, ToolError> {
        let col = resolve_collection(&args.collection, &self.0)?;
        let collection =
            self.0.client.database(&self.0.database).collection::<bson::Document>(&col);

        let cursor = collection.list_indexes().await?;
        let indexes: Vec<mongodb::IndexModel> = cursor.try_collect().await?;

        let index_list: Vec<serde_json::Value> = indexes
            .into_iter()
            .map(|idx| {
                let keys = doc_to_json(&idx.keys);
                let name = idx.options.as_ref().and_then(|o| o.name.clone()).unwrap_or_default();
                let unique = idx.options.as_ref().and_then(|o| o.unique).unwrap_or(false);
                let sparse = idx.options.as_ref().and_then(|o| o.sparse).unwrap_or(false);
                serde_json::json!({
                    "name": name,
                    "keys": keys,
                    "unique": unique,
                    "sparse": sparse,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "collection": col,
            "indexes": index_list,
            "count": index_list.len(),
        }))
    }
}
