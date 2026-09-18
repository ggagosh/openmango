use rig::tool::{Tool, ToolContext};
use serde::Deserialize;

use super::{MongoContext, ToolError};

/// Search what was said in earlier conversations.
///
/// The current conversation is already in context, so it is excluded: this is for the things the
/// user and the assistant worked out days ago and would otherwise have to repeat.
pub struct RecallConversationsTool(MongoContext);

impl RecallConversationsTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct RecallArgs {
    pub query: String,
}

const MAX_HITS: usize = 5;

impl Tool for RecallConversationsTool {
    const NAME: &'static str = "recall_conversations";
    type Error = ToolError;
    type Args = RecallArgs;
    type Output = serde_json::Value;

    fn description(&self) -> String {
        "Search earlier conversations with this user for something already discussed. Use when \
         the user refers to previous work (\"like we did last time\", \"the query from \
         yesterday\") or when a question implies a decision that was already made. Searches by \
         keyword; the conversation in progress is not included."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords to look for, e.g. a collection or field name"
                }
            },
            "required": ["query"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: RecallArgs,
    ) -> Result<serde_json::Value, ToolError> {
        let Some(memory) = &self.0.memory else {
            return Ok(serde_json::json!({
                "matches": [],
                "note": "Earlier conversations are not being kept on this machine.",
            }));
        };
        let hits = memory
            .search(&args.query, &self.0.conversation_id, MAX_HITS)
            .map_err(|error| ToolError::InvalidInput(error.to_string()))?;

        let matches: Vec<serde_json::Value> = hits
            .into_iter()
            .map(|hit| {
                serde_json::json!({
                    "conversation": hit.conversation_id,
                    "when": chrono::DateTime::from_timestamp_millis(hit.updated_ms)
                        .map(|when| when.format("%Y-%m-%d").to_string())
                        .unwrap_or_default(),
                    "text": hit.text,
                })
            })
            .collect();

        Ok(serde_json::json!({ "query": args.query, "matches": matches }))
    }
}
