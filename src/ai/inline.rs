//! Writing a query from a description, in place.
//!
//! The chat panel is for working a question through. This is the other half: the user is already
//! in the filter box, knows what they want, and does not know the syntax. One request, one
//! document back, written into the input they were looking at — where they can read it, edit it,
//! and undo it.
//!
//! Nothing here runs a query or touches the database. The model is given field names and types
//! and asked for a document; the caller decides what to do with it.

use crate::ai::errors::AiError;
use crate::ai::provider::{AiGenerationRequest, generate_text};
use crate::ai::settings::AiSettings;

/// Which of the query inputs is being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryInput {
    Filter,
    Sort,
    Projection,
}

impl QueryInput {
    pub fn label(self) -> &'static str {
        match self {
            Self::Filter => "filter",
            Self::Sort => "sort",
            Self::Projection => "projection",
        }
    }

    /// What the model is asked for, and the shape it has to come back in.
    fn instruction(self) -> &'static str {
        match self {
            Self::Filter => {
                "Write a MongoDB query filter document that finds what the user describes. Use \
                 operators such as $gte, $in, $regex and $exists where they fit. For a relative \
                 date, write the absolute value the date section gives you."
            }
            Self::Sort => {
                "Write a MongoDB sort document: field names mapped to 1 for ascending or -1 for \
                 descending, in the order they should apply."
            }
            Self::Projection => {
                "Write a MongoDB projection document: field names mapped to 1 to include them, or \
                 to 0 to exclude them. Do not mix the two, except for _id, which may be excluded \
                 alongside included fields."
            }
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Filter => "e.g. created this week and action is CREATE",
            Self::Sort => "Describe the order…",
            Self::Projection => "Describe which fields to keep…",
        }
    }
}

/// The collection the query is for, as the model needs to see it.
pub struct QueryContext {
    pub database: String,
    pub collection: String,
    /// `path: type` lines, most common field first.
    pub fields: Vec<String>,
}

/// Fields worth showing the model. More than this and the prompt costs more than the answer.
pub const MAX_FIELDS: usize = 60;

const RULES: &str = "You write MongoDB queries for a database GUI.\n\n\
     - Reply with one JSON document and nothing else: no prose, no explanation, no ``` fence.\n\
     - Use only the fields listed below. If the request names something that is not there, use \
     the closest field that is.\n\
     - Quote field names and string values with double quotes.\n\
     - An ObjectId is written ObjectId(\"…\"), a date ISODate(\"…\"); the GUI understands both.\n\
     - An empty request is the empty document: {}";

fn prompt(kind: QueryInput, context: &QueryContext) -> String {
    let fields = if context.fields.is_empty() {
        "(none sampled yet — the collection may be empty)".to_string()
    } else {
        context.fields.join("\n")
    };
    format!(
        "{RULES}\n\n{}\n\nCollection: {}.{}\n\nFields:\n{fields}\n\nToday is {}.",
        kind.instruction(),
        context.database,
        context.collection,
        chrono::Local::now().format("%Y-%m-%d"),
    )
}

/// Ask the model for one query document. The text comes back ready to put in the input.
pub async fn write_query(
    settings: &AiSettings,
    kind: QueryInput,
    context: &QueryContext,
    description: &str,
) -> Result<String, AiError> {
    // Cheapest check first: an empty box is not worth validating settings for, let alone a call.
    let description = description.trim();
    if description.is_empty() {
        return Err(AiError::InvalidConfig {
            field: "description".to_string(),
            message: "say what you are looking for".to_string(),
        });
    }
    settings.validate_for_request()?;

    let request = AiGenerationRequest {
        system_prompt: prompt(kind, context),
        history: Vec::new(),
        user_prompt: description.to_string(),
        conversation_id: String::new(),
        // This is not a conversation and must not be stored as one.
        memory: None,
        context_tokens: None,
        price: None,
    };

    let answer = generate_text(settings, request).await?;
    unfence(&answer).ok_or_else(|| {
        AiError::Parse(format!("{} did not answer with a query", settings.provider.label()))
    })
}

/// Models wrap a document in a ``` fence about half the time, and sometimes add a sentence after
/// it. Take what is between the outermost braces.
fn unfence(answer: &str) -> Option<String> {
    let start = answer.find('{')?;
    let end = answer.rfind('}')?;
    if end < start {
        return None;
    }
    let document = answer[start..=end].trim();
    (!document.is_empty()).then(|| document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> QueryContext {
        QueryContext {
            database: "au_new".to_string(),
            collection: "auditlogs".to_string(),
            fields: vec!["action: string".to_string(), "createdAt: date".to_string()],
        }
    }

    #[test]
    fn a_document_survives_however_the_model_wrapped_it() {
        assert_eq!(
            unfence("{ \"action\": \"CREATE\" }").as_deref(),
            Some("{ \"action\": \"CREATE\" }")
        );
        assert_eq!(
            unfence("```json\n{ \"action\": \"CREATE\" }\n```").as_deref(),
            Some("{ \"action\": \"CREATE\" }")
        );
        assert_eq!(
            unfence("Here you go:\n{ \"action\": \"CREATE\" }\nLet me know!").as_deref(),
            Some("{ \"action\": \"CREATE\" }")
        );
        assert_eq!(
            unfence("{ \"a\": { \"$in\": [1, 2] } }").as_deref(),
            Some("{ \"a\": { \"$in\": [1, 2] } }"),
            "the outermost braces, not the first pair"
        );
        assert_eq!(unfence("I cannot help with that."), None);
        assert_eq!(unfence("}{"), None);
    }

    #[test]
    fn the_prompt_carries_the_collection_and_its_fields() {
        let prompt = prompt(QueryInput::Filter, &context());
        assert!(prompt.contains("au_new.auditlogs"));
        assert!(prompt.contains("createdAt: date"));
        assert!(prompt.contains("$gte"), "the filter instruction, not the sort one");
        assert!(!prompt.contains("ascending"));

        let sort = prompt_for(QueryInput::Sort);
        assert!(sort.contains("ascending") && !sort.contains("$gte"));
    }

    fn prompt_for(kind: QueryInput) -> String {
        prompt(kind, &context())
    }

    #[test]
    fn an_empty_description_is_not_worth_a_request() {
        let error = futures::executor::block_on(write_query(
            &AiSettings::default(),
            QueryInput::Filter,
            &context(),
            "   ",
        ))
        .expect_err("an empty box asks nothing");
        assert!(
            matches!(&error, AiError::InvalidConfig { field, .. } if field == "description"),
            "it stops on the empty box, before the provider is even checked: {error}"
        );
    }
}
