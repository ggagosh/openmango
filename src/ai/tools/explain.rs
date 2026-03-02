use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use super::{
    MAX_OUTPUT_BYTES, MongoContext, ToolError, doc_to_json, parse_json_to_doc, resolve_collection,
    truncate_output,
};

pub struct ExplainQueryTool(MongoContext);

impl ExplainQueryTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct ExplainArgs {
    pub collection: Option<String>,
    pub filter: Option<String>,
    pub sort: Option<String>,
}

impl Tool for ExplainQueryTool {
    const NAME: &'static str = "explain_query";
    type Error = ToolError;
    type Args = ExplainArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Explain a find query's execution plan (queryPlanner verbosity). \
                Shows whether indexes are used, scan type, and key patterns."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection name (optional if a default is set)"
                    },
                    "filter": {
                        "type": "string",
                        "description": "MongoDB filter as a JSON string"
                    },
                    "sort": {
                        "type": "string",
                        "description": "Sort order as JSON"
                    }
                }
            }),
        }
    }

    async fn call(&self, args: ExplainArgs) -> Result<serde_json::Value, ToolError> {
        let col = resolve_collection(&args.collection, &self.0)?;
        let db = self.0.client.database(&self.0.database);

        let filter = match &args.filter {
            Some(f) if !f.is_empty() => parse_json_to_doc(f)?,
            _ => bson::Document::new(),
        };

        let mut find_cmd = bson::doc! {
            "find": &col,
            "filter": filter,
        };
        if let Some(ref sort_str) = args.sort
            && !sort_str.is_empty()
        {
            find_cmd.insert("sort", parse_json_to_doc(sort_str)?);
        }

        let explain_cmd = bson::doc! {
            "explain": find_cmd,
            "verbosity": "queryPlanner",
        };

        let result = db.run_command(explain_cmd).await?;
        Ok(truncate_output(doc_to_json(&result), MAX_OUTPUT_BYTES))
    }
}
