use futures::TryStreamExt;
use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::ai::safety::OperationPreview;

use super::{
    MAX_FIND_LIMIT, MAX_OUTPUT_BYTES, MongoContext, ToolError, doc_to_json, ensure_writable,
    parse_json_to_doc, require_confirmation, resolve_collection, truncate_output,
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

        // Re-serialize each stage and parse through parse_json_to_doc which handles
        // extended JSON ($oid, $date) and shell syntax (ObjectId, ISODate).
        let mut pipeline: Vec<bson::Document> = stages
            .into_iter()
            .map(|stage| {
                let stage_str = serde_json::to_string(&stage)
                    .map_err(|e| ToolError::InvalidInput(e.to_string()))?;
                parse_json_to_doc(&stage_str)
            })
            .collect::<Result<Vec<_>, _>>()?;

        if let Some((operator, target)) = output_stage(&pipeline, &self.0.database) {
            ensure_writable(&self.0)?;
            let preview = OperationPreview {
                collection: target.clone(),
                affected_count: 0,
                sample_docs: vec![serde_json::json!({
                    "operation": operator,
                    "target": target,
                })],
                reason: Some(format!("{operator} writes aggregation results to a collection")),
            };
            let args_json = serde_json::to_string(&serde_json::json!({
                "collection": col,
                "pipeline": args.pipeline,
                "output_stage": operator,
            }))
            .unwrap_or_default();
            require_confirmation(&self.0, Self::NAME, &args_json, preview).await?;
        } else {
            // Bound read-only pipelines without changing output-stage ordering.
            let has_limit = pipeline.iter().any(|stage| stage.keys().any(|k| k == "$limit"));
            if !has_limit {
                pipeline.push(bson::doc! { "$limit": MAX_FIND_LIMIT });
            }
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

fn output_stage(
    pipeline: &[bson::Document],
    default_database: &str,
) -> Option<(&'static str, String)> {
    for stage in pipeline {
        if let Some(target) = stage.get("$out") {
            return Some(("$out", output_namespace(target, default_database)));
        }
        if let Some(target) = stage.get("$merge") {
            let target = match target {
                bson::Bson::Document(options) => options.get("into").unwrap_or(target),
                _ => target,
            };
            return Some(("$merge", output_namespace(target, default_database)));
        }
    }
    None
}

fn output_namespace(target: &bson::Bson, default_database: &str) -> String {
    match target {
        bson::Bson::String(collection) => format!("{default_database}.{collection}"),
        bson::Bson::Document(namespace) => {
            let database = namespace.get_str("db").unwrap_or(default_database);
            let collection = namespace.get_str("coll").unwrap_or("<unknown>");
            format!("{database}.{collection}")
        }
        _ => format!("{default_database}.<unknown>"),
    }
}
