use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::ai::safety::OperationPreview;

use super::{MongoContext, ToolError, require_confirmation, resolve_collection};

pub struct InsertDocumentsTool(MongoContext);

impl InsertDocumentsTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct InsertArgs {
    pub collection: Option<String>,
    pub documents: String,
}

const MAX_INSERT_COUNT: usize = 100;

impl Tool for InsertDocumentsTool {
    const NAME: &'static str = "insert_documents";
    type Error = ToolError;
    type Args = InsertArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Insert one or more documents into a MongoDB collection. \
                Pass documents as a JSON array string. Max 100 documents per call."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection name (optional if a default is set)"
                    },
                    "documents": {
                        "type": "string",
                        "description": "JSON array of documents to insert, e.g. [{\"name\": \"Alice\"}, {\"name\": \"Bob\"}]"
                    }
                },
                "required": ["documents"]
            }),
        }
    }

    async fn call(&self, args: InsertArgs) -> Result<serde_json::Value, ToolError> {
        let col_name = resolve_collection(&args.collection, &self.0)?;

        // Parse documents array
        let docs_value: serde_json::Value = serde_json::from_str(&args.documents)?;
        let docs_array = match docs_value {
            serde_json::Value::Array(arr) => arr,
            serde_json::Value::Object(_) => vec![docs_value],
            _ => {
                return Err(ToolError::InvalidInput(
                    "documents must be a JSON array or object".to_string(),
                ));
            }
        };

        if docs_array.is_empty() {
            return Err(ToolError::InvalidInput("No documents to insert".to_string()));
        }
        if docs_array.len() > MAX_INSERT_COUNT {
            return Err(ToolError::InvalidInput(format!(
                "Too many documents ({}). Maximum is {MAX_INSERT_COUNT}.",
                docs_array.len()
            )));
        }

        // Convert to BSON documents
        let bson_docs: Vec<bson::Document> = docs_array
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let bson_val =
                    bson::to_bson(v).map_err(|e| ToolError::InvalidInput(e.to_string()))?;
                match bson_val {
                    bson::Bson::Document(doc) => Ok(doc),
                    _ => Err(ToolError::InvalidInput(format!(
                        "Document at index {i} is not a JSON object"
                    ))),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Build preview
        let sample_docs: Vec<serde_json::Value> = docs_array.iter().take(3).cloned().collect();
        let preview = OperationPreview {
            collection: col_name.clone(),
            affected_count: bson_docs.len() as u64,
            sample_docs,
            reason: None,
        };

        // Serialize args for safety classification
        let args_json = serde_json::to_string(&serde_json::json!({"documents": args.documents}))
            .unwrap_or_default();
        require_confirmation(&self.0, Self::NAME, &args_json, preview).await?;

        // Execute
        let collection =
            self.0.client.database(&self.0.database).collection::<bson::Document>(&col_name);
        let result = collection.insert_many(bson_docs).await?;

        Ok(serde_json::json!({
            "inserted_count": result.inserted_ids.len(),
        }))
    }
}
