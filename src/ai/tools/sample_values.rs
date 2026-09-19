use mongodb::bson;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;

use super::{MAX_OUTPUT_BYTES, MongoContext, ToolError, resolve_collection, truncate_output};

const MAX_DISTINCT: usize = 50;

pub struct SampleFieldValuesTool(MongoContext);

impl SampleFieldValuesTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct SampleFieldValuesArgs {
    pub collection: Option<String>,
    pub field: String,
    pub filter: Option<String>,
}

impl Tool for SampleFieldValuesTool {
    const NAME: &'static str = "sample_field_values";
    type Error = ToolError;
    type Args = SampleFieldValuesArgs;
    type Output = serde_json::Value;

    fn description(&self) -> String {
        "Get distinct values for a field in a collection. \
                Use to discover statuses, categories, types, or any enumeration \
                before querying. Essential when you need exact values \
                (e.g., \"APPROVED\" vs \"approved\"). Returns up to 50 values."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "collection": {
                    "type": "string",
                    "description": "Collection name (optional if a default is set)"
                },
                "field": {
                    "type": "string",
                    "description": "Field name to get distinct values for (e.g., \"status\", \"type\")"
                },
                "filter": {
                    "type": "string",
                    "description": "Optional MongoDB filter as a JSON string to scope the distinct query"
                }
            },
            "required": ["field"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: SampleFieldValuesArgs,
    ) -> Result<serde_json::Value, ToolError> {
        let col = resolve_collection(&args.collection, &self.0)?;
        let collection =
            self.0.client.database(&self.0.database).collection::<bson::Document>(&col);

        let filter = match &args.filter {
            Some(f) if !f.is_empty() => super::parse_json_to_doc(f)?,
            _ => bson::doc! {},
        };

        let raw_values: Vec<bson::Bson> = collection.distinct(&args.field, filter).await?;

        let total_distinct = raw_values.len();
        let values: Vec<serde_json::Value> = raw_values
            .into_iter()
            .take(MAX_DISTINCT)
            .map(|b| serde_json::to_value(&b).unwrap_or(serde_json::Value::Null))
            .collect();

        let result = serde_json::json!({
            "field": args.field,
            "collection": col,
            "values": values,
            "total_distinct": total_distinct,
        });
        Ok(truncate_output(result, MAX_OUTPUT_BYTES))
    }
}
