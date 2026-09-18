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
fn ask_ai_context(
    state: &Entity<AppState>,
    session_key: &SessionKey,
    cx: &gpui_kit::App,
) -> QueryContext {
    let state = state.read(cx);
    let values =
        state.session_data(session_key).map(|data| sample_values(&data.items)).unwrap_or_default();

    let mut fields = Vec::new();
    if let Some(meta) = state.collection_meta(session_key) {
        collect_field_lines(&meta.schema.fields, &values, &mut fields);
    }
    if fields.is_empty()
        && let Some(session) = state.session(session_key)
        && let Some(schema) = session.data.schema.as_ref()
    {
        collect_field_lines(&schema.fields, &values, &mut fields);
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

/// "action: string (CREATE, UPDATE, DELETE)" — parents before children, as the schema sampled
/// them, with the values the loaded page shows where the field looks like a set of choices.
fn collect_field_lines(
    fields: &[crate::state::SchemaField],
    values: &HashMap<String, BTreeSet<String>>,
    out: &mut Vec<String>,
) {
    for field in fields {
        if out.len() >= MAX_FIELDS {
            return;
        }
        let types: Vec<&str> =
            field.types.iter().map(|kind| kind.bson_type.as_str()).take(3).collect();
        let mut line = match types.is_empty() {
            true => field.path.clone(),
            false => format!("{}: {}", field.path, types.join(" | ")),
        };
        if let Some(choices) = values.get(&field.path)
            && choices.len() <= MAX_VALUES
        {
            line.push_str(&format!(
                " ({})",
                choices.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        out.push(line);
        collect_field_lines(&field.children, values, out);
    }
}

/// The distinct strings each field holds across the loaded page.
///
/// This is what stops "create and update documents" becoming `$in: ["create", "update"]` against
/// a collection whose values are `CREATE` and `UPDATE`. A field is given up on once it passes
/// the cap: it is prose, and no list of examples would help.
fn sample_values(documents: &[crate::state::SessionDocument]) -> HashMap<String, BTreeSet<String>> {
    let mut values: HashMap<String, BTreeSet<String>> = HashMap::new();
    for document in documents {
        collect_values(&document.doc, "", 0, &mut values);
    }
    values.retain(|_, seen| seen.len() <= MAX_VALUES);
    values
}

/// Strings only, and only short ones: a message body is not a choice.
const MAX_VALUE_CHARS: usize = 40;
const MAX_VALUE_DEPTH: usize = 2;

fn collect_values(
    document: &Document,
    prefix: &str,
    depth: usize,
    out: &mut HashMap<String, BTreeSet<String>>,
) {
    for (key, value) in document {
        let path = match prefix.is_empty() {
            true => key.clone(),
            false => format!("{prefix}.{key}"),
        };
        match value {
            Bson::String(text) if text.chars().count() <= MAX_VALUE_CHARS => {
                let seen = out.entry(path).or_default();
                // Once it is over the cap it stays over it: one more value cannot make it a set.
                if seen.len() <= MAX_VALUES {
                    seen.insert(text.clone());
                }
            }
            Bson::Document(nested) if depth < MAX_VALUE_DEPTH => {
                collect_values(nested, &path, depth + 1, out);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{collect_field_lines, filter_placeholder};
    use mongodb::bson::Document;

    use crate::bson::DocumentKey;
    use crate::state::{SchemaField, SchemaFieldType, SessionDocument};

    fn document(pairs: &[(&str, &str)]) -> SessionDocument {
        let mut doc = Document::new();
        for (key, value) in pairs {
            doc.insert(*key, *value);
        }
        SessionDocument { key: DocumentKey::from_document(&doc, 0), doc }
    }

    fn field(path: &str, kind: &str, children: Vec<SchemaField>) -> SchemaField {
        SchemaField {
            path: path.to_string(),
            name: path.rsplit('.').next().unwrap_or(path).to_string(),
            depth: path.matches('.').count(),
            types: vec![SchemaFieldType {
                bson_type: kind.to_string(),
                count: 1,
                percentage: 100.0,
            }],
            presence: 1,
            null_count: 0,
            is_polymorphic: false,
            children,
        }
    }

    #[test]
    fn the_model_is_told_the_nested_fields_too() {
        let schema = vec![
            field("action", "string", Vec::new()),
            field("diff", "object", vec![field("diff.before", "object", Vec::new())]),
        ];
        let mut lines = Vec::new();
        collect_field_lines(&schema, &HashMap::new(), &mut lines);
        assert_eq!(lines, ["action: string", "diff: object", "diff.before: object"]);
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
        let values = super::sample_values(&documents);

        let mut lines = Vec::new();
        collect_field_lines(
            &[field("action", "string", Vec::new()), field("message", "string", Vec::new())],
            &values,
            &mut lines,
        );
        assert_eq!(
            lines,
            ["action: string (CREATE, UPDATE)", "message: string"],
            "the choices are named; the prose is not, and its three values are not choices"
        );
    }

    #[test]
    fn a_field_with_too_many_values_is_left_alone() {
        let documents: Vec<_> = (0..super::MAX_VALUES + 2)
            .map(|index| {
                let mut doc = Document::new();
                doc.insert("id", format!("id-{index}"));
                SessionDocument { key: DocumentKey::from_document(&doc, index), doc }
            })
            .collect();
        assert!(
            !super::sample_values(&documents).contains_key("id"),
            "past the cap it is an identifier, not a set of choices"
        );
    }

    #[test]
    fn a_polymorphic_field_names_what_it_holds() {
        let mut polymorphic = field("value", "string", Vec::new());
        polymorphic.types.push(SchemaFieldType {
            bson_type: "int".to_string(),
            count: 1,
            percentage: 50.0,
        });
        let mut lines = Vec::new();
        collect_field_lines(&[polymorphic], &HashMap::new(), &mut lines);
        assert_eq!(lines, ["value: string | int"]);
    }

    #[test]
    fn the_bar_says_which_of_the_two_things_it_is() {
        assert_ne!(filter_placeholder(true), filter_placeholder(false));
        assert!(filter_placeholder(true).contains("e.g."), "the ask mode teaches what to type");
    }
}
