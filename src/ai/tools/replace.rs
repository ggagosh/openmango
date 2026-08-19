use futures::TryStreamExt as _;
use mongodb::bson;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::ai::safety::OperationPreview;

use super::{
    MongoContext, StreamEvent, ToolError, doc_to_json, ensure_writable, parse_json_to_doc,
    require_confirmation, resolve_collection,
};

pub struct ReplaceDocumentsTool(MongoContext);

fn target_sort() -> bson::Document {
    bson::doc! { "_id": 1 }
}

fn validate_replacement(replacement: &bson::Document) -> Result<(), ToolError> {
    if replacement.is_empty() {
        return Err(ToolError::InvalidInput("Replacement document cannot be empty".to_string()));
    }
    if replacement.contains_key("_id") || replacement.keys().any(|key| key.starts_with('$')) {
        return Err(ToolError::InvalidInput(
            "Replacement must be a complete document without _id or update operators".to_string(),
        ));
    }
    Ok(())
}

impl ReplaceDocumentsTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct ReplaceArgs {
    pub collection: Option<String>,
    pub filter: String,
    pub replacement: String,
    pub many: Option<bool>,
}

impl Tool for ReplaceDocumentsTool {
    const NAME: &'static str = "replace_documents";
    type Error = ToolError;
    type Args = ReplaceArgs;
    type Output = serde_json::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Replace up to 100 documents matching a filter with a complete document. \
                Each original _id is preserved and local confirmation is required."
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
                    "replacement": {
                        "type": "string",
                        "description": "Complete replacement document as a JSON string; omit _id"
                    },
                    "many": {
                        "type": "boolean",
                        "description": "Replace every match by default; false replaces the first _id-sorted match"
                    }
                },
                "required": ["filter", "replacement"]
            }),
        }
    }

    async fn call(&self, args: ReplaceArgs) -> Result<serde_json::Value, ToolError> {
        ensure_writable(&self.0)?;
        let col_name = resolve_collection(&args.collection, &self.0)?;
        let filter = parse_json_to_doc(&args.filter)?;
        let replacement = parse_json_to_doc(&args.replacement)?;
        validate_replacement(&replacement)?;

        let collection =
            self.0.client.database(&self.0.database).collection::<bson::Document>(&col_name);
        let many = args.many.unwrap_or(true);
        let matching_count = collection.count_documents(filter.clone()).await?;
        let affected_count = if many { matching_count } else { matching_count.min(1) };
        let cursor = collection
            .find(filter.clone())
            .sort(target_sort())
            .limit(if many { 3 } else { 1 })
            .await?;
        let sample_bson: Vec<bson::Document> = cursor.try_collect().await?;
        let sample_docs = sample_bson.iter().map(doc_to_json).collect();
        let preview = OperationPreview {
            collection: col_name.clone(),
            affected_count,
            sample_docs,
            reason: None,
        };
        let args_json = serde_json::to_string(&serde_json::json!({
            "filter": args.filter,
            "replacement": args.replacement,
            "many": many,
        }))
        .unwrap_or_default();
        require_confirmation(&self.0, Self::NAME, &args_json, preview).await?;

        let limit = if many { 0 } else { 1 };
        let cursor = collection.find(filter).sort(target_sort()).limit(limit).await?;
        let documents: Vec<bson::Document> = cursor.try_collect().await?;
        let mut matched_count = 0u64;
        let mut modified_count = 0u64;
        for before in documents {
            let id = before.get("_id").cloned().ok_or_else(|| {
                ToolError::InvalidInput("A matched document has no _id".to_string())
            })?;
            let mut after = replacement.clone();
            after.insert("_id", id.clone());
            let result = collection.replace_one(bson::doc! { "_id": id }, after).await?;
            matched_count += result.matched_count;
            modified_count += result.modified_count;
        }
        if matched_count > 0
            && let Some(tx) = &self.0.event_tx
        {
            let _ = tx.send(StreamEvent::DocumentsChanged {
                connection_id: self.0.write_identity.id,
                database: self.0.database.clone(),
                collection: col_name,
            });
        }
        Ok(serde_json::json!({
            "matched_count": matched_count,
            "modified_count": modified_count,
        }))
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::doc;

    use super::{target_sort, validate_replacement};

    #[test]
    fn replacement_must_be_a_complete_document_without_id() {
        assert!(validate_replacement(&doc! { "status": "archived" }).is_ok());
        assert!(validate_replacement(&doc! {}).is_err());
        assert!(validate_replacement(&doc! { "_id": 1 }).is_err());
        assert!(validate_replacement(&doc! { "$set": { "status": "archived" } }).is_err());
        assert_eq!(target_sort(), doc! { "_id": 1 });
    }
}
