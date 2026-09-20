//! Describing a filter instead of writing one.
//!
//! The filter bar itself becomes the place you type the description: same row, same input, same
//! button. A second bar under the first one read as a second filter, and its own Find-sized
//! button competed with Find. One bar that changes what it means costs no layout at all.
//!
//! Nothing here runs a query. The written filter lands in the editor, where it can be read,
//! edited, and undone before anyone presses Find.

use std::collections::{BTreeSet, HashMap};

use mongodb::bson::{Bson, Document};

use gpui_kit::{AppContext as _, Context, Entity, Focusable as _, Window};

use crate::ai::bridge::AiBridge;
use crate::ai::inline::{MAX_FIELDS, QueryContext, WrittenQuery, placeholder, write_query};
use crate::state::{AppState, SessionKey};
use crate::views::documents::CollectionView;
use crate::views::documents::query_editor::format_query_editor;

impl CollectionView {
    /// Turn the filter bar into the ask bar, or turn it back and put the filter where it was.
    pub(super) fn set_ask_mode(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.ask_mode == on || self.ask_ai_busy {
            return;
        }
        let Some(input) = self.filter_state.clone() else { return };

        self.ask_mode = on;
        self.ask_ai_error = None;
        // The filter the user had is theirs; asking about something else must not cost it.
        let restore = match on {
            true => {
                self.ask_ai_filter = Some(input.read(cx).value().to_string());
                String::new()
            }
            false => self.ask_ai_filter.take().unwrap_or_default(),
        };

        self.syncing_query_inputs = true;
        input.update(cx, |state, cx| {
            state.set_value(restore.clone(), window, cx);
            state.set_placeholder(filter_placeholder(on), window, cx);
        });
        self.syncing_query_inputs = false;

        self.filter_auto_pair.sync(&restore);
        self.filter_error_message = None;
        self.dismiss_filter_completions(cx);

        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// The menu has no business over prose, and it would eat the Enter that sends it.
    pub(super) fn dismiss_filter_completions(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = &self.filter_completion_menu {
            menu.update(cx, |menu, cx| menu.dismiss(cx));
        }
    }

    /// Send what is in the bar to the model and put the answer back in the same bar.
    pub(super) fn submit_ask_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ask_ai_busy {
            return;
        }
        let Some(input) = self.filter_state.clone() else { return };
        let Some(session_key) = self.view_model.current_session() else { return };
        let description = input.read(cx).value().trim().to_string();
        if description.is_empty() {
            return;
        }

        let settings = self.state.read(cx).settings.ai.clone();
        let context = ask_ai_context(&self.state, &session_key, cx);
        self.ask_ai_busy = true;
        self.ask_ai_error = None;
        cx.notify();

        let task = cx.background_spawn(async move {
            AiBridge::block_on(write_query(&settings, &context, &description))
        });

        cx.spawn_in(window, async move |view, cx| {
            let written = task.await;
            let _ = cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.ask_ai_busy = false;
                    match written {
                        Ok(written) => view.accept_written_query(written, window, cx),
                        Err(error) => view.ask_ai_error = Some(error.user_message()),
                    }
                    cx.notify();
                })
            });
        })
        .detach();
    }

    /// The description becomes the find it described: the filter in the box it was typed in, and
    /// the order and the fields in theirs. The bar goes back to being a filter bar, so the next
    /// thing to press is Find.
    fn accept_written_query(
        &mut self,
        written: WrittenQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self.filter_state.clone() else { return };
        self.ask_mode = false;
        // What it replaced was the description, not a filter worth restoring.
        self.ask_ai_filter = None;

        let touches_options = written.touches_options();
        self.syncing_query_inputs = true;
        input.update(cx, |state, cx| {
            state.replace_all(written.filter, window, cx);
            state.set_placeholder(filter_placeholder(false), window, cx);
        });
        let formatted = format_query_editor(&input, window, cx);

        if let Some(sort) = written.sort.clone()
            && let Some(state) = self.sort_state.clone()
        {
            state.update(cx, |state, cx| state.replace_all(sort.clone(), window, cx));
            self.sort_auto_pair.sync(&sort);
            self.sort_error = super::query::query_validation_error(&sort).is_some();
        }
        if let Some(projection) = written.projection.clone()
            && let Some(state) = self.projection_state.clone()
        {
            state.update(cx, |state, cx| state.replace_all(projection.clone(), window, cx));
            self.projection_auto_pair.sync(&projection);
            self.projection_error = super::query::query_validation_error(&projection).is_some();
        }
        self.syncing_query_inputs = false;

        // An order or a field list that nobody can see is one nobody asked to run. Open the row
        // that holds them so the whole query is on screen before Find is pressed.
        if touches_options && let Some(session_key) = self.view_model.current_session() {
            self.state.update(cx, |state, cx| {
                state.set_query_options_open(&session_key, true);
                cx.notify();
            });
        }

        self.filter_auto_pair.sync(&formatted);
        self.filter_error_message = super::query::filter_query_validation_error(&formatted);
        self.filter_dirty = true;
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }
}

