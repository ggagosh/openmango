use futures::TryStreamExt;
use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use super::{
    MAX_FIND_LIMIT, MAX_OUTPUT_BYTES, MongoContext, ToolError, doc_to_json, resolve_collection,
    truncate_output,
};

pub struct AggregateTool(MongoContext);

impl AggregateTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct AggregateArgs {
    pub collection: Option<String>,
    pub pipeline: String,
}

impl Tool for AggregateTool {
    const NAME: &'static str = "aggregate";
    type Error = ToolError;
    type Args = AggregateArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Run a MongoDB aggregation pipeline. A $limit stage (max 50) is \
                appended if the pipeline does not already contain one."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection name (optional if a default is set)"
                    },
                    "pipeline": {
                        "type": "string",
                        "description": "Aggregation pipeline as a JSON array of stage objects"
                    }
                },
                "required": ["pipeline"]
            }),
        }
    }

    async fn call(&self, args: AggregateArgs) -> Result<serde_json::Value, ToolError> {
        let col = resolve_collection(&args.collection, &self.0)?;

        let value: serde_json::Value = serde_json::from_str(&args.pipeline)?;
        let stages: Vec<serde_json::Value> = match value {
            serde_json::Value::Array(arr) => arr,
            _ => {
                return Err(ToolError::InvalidInput("Pipeline must be a JSON array".to_string()));
            }
        };

        let mut pipeline: Vec<bson::Document> = stages
            .into_iter()
            .map(|stage| {
                let bson_val =
                    bson::to_bson(&stage).map_err(|e| ToolError::InvalidInput(e.to_string()))?;
                match bson_val {
                    bson::Bson::Document(doc) => Ok(doc),
                    _ => Err(ToolError::InvalidInput(
                        "Each pipeline stage must be an object".to_string(),
                    )),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Force $limit if missing
        let has_limit = pipeline.iter().any(|stage| stage.keys().any(|k| k == "$limit"));
        if !has_limit {
            pipeline.push(bson::doc! { "$limit": MAX_FIND_LIMIT });
        }

        let collection =
            self.0.client.database(&self.0.database).collection::<bson::Document>(&col);
        let cursor = collection.aggregate(pipeline).await?;
        let docs: Vec<bson::Document> = cursor.try_collect().await?;
        let json_docs: Vec<serde_json::Value> = docs.iter().map(doc_to_json).collect();

        let result = serde_json::json!({
            "count": json_docs.len(),
            "results": json_docs,
        });
        Ok(truncate_output(result, MAX_OUTPUT_BYTES))
    }
}
