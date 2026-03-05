use futures::TryStreamExt;
use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::state::commands::schema_to_summary;

use super::{MAX_OUTPUT_BYTES, MongoContext, ToolError, resolve_collection, truncate_output};

const SCHEMA_SAMPLE_SIZE: i64 = 100;

pub struct CollectionSchemaTool(MongoContext);

impl CollectionSchemaTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct SchemaArgs {
    pub collection: Option<String>,
}

impl Tool for CollectionSchemaTool {
    const NAME: &'static str = "collection_schema";
    type Error = ToolError;
    type Args = SchemaArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Sample documents and analyze the schema of a collection. \
                Returns field names, types, presence percentages, and cardinality."
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

    async fn call(&self, args: SchemaArgs) -> Result<serde_json::Value, ToolError> {
        let col = resolve_collection(&args.collection, &self.0)?;
        let collection =
            self.0.client.database(&self.0.database).collection::<bson::Document>(&col);

        let pipeline = vec![bson::doc! { "$sample": { "size": SCHEMA_SAMPLE_SIZE } }];
        let cursor = collection.aggregate(pipeline).await?;
        let docs: Vec<bson::Document> = cursor.try_collect().await?;

        let total = collection.estimated_document_count().await.unwrap_or(0);
        let analysis = crate::state::commands::build_schema_analysis(&docs, total);
        let summary = schema_to_summary(&analysis);

        let result = serde_json::json!({
            "collection": col,
            "schema_summary": summary,
        });
        Ok(truncate_output(result, MAX_OUTPUT_BYTES))
    }
}