fn filter_placeholder(ask_mode: bool) -> &'static str {
    match ask_mode {
        true => placeholder(),
        false => "Filter documents…",
    }
}

/// What the model is told about the collection: its name, the fields it has, and — for the
/// fields that only ever hold a handful of strings — which strings those are.
///
/// The documents on screen are the source, not the sampled schema: the schema is only there
/// once something has asked for it, and a model told nothing about the collection writes
/// `"create"` against a field whose values are `CREATE`.
fn ask_ai_context(
    state: &Entity<AppState>,
    session_key: &SessionKey,
    cx: &gpui_kit::App,
) -> QueryContext {
    let state = state.read(cx);
    let mut fields =
        state.session_data(session_key).map(|data| field_lines(&data.items)).unwrap_or_default();

    // An empty page still has a shape, if anything has sampled it.
    if fields.is_empty() {
        let schema = state
            .collection_meta(&session_key.collection_key())
            .map(|meta| meta.schema.fields.clone())
            .or_else(|| state.session(session_key)?.data.schema.as_ref().map(|s| s.fields.clone()));
        if let Some(schema) = schema {
            collect_schema_lines(&schema, &mut fields);
        }
    }
    fields.truncate(MAX_FIELDS);

    QueryContext {
        database: session_key.database.clone(),
        collection: session_key.collection.clone(),
        fields,
    }
}

/// Values a field is worth listing. Past this it is free text, not a set of choices, and the
/// list would only cost tokens.
const MAX_VALUES: usize = 8;
/// Strings only, and only short ones: a message body is not a choice.
const MAX_VALUE_CHARS: usize = 40;
const MAX_VALUE_DEPTH: usize = 2;

/// What one field looked like across the page.
#[derive(Default)]
struct FieldFacts {
    /// Where it first appeared, so parents come before children and the order reads like a
    /// document rather than an alphabet.
    order: usize,
    types: BTreeSet<&'static str>,
    values: BTreeSet<String>,
    /// More distinct values than anyone would call a choice.
    open_ended: bool,
}

