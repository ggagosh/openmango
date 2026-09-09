use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use mongodb::bson::Document;

use crate::bson::parse_document_from_json;
use crate::state::{AppCommands, AppState, SessionKey, StatusMessage};

use super::CollectionView;
use super::fast_filter::{compile_filter_input, format_compiled_filter};
use super::query_editor::format_query_editor;

impl CollectionView {
    pub(super) fn apply_filter(
        state: Entity<AppState>,
        session_key: SessionKey,
        filter_state: Entity<EditorState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let raw = filter_state.read(cx).value().to_string();

        match compile_filter_input(&raw) {
            Ok(compiled) => {
                let raw = format_query_editor(&filter_state, window, cx);
                let Some(filter) = compiled.document else {
                    state.update(cx, |state, cx| {
                        state.clear_filter(&session_key);
                        state.set_status_message(Some(StatusMessage::info("Filter cleared")));
                        cx.notify();
                    });
                    AppCommands::load_documents_for_session(state.clone(), session_key, cx);
                    return;
                };
                let raw_store = raw.trim().to_string();
                state.update(cx, |state, cx| {
                    state.set_filter(&session_key, raw_store.clone(), Some(filter));
                    state.set_status_message(Some(StatusMessage::info("Filter applied")));
                    cx.notify();
                });
                AppCommands::load_documents_for_session(state.clone(), session_key, cx);
            }
            Err(err) => {
                state.update(cx, |state, cx| {
                    state.set_status_message(Some(StatusMessage::error(format!(
                        "Invalid filter: {err}"
                    ))));
                    cx.notify();
                });
            }
        }
    }

    pub(super) fn apply_query_options(
        state: Entity<AppState>,
        session_key: SessionKey,
        sort_state: Entity<EditorState>,
        projection_state: Entity<EditorState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let sort_raw = sort_state.read(cx).value().to_string();
        let projection_raw = projection_state.read(cx).value().to_string();

        let (mut sort_raw_store, sort_doc) = match parse_optional_doc(&sort_raw) {
            Ok(result) => result,
            Err(err) => {
                state.update(cx, |state, cx| {
                    state.set_status_message(Some(StatusMessage::error(format!(
                        "Invalid sort JSON: {err}"
                    ))));
                    cx.notify();
                });
                return;
            }
        };

        let (mut projection_raw_store, projection_doc) = match parse_optional_doc(&projection_raw) {
            Ok(result) => result,
            Err(err) => {
                state.update(cx, |state, cx| {
                    state.set_status_message(Some(StatusMessage::error(format!(
                        "Invalid projection JSON: {err}"
                    ))));
                    cx.notify();
                });
                return;
            }
        };

        let sort_formatted = format_query_editor(&sort_state, window, cx);
        let projection_formatted = format_query_editor(&projection_state, window, cx);
        if sort_doc.is_some() {
            sort_raw_store = sort_formatted.trim().to_string();
        }
        if projection_doc.is_some() {
            projection_raw_store = projection_formatted.trim().to_string();
        }

        let message = if sort_doc.is_none() && projection_doc.is_none() {
            "Sort/projection cleared"
        } else {
            "Sort/projection applied"
        };

        state.update(cx, |state, cx| {
            state.set_sort_projection(
                &session_key,
                sort_raw_store,
                sort_doc,
                projection_raw_store,
                projection_doc,
            );
            state.set_status_message(Some(StatusMessage::info(message)));
            cx.notify();
        });
        AppCommands::load_documents_for_session(state.clone(), session_key, cx);
    }
}

pub(super) fn normalized_filter_query(raw: &str) -> String {
    if let Ok(compiled) = compile_filter_input(raw) {
        return format_compiled_filter(&compiled);
    }

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "{}".to_string();
    }
    if trimmed.starts_with('{') {
        return trimmed.to_string();
    }
    format!("{{{trimmed}}}")
}

/// Drafts are persisted on every edit; only the empty-object shorthand is normalized.
pub(super) fn query_drafts_equal(current: &str, stored: &str) -> bool {
    let normalize = |text: &str| matches!(text.trim(), "" | "{}");
    current == stored || (normalize(current) && normalize(stored))
}

pub(super) fn document_empty_message(loaded: bool, failed: bool, filtered: bool) -> &'static str {
    if failed {
        "No results available"
    } else if !loaded {
        "No results yet"
    } else if filtered {
        "No documents match this filter"
    } else {
        "This collection is empty"
    }
}

