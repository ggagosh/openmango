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

/// What one description turned into: the three inputs of a find, as text ready for their boxes.
///
/// Only the filter is always there. A request that says nothing about order or fields leaves
/// those alone rather than inventing them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WrittenQuery {
    pub filter: String,
    pub sort: Option<String>,
    pub projection: Option<String>,
}

impl WrittenQuery {
    /// Whether anything beyond the filter came back, which is what the options row is for.
    pub fn touches_options(&self) -> bool {
        self.sort.is_some() || self.projection.is_some()
    }
}

pub fn placeholder() -> &'static str {
    "e.g. the newest CREATE and UPDATE entries"
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

const RULES: &str = "You turn a description into a MongoDB find for a database GUI.\n\n\
     Reply with one to three labelled lines and nothing else — no prose, no ``` fence:\n\
     FILTER: {…}\n\
     SORT: {…}\n\
     PROJECTION: {…}\n\n\
     - FILTER is always required; {} means everything.\n\
     - Include SORT only if the description asks for an order, and PROJECTION only if it asks \
     for particular fields. Leave a line out rather than guessing at it.\n\
     - SORT maps field names to 1 for ascending or -1 for descending.\n\
     - PROJECTION maps field names to 1 to include or 0 to exclude, never both, except that _id \
     may be excluded alongside included fields.\n\
     - In FILTER use operators such as $gte, $in, $regex and $exists where they fit, and write \
     absolute dates from the date given below.\n\
     - Use only the fields listed. Where a field lists its values, match one of them exactly — \
     they are case sensitive.\n\
     - Quote field names and string values with double quotes. An ObjectId is written \
     ObjectId(\"…\") and a date ISODate(\"…\"); the GUI understands both.";

fn prompt(context: &QueryContext) -> String {
    let fields = if context.fields.is_empty() {
        "(none sampled yet — the collection may be empty)".to_string()
    } else {
        context.fields.join("\n")
    };
    format!(
        "{RULES}\n\nCollection: {}.{}\n\nFields:\n{fields}\n\nToday is {}.",
        context.database,
        context.collection,
        chrono::Local::now().format("%Y-%m-%d"),
    )
}

/// Ask the model to turn a description into a find. The text comes back ready for the inputs.
pub async fn write_query(
    settings: &AiSettings,
    context: &QueryContext,
    description: &str,
) -> Result<WrittenQuery, AiError> {
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
        system_prompt: prompt(context),
        history: Vec::new(),
        user_prompt: description.to_string(),
        conversation_id: String::new(),
        // This is not a conversation and must not be stored as one.
        memory: None,
        context_tokens: None,
        price: None,
    };

    let answer = generate_text(settings, request).await?;
    parse_written(&answer).ok_or_else(|| {
        AiError::Parse(format!("{} did not answer with a query", settings.provider.label()))
    })
}

/// Read the labelled lines back. A model that answers with a bare document meant the filter, and
/// one that wrapped its answer in a fence or added a sentence still gets read.
fn parse_written(answer: &str) -> Option<WrittenQuery> {
    let filter = document_after(answer, "FILTER").or_else(|| take_document(answer))?;
    Some(WrittenQuery {
        filter,
        sort: document_after(answer, "SORT").filter(|document| !is_empty_document(document)),
        projection: document_after(answer, "PROJECTION")
            .filter(|document| !is_empty_document(document)),
    })
}

fn is_empty_document(document: &str) -> bool {
    document.trim_matches(['{', '}', ' ', '\n', '\r', '\t']).is_empty()
}

/// The document following a label, wherever the label sits in the answer.
fn document_after(answer: &str, label: &str) -> Option<String> {
    let at = answer.find(label)?;
    take_document(&answer[at + label.len()..])
}

/// The first balanced `{…}` in `text`, counting braces so a nested document is kept whole and a
/// brace inside a string does not end it early.
fn take_document(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, character) in text[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[start..start + offset + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> QueryContext {
        QueryContext {
            database: "au_new".to_string(),
            collection: "auditlogs".to_string(),
            fields: vec![
                "action: string (CREATE, UPDATE, DELETE)".to_string(),
                "createdAt: date".to_string(),
            ],
        }
    }

    #[test]
    fn a_document_survives_however_the_model_wrapped_it() {
        let bare =
            parse_written("{ \"action\": \"CREATE\" }").expect("a bare document is a filter");
        assert_eq!(bare.filter, "{ \"action\": \"CREATE\" }");
        assert!(!bare.touches_options());

        let fenced =
            parse_written("```json\nFILTER: { \"action\": \"CREATE\" }\n```").expect("fenced");
        assert_eq!(fenced.filter, "{ \"action\": \"CREATE\" }");

        let chatty = parse_written("Sure!\nFILTER: { \"a\": 1 }\nHope that helps").expect("chatty");
        assert_eq!(chatty.filter, "{ \"a\": 1 }");

        assert_eq!(parse_written("I cannot help with that."), None);
    }

    #[test]
    fn a_nested_document_is_kept_whole() {
        let written = parse_written("FILTER: { \"a\": { \"$in\": [1, 2] } }").expect("nested");
        assert_eq!(written.filter, "{ \"a\": { \"$in\": [1, 2] } }");

        // A brace inside a string does not close the document.
        let regex = parse_written("FILTER: { \"a\": { \"$regex\": \"^{x}$\" } }").expect("regex");
        assert_eq!(regex.filter, "{ \"a\": { \"$regex\": \"^{x}$\" } }");
    }

    #[test]
    fn order_and_fields_come_back_only_when_they_were_asked_for() {
        let all = parse_written(
            "FILTER: { \"action\": \"CREATE\" }\nSORT: { \"createdAt\": -1 }\nPROJECTION: { \"action\": 1 }",
        )
        .expect("three lines");
        assert_eq!(all.sort.as_deref(), Some("{ \"createdAt\": -1 }"));
        assert_eq!(all.projection.as_deref(), Some("{ \"action\": 1 }"));
        assert!(all.touches_options(), "the options row has something to show");

        // A model that answers the unasked lines with {} has said nothing: leave them alone.
        let empty =
            parse_written("FILTER: { \"a\": 1 }\nSORT: {}\nPROJECTION: { }").expect("empty");
        assert!(!empty.touches_options());
        assert_eq!((empty.sort, empty.projection), (None, None));
    }

    #[test]
    fn the_prompt_carries_the_collection_its_fields_and_their_values() {
        let prompt = prompt(&context());
        assert!(prompt.contains("au_new.auditlogs"));
        assert!(prompt.contains("action: string (CREATE, UPDATE, DELETE)"));
        assert!(prompt.contains("case sensitive"), "or it writes \"create\" for CREATE");
        assert!(prompt.contains("FILTER:") && prompt.contains("SORT:"));
    }

    #[test]
    fn an_empty_description_is_not_worth_a_request() {
        let error =
            futures::executor::block_on(write_query(&AiSettings::default(), &context(), "   "))
                .expect_err("an empty box asks nothing");
        assert!(
            matches!(&error, AiError::InvalidConfig { field, .. } if field == "description"),
            "it stops on the empty box, before the provider is even checked: {error}"
        );
    }
}
