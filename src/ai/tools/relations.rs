//! What the relation graph knows, for the agent.
//!
//! Without these the model has to guess which collection a `userId` points at, or spend a turn
//! sampling to find out. The graph already knows, and says so in a line per collection rather
//! than a JSON object per relation: about a tenth of the tokens, for the same facts.

use rig::tool::{Tool, ToolContext};
use serde::Deserialize;

use super::{MongoContext, ToolError};
use crate::state::relations::export::{compact, describe_steps, lookup_stages};
use crate::state::relations::resolve::NAVIGATION_CONFIDENCE;

pub struct GetRelationsTool(MongoContext);

impl GetRelationsTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct GetRelationsArgs {
    #[serde(default)]
    collection: Option<String>,
}

impl Tool for GetRelationsTool {
    const NAME: &'static str = "get_relations";
    type Error = ToolError;
    type Args = GetRelationsArgs;
    type Output = String;

    fn description(&self) -> String {
        "Which fields reference which collections in the current database. One line per \
         collection, fields grouped under what they point at: \
         `orders: users<buyerId,sellerId; products<items[].productId`. `[]` marks an array and \
         every target is an `_id`. With `collection`, returns that collection's line plus a `<-` \
         line of the fields elsewhere that point at it. Call this before writing a $lookup or \
         resolving an ObjectId by hand."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "collection": {
                    "type": "string",
                    "description": "Limit to one collection and what points at it. Omit for the whole database."
                }
            }
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: GetRelationsArgs,
    ) -> Result<String, ToolError> {
        Ok(compact(&self.0.relations, &self.0.database, args.collection.as_deref()))
    }
}

pub struct JoinPathTool(MongoContext);

impl JoinPathTool {
    pub fn new(ctx: MongoContext) -> Self {
        Self(ctx)
    }
}

#[derive(Deserialize)]
pub struct JoinPathArgs {
    from: String,
    to: String,
}

impl Tool for JoinPathTool {
    const NAME: &'static str = "join_path";
    type Error = ToolError;
    type Args = JoinPathArgs;
    type Output = serde_json::Value;

    fn description(&self) -> String {
        "The shortest chain of references joining two collections, and the $lookup stages that \
         follow it, ready to put in an `aggregate` pipeline run on `from`."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "The collection the pipeline runs on." },
                "to": { "type": "string", "description": "The collection to reach." }
            },
            "required": ["from", "to"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: JoinPathArgs,
    ) -> Result<serde_json::Value, ToolError> {
        let database = self.0.database.as_str();
        let steps = self
            .0
            .relations
            .join_path((database, &args.from), (database, &args.to), NAVIGATION_CONFIDENCE)
            .ok_or_else(|| {
                ToolError::InvalidInput(format!(
                    "No known chain of references joins {} to {}. Call get_relations to see what is known.",
                    args.from, args.to
                ))
            })?;
        Ok(serde_json::json!({
            "path": describe_steps(&steps),
            "pipeline": lookup_stages(&steps),
        }))
    }
}
