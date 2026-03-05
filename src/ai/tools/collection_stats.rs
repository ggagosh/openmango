use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use super::{MongoContext, ToolError, resolve_collection};

pub struct CollectionStatsTool(MongoContext);

impl CollectionStatsTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct CollStatsArgs {
    pub collection: Option<String>,
}

impl Tool for CollectionStatsTool {
    const NAME: &'static str = "collection_stats";
    type Error = ToolError;
    type Args = CollStatsArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Get statistics for a collection (document count, sizes, index info)."
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

    async fn call(&self, args: CollStatsArgs) -> Result<serde_json::Value, ToolError> {
        use futures::TryStreamExt as _;

        let col_name = resolve_collection(&args.collection, &self.0)?;
        let db = self.0.client.database(&self.0.database);
        let collection = db.collection::<bson::Document>(&col_name);

        // Use $collStats aggregation (works on MongoDB 6.0+, unlike the legacy collStats command)
        let pipeline = vec![bson::doc! {
            "$collStats": { "storageStats": { "scale": 1 } }
        }];
        let mut cursor = collection.aggregate(pipeline).await?;

        let stats = cursor
            .try_next()
            .await?
            .ok_or_else(|| ToolError::InvalidInput("No stats returned".to_string()))?;

        let ss = stats.get_document("storageStats").ok();

        let get_i64 = |doc: Option<&bson::Document>, key: &str| -> i64 {
            doc.and_then(|d| d.get(key))
                .and_then(|v| match v {
                    bson::Bson::Int64(n) => Some(*n),
                    bson::Bson::Int32(n) => Some(*n as i64),
                    bson::Bson::Double(n) => Some(*n as i64),
                    _ => None,
                })
                .unwrap_or(0)
        };

        let count = get_i64(ss, "count");
        let size = get_i64(ss, "size");
        let avg_obj_size = get_i64(ss, "avgObjSize");
        let storage_size = get_i64(ss, "storageSize");
        let n_indexes = get_i64(ss, "nindexes") as i32;
        let total_index_size = get_i64(ss, "totalIndexSize");
        let capped = ss.and_then(|d| d.get_bool("capped").ok()).unwrap_or(false);

        Ok(serde_json::json!({
            "collection": col_name,
            "document_count": count,
            "data_size_bytes": size,
            "avg_document_size_bytes": avg_obj_size,
            "storage_size_bytes": storage_size,
            "index_count": n_indexes,
            "total_index_size_bytes": total_index_size,
            "capped": capped,
        }))
    }
}