pub(super) fn format_filter_query(raw: &str) -> Result<String, String> {
    compile_filter_input(raw)
        .map(|compiled| format_compiled_filter(&compiled))
        .map_err(|err| err.to_string())
}

/// Return a user-facing validation error for a query string, if any.
pub(super) fn query_validation_error(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return None;
    }

    parse_document_from_json(trimmed).err().map(|err| err.to_string())
}

pub(super) fn filter_query_validation_error(raw: &str) -> Option<String> {
    compile_filter_input(raw)
        .err()
        .and_then(|err| if err.is_incomplete() { None } else { Some(err.to_string()) })
}

pub(super) fn strict_filter_query_validation_error(raw: &str) -> Option<String> {
    compile_filter_input(raw).err().map(|err| err.to_string())
}

/// Check if a query string is valid (empty, `{}`, or parseable as a document).
pub(super) fn is_valid_query(raw: &str) -> bool {
    query_validation_error(raw).is_none()
}

pub(super) fn optional_query_validation_error(raw: &str) -> Option<String> {
    query_validation_error(raw)
}

fn parse_optional_doc(raw: &str) -> Result<(String, Option<Document>), String> {
    let trimmed = raw.trim();
    match optional_query_validation_error(trimmed) {
        None if trimmed.is_empty() || trimmed == "{}" => Ok((String::new(), None)),
        None => parse_document_from_json(trimmed)
            .map(|doc| (trimmed.to_string(), Some(doc)))
            .map_err(|err| err.to_string()),
        Some(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        filter_query_validation_error, format_filter_query, is_valid_query,
        normalized_filter_query, optional_query_validation_error, parse_optional_doc,
        query_validation_error,
    };

    #[test]
    fn is_valid_query_accepts_empty_and_braces() {
        assert!(is_valid_query(""));
        assert!(is_valid_query("   "));
        assert!(is_valid_query("{}"));
    }

    #[test]
    fn draft_sync_preserves_empty_braces_and_detects_restored_multiline_queries() {
        assert!(super::query_drafts_equal("{}", ""));
        assert!(super::query_drafts_equal("  {} ", " "));
        assert!(super::query_drafts_equal("{\n  age: 30\n}", "{\n  age: 30\n}"));
        assert!(!super::query_drafts_equal("{ age: 30 }", "{ age: 40 }"));
    }

    #[test]
    fn empty_states_distinguish_failed_queries_from_no_matches() {
        assert_eq!(super::document_empty_message(false, false, false), "No results yet");
        assert_eq!(super::document_empty_message(true, true, true), "No results available");
        assert_eq!(
            super::document_empty_message(true, false, true),
            "No documents match this filter"
        );
        assert_eq!(super::document_empty_message(true, false, false), "This collection is empty");
    }

    #[test]
    fn parse_optional_doc_returns_none_for_empty_queries() {
        assert_eq!(
            parse_optional_doc("").expect("empty query should parse"),
            (String::new(), None)
        );
        assert_eq!(
            parse_optional_doc("  {} ").expect("brace query should parse"),
            (String::new(), None)
        );
    }

    #[test]
    fn parse_optional_doc_trims_and_parses_document() {
        let (raw, doc) = parse_optional_doc("  {\"x\": 1}  ").expect("document query should parse");
        assert_eq!(raw, "{\"x\": 1}");
        assert!(doc.is_some());
    }

    #[test]
    fn query_validation_error_reports_invalid_query() {
        let err = query_validation_error("{\"x\":").expect("invalid query should return an error");
        assert!(!err.is_empty());
    }

    #[test]
    fn optional_query_validation_error_accepts_empty_queries() {
        assert!(optional_query_validation_error("").is_none());
        assert!(optional_query_validation_error(" {} ").is_none());
    }

    #[test]
    fn normalized_filter_query_wraps_missing_braces() {
        assert_eq!(normalized_filter_query("name: \"alice\""), "{name: \"alice\"}");
        assert_eq!(normalized_filter_query(""), "{}");
    }

    #[test]
    fn filter_query_validation_accepts_missing_outer_braces() {
        assert!(filter_query_validation_error("name: \"alice\"").is_none());
    }

    #[test]
    fn format_filter_query_wraps_and_formats() {
        assert_eq!(
            format_filter_query("name:\"alice\",age:1").expect("format"),
            "{name: \"alice\", age: 1}"
        );
    }
}
