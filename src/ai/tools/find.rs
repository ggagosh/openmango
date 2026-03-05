use futures::TryStreamExt;
use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use super::{
    MAX_FIND_LIMIT, MAX_OUTPUT_BYTES, MongoContext, ToolError, doc_to_json, parse_json_to_doc,
    resolve_collection, truncate_output,
};

pub struct FindDocumentsTool(MongoContext);

impl FindDocumentsTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct FindArgs {
    pub collection: Option<String>,
    pub filter: Option<String>,
    pub projection: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<i64>,
}

impl Tool for FindDocumentsTool {
    const NAME: &'static str = "find_documents";
    type Error = ToolError;
    type Args = FindArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Query documents in a MongoDB collection with optional filter, \
                projection, sort, and limit. Returns up to 10 documents."
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
                        "description": "MongoDB filter as a JSON string, e.g. {\"age\": {\"$gt\": 25}}"
                    },
                    "projection": {
                        "type": "string",
                        "description": "Fields to include/exclude as JSON, e.g. {\"name\": 1, \"_id\": 0}"
                    },
                    "sort": {
                        "type": "string",
                        "description": "Sort order as JSON, e.g. {\"created_at\": -1}"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max documents to return (capped at 10)"
                    }
                }
            }),
        }
    }

    async fn call(&self, args: FindArgs) -> Result<serde_json::Value, ToolError> {
        let col = resolve_collection(&args.collection, &self.0)?;
        let filter = match &args.filter {
            Some(f) if !f.is_empty() => parse_json_to_doc(f)?,
            _ => bson::Document::new(),
        };
        let limit = args.limit.unwrap_or(MAX_FIND_LIMIT).min(MAX_FIND_LIMIT);

        let collection =
            self.0.client.database(&self.0.database).collection::<bson::Document>(&col);
        let mut find_opts = mongodb::options::FindOptions::builder().limit(Some(limit)).build();

        if let Some(ref sort_str) = args.sort
            && !sort_str.is_empty()
        {
            find_opts.sort = Some(parse_json_to_doc(sort_str)?);
        }
        if let Some(ref proj_str) = args.projection
            && !proj_str.is_empty()
        {
            find_opts.projection = Some(parse_json_to_doc(proj_str)?);
        }

        let cursor = collection.find(filter).with_options(find_opts).await?;
        let docs: Vec<bson::Document> = cursor.try_collect().await?;
        let json_docs: Vec<serde_json::Value> = docs.iter().map(doc_to_json).collect();

        let result = serde_json::json!({
            "count": json_docs.len(),
            "documents": json_docs,
        });
        Ok(truncate_output(result, MAX_OUTPUT_BYTES))
    }
}
