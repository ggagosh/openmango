//! Describing a filter instead of writing one.
//!
//! The filter bar itself becomes the place you type the description: same row, same input, same
//! button. A second bar under the first one read as a second filter, and its own Find-sized
//! button competed with Find. One bar that changes what it means costs no layout at all.
//!
//! Nothing here runs a query. The written filter lands in the editor, where it can be read,
//! edited, and undone before anyone presses Find.

use gpui_kit::{AppContext as _, Context, Entity, Focusable as _, Window};

use crate::ai::bridge::AiBridge;
use crate::ai::inline::{MAX_FIELDS, QueryContext, QueryInput, write_query};
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
            state.set_placeholder(placeholder(on), window, cx);
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
            AiBridge::block_on(write_query(&settings, QueryInput::Filter, &context, &description))
        });

        cx.spawn_in(window, async move |view, cx| {
            let written = task.await;
            let _ = cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.ask_ai_busy = false;
                    match written {
                        Ok(filter) => view.accept_written_filter(filter, window, cx),
                        Err(error) => view.ask_ai_error = Some(error.user_message()),
                    }
                    cx.notify();
                })
            });
        })
        .detach();
    }

    /// The description becomes the filter it described, in the box it was typed in. The bar goes
    /// back to being a filter bar, so the next thing to press is Find.
    fn accept_written_filter(
        &mut self,
        filter: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self.filter_state.clone() else { return };
        self.ask_mode = false;
        // What it replaced was the description, not a filter worth restoring.
        self.ask_ai_filter = None;

        self.syncing_query_inputs = true;
        input.update(cx, |state, cx| {
            state.replace_all(filter, window, cx);
            state.set_placeholder(placeholder(false), window, cx);
        });
        let formatted = format_query_editor(&input, window, cx);
        self.syncing_query_inputs = false;

        self.filter_auto_pair.sync(&formatted);
        self.filter_error_message = super::query::filter_query_validation_error(&formatted);
        self.filter_dirty = true;
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }
}

fn placeholder(ask_mode: bool) -> &'static str {
    match ask_mode {
        true => QueryInput::Filter.placeholder(),
        false => "Filter documents…",
    }
}

/// What the model is told about the collection: its name, and the fields it actually has.
fn ask_ai_context(
    state: &Entity<AppState>,
    session_key: &SessionKey,
    cx: &gpui_kit::App,
) -> QueryContext {
    let state = state.read(cx);
    let mut fields = Vec::new();
    if let Some(meta) = state.collection_meta(session_key) {
        collect_field_lines(&meta.schema.fields, &mut fields);
    }
    if fields.is_empty()
        && let Some(session) = state.session(session_key)
        && let Some(schema) = session.data.schema.as_ref()
    {
        collect_field_lines(&schema.fields, &mut fields);
    }
    fields.truncate(MAX_FIELDS);

    QueryContext {
        database: session_key.database.clone(),
        collection: session_key.collection.clone(),
        fields,
    }
}

/// "user.name: objectId" for every field, parents before children, as the schema sampled them.
fn collect_field_lines(fields: &[crate::state::SchemaField], out: &mut Vec<String>) {
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
        collect_field_lines(&field.children, out);
    }
}

#[cfg(test)]
mod tests {
    use super::{collect_field_lines, placeholder};
    use crate::state::{SchemaField, SchemaFieldType};

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
        collect_field_lines(&schema, &mut lines);
        assert_eq!(lines, ["action: string", "diff: object", "diff.before: object"]);
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
        collect_field_lines(&[polymorphic], &mut lines);
        assert_eq!(lines, ["value: string | int"]);
    }

    #[test]
    fn the_bar_says_which_of_the_two_things_it_is() {
        assert_ne!(placeholder(true), placeholder(false));
        assert!(placeholder(true).contains("e.g."), "the ask mode teaches what to type");
    }
}
