use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::ai::safety::OperationPreview;

use super::{
    MongoContext, ToolError, ensure_writable, execute_reversible_mutations, require_confirmation,
    require_reversible_history, resolve_collection, reversible_target,
};

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
        ensure_writable(&self.0)?;
        require_reversible_history(&self.0)?;
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
        if docs_array.len() > crate::operations::MAX_REVERSIBLE_BULK_DOCUMENTS {
            return Err(ToolError::InvalidInput(format!(
                "Too many documents ({}). Maximum is {}.",
                docs_array.len(),
                crate::operations::MAX_REVERSIBLE_BULK_DOCUMENTS
            )));
        }

        // Convert to BSON documents
        let mut bson_docs: Vec<bson::Document> = docs_array
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

        for document in &mut bson_docs {
            if !document.contains_key("_id") {
                document.insert("_id", bson::oid::ObjectId::new());
            }
        }
        crate::operations::ensure_reversible_bulk_size(bson_docs.iter())
            .map_err(|error| ToolError::History(error.user_message().to_string()))?;
        let mutations = bson_docs
            .into_iter()
            .map(|document| {
                let id = document.get("_id").cloned().expect("assigned above");
                crate::operations::Mutation::InsertDocument {
                    target: reversible_target(&self.0, &col_name, id),
                    document,
                }
            })
            .collect();
        let inserted_count = execute_reversible_mutations(&self.0, &col_name, mutations).await?;

        Ok(serde_json::json!({ "inserted_count": inserted_count }))
    }
}