/// "action: string (CREATE, UPDATE, DELETE)" for every field the loaded page shows.
fn field_lines(documents: &[crate::state::SessionDocument]) -> Vec<String> {
    let mut facts: HashMap<String, FieldFacts> = HashMap::new();
    for document in documents {
        collect_facts(&document.doc, "", 0, &mut facts);
    }

    let mut ordered: Vec<(String, FieldFacts)> = facts.into_iter().collect();
    ordered.sort_by_key(|(_, facts)| facts.order);
    ordered
        .into_iter()
        .map(|(path, facts)| {
            let mut line = match facts.types.is_empty() {
                true => path,
                false => {
                    format!(
                        "{path}: {}",
                        facts.types.iter().copied().collect::<Vec<_>>().join(" | ")
                    )
                }
            };
            // "e.g." and not a bare list: these are the values this page happened to show, and
            // a model that reads them as the whole set drops the one the user asked for.
            if !facts.open_ended && !facts.values.is_empty() {
                line.push_str(&format!(
                    " e.g. {}",
                    facts.values.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
            line
        })
        .collect()
}

fn collect_facts(
    document: &Document,
    prefix: &str,
    depth: usize,
    out: &mut HashMap<String, FieldFacts>,
) {
    for (key, value) in document {
        let path = match prefix.is_empty() {
            true => key.clone(),
            false => format!("{prefix}.{key}"),
        };
        let next = out.len();
        let facts = out
            .entry(path.clone())
            .or_insert_with(|| FieldFacts { order: next, ..FieldFacts::default() });
        facts.types.insert(type_name(value));

        match value {
            Bson::String(text) if text.chars().count() <= MAX_VALUE_CHARS => {
                if !facts.open_ended {
                    facts.values.insert(text.clone());
                    if facts.values.len() > MAX_VALUES {
                        // Once it is over the cap it stays over it: no later value makes it a set.
                        facts.open_ended = true;
                        facts.values.clear();
                    }
                }
            }
            Bson::String(_) => facts.open_ended = true,
            Bson::Document(nested) if depth < MAX_VALUE_DEPTH => {
                collect_facts(nested, &path, depth + 1, out);
            }
            _ => {}
        }
    }
}

fn type_name(value: &Bson) -> &'static str {
    match value {
        Bson::Double(_) | Bson::Int32(_) | Bson::Int64(_) | Bson::Decimal128(_) => "number",
        Bson::String(_) => "string",
        Bson::Boolean(_) => "boolean",
        Bson::DateTime(_) => "date",
        Bson::ObjectId(_) => "objectId",
        Bson::Array(_) => "array",
        Bson::Document(_) => "object",
        Bson::Null => "null",
        _ => "value",
    }
}

/// The same lines from a sampled schema, for when the page itself is empty.
fn collect_schema_lines(fields: &[crate::state::SchemaField], out: &mut Vec<String>) {
    for field in fields {
        if out.len() >= MAX_FIELDS {
            return;
        }
        let types: Vec<&str> =
            field.types.iter().map(|kind| kind.bson_type.as_str()).take(3).collect();
        out.push(match types.is_empty() {
            true => field.path.clone(),
            false => format!("{}: {}", field.path, types.join(" | ")),
        });
        collect_schema_lines(&field.children, out);
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::Document;

    use super::{field_lines, filter_placeholder};
    use crate::bson::DocumentKey;
    use crate::state::SessionDocument;

    fn document(pairs: &[(&str, &str)]) -> SessionDocument {
        let mut doc = Document::new();
        for (key, value) in pairs {
            doc.insert(*key, *value);
        }
        SessionDocument { key: DocumentKey::from_document(&doc, 0), doc }
    }

    /// "create and update documents" became `$in: ["create", "update"]` against a collection
    /// whose values are `CREATE` and `UPDATE`. The model was never told what was in there.
    #[test]
    fn a_field_that_holds_a_few_values_lists_them() {
        // A page of documents: two actions over and over, and a message that never repeats.
        let documents: Vec<_> = (0..20)
            .map(|index| {
                let action = if index % 2 == 0 { "CREATE" } else { "UPDATE" };
                document(&[("action", action), ("message", &format!("line number {index}"))])
            })
            .collect();

        assert_eq!(
            field_lines(&documents),
            ["action: string e.g. CREATE, UPDATE", "message: string"],
            "the choices are named; the prose is not"
        );
    }

    /// The schema is only sampled once something asks for it. The page is always there, and
    /// without this the model was told nothing at all about the collection.
    #[test]
    fn the_fields_come_from_the_page_without_any_schema() {
        let mut doc = Document::new();
        doc.insert("logId", "DOC-C-1");
        doc.insert("version", 3i32);
        let mut diff = Document::new();
        diff.insert("before", "x");
        doc.insert("diff", diff);
        let page = [SessionDocument { key: DocumentKey::from_document(&doc, 0), doc }];

        assert_eq!(
            field_lines(&page),
            [
                "logId: string e.g. DOC-C-1",
                "version: number",
                "diff: object",
                "diff.before: string e.g. x",
            ],
            "parents before children, in the order the document lists them"
        );
    }

    #[test]
    fn a_field_that_holds_two_kinds_names_both() {
        let mut first = Document::new();
        first.insert("value", "text");
        let mut second = Document::new();
        second.insert("value", 7i32);
        let page = [
            SessionDocument { key: DocumentKey::from_document(&first, 0), doc: first },
            SessionDocument { key: DocumentKey::from_document(&second, 1), doc: second },
        ];
        assert_eq!(field_lines(&page), ["value: number | string e.g. text"]);
    }

    #[test]
    fn the_bar_says_which_of_the_two_things_it_is() {
        assert_ne!(filter_placeholder(true), filter_placeholder(false));
        assert!(filter_placeholder(true).contains("e.g."), "the ask mode teaches what to type");
    }
}
