use futures::TryStreamExt;
use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use super::{MongoContext, ToolError, doc_to_json, parse_json_to_doc, resolve_collection};

/// Maximum rows fetched for the preview (one extra to detect "has more").
const PREVIEW_FETCH_LIMIT: i64 = 11;

const PREVIEW_DISPLAY_LIMIT: usize = 10;

/// Aggregation pipeline stages that perform writes — stripped for safety.
const UNSAFE_STAGES: &[&str] = &["$merge", "$out"];

pub struct GenerateReportTool(MongoContext);

impl GenerateReportTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct GenerateReportArgs {
    pub title: Option<String>,
    pub sheets: Vec<SheetSpec>,
}

#[derive(Deserialize)]
pub struct SheetSpec {
    pub name: String,
    pub collection: Option<String>,
    pub pipeline: String,
}

impl Tool for GenerateReportTool {
    const NAME: &'static str = "generate_report";
    type Error = ToolError;
    type Args = GenerateReportArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Generate a downloadable report from MongoDB data. Use this when the \
                user asks for a report, export, spreadsheet, or downloadable data. Supports \
                multiple sheets (tabs) in a single Excel report. Each sheet runs an aggregation \
                pipeline. Returns a small preview; the user can then download the full report \
                as Excel."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Report title (e.g. 'Top Customers by Revenue')"
                    },
                    "sheets": {
                        "type": "array",
                        "description": "One or more sheets. Each runs its own aggregation \
                            pipeline. For a simple report use a single sheet.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {
                                    "type": "string",
                                    "description": "Sheet/tab name (e.g. 'By Region')"
                                },
                                "collection": {
                                    "type": "string",
                                    "description": "Collection name (optional if default is set)"
                                },
                                "pipeline": {
                                    "type": "string",
                                    "description": "Aggregation pipeline as a JSON array"
                                }
                            },
                            "required": ["name", "pipeline"]
                        },
                        "minItems": 1
                    }
                },
                "required": ["sheets"]
            }),
        }
    }

    async fn call(&self, args: GenerateReportArgs) -> Result<serde_json::Value, ToolError> {
        if args.sheets.is_empty() {
            return Err(ToolError::InvalidInput("At least one sheet is required".to_string()));
        }

        let title = args.title.unwrap_or_else(|| "Report".to_string());
        let mut sheet_results = Vec::new();

        for spec in &args.sheets {
            let col = resolve_collection(&spec.collection, &self.0)?;
            let pipeline = parse_and_sanitize_pipeline(&spec.pipeline)?;

            let recipe_json = pipeline_to_json(&pipeline)?;

            let mut preview_pipeline = pipeline;
            preview_pipeline.push(bson::doc! { "$limit": PREVIEW_FETCH_LIMIT });

            let collection =
                self.0.client.database(&self.0.database).collection::<bson::Document>(&col);
            let cursor = collection.aggregate(preview_pipeline).await?;
            let docs: Vec<bson::Document> = cursor.try_collect().await?;

            let has_more = docs.len() as i64 >= PREVIEW_FETCH_LIMIT;
            let preview_docs: Vec<serde_json::Value> =
                docs.iter().take(PREVIEW_DISPLAY_LIMIT).map(doc_to_json).collect();

            sheet_results.push(serde_json::json!({
                "name": spec.name,
                "collection": col,
                "pipeline": recipe_json,
                "preview": preview_docs,
                "preview_count": preview_docs.len(),
                "has_more": has_more,
            }));
        }

        Ok(serde_json::json!({
            "title": title,
            "sheets": sheet_results,
        }))
    }
}

/// Parse a pipeline JSON string into BSON documents, stripping unsafe write stages.
pub fn parse_and_sanitize_pipeline(pipeline_json: &str) -> Result<Vec<bson::Document>, ToolError> {
    let value: serde_json::Value = serde_json::from_str(pipeline_json)?;
    let stages: Vec<serde_json::Value> = match value {
        serde_json::Value::Array(arr) => arr,
        _ => {
            return Err(ToolError::InvalidInput("Pipeline must be a JSON array".to_string()));
        }
    };

    let mut pipeline = Vec::new();
    for stage in stages {
        let stage_str =
            serde_json::to_string(&stage).map_err(|e| ToolError::InvalidInput(e.to_string()))?;
        let doc = parse_json_to_doc(&stage_str)?;
        // Skip unsafe write stages ($merge, $out).
        if doc.keys().any(|k| UNSAFE_STAGES.contains(&k.as_str())) {
            continue;
        }
        pipeline.push(doc);
    }

    Ok(pipeline)
}

fn pipeline_to_json(pipeline: &[bson::Document]) -> Result<String, ToolError> {
    let json_stages: Vec<serde_json::Value> = pipeline.iter().map(doc_to_json).collect();
    serde_json::to_string(&json_stages).map_err(ToolError::Json)
}
