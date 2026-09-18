//! "Ask AI" for the query inputs.
//!
//! The chat panel answers questions. This answers one: what does this filter look like. The
//! description goes in a box under the filter, the document comes back into the filter itself,
//! and the user reads it before running anything — nothing here touches the database.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{App, AppContext as _, Context, Entity, Focusable as _, Window};

use crate::ai::bridge::AiBridge;
use crate::ai::inline::{MAX_FIELDS, QueryContext, QueryInput, write_query};
use crate::state::{AppState, SessionKey};
use crate::views::documents::CollectionView;
use crate::views::documents::query_editor::format_query_editor;

impl CollectionView {
    pub(super) fn ensure_ask_ai_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(state) = self.ask_ai_state.clone() {
            return state;
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(QueryInput::Filter.placeholder()));
        let subscription =
            cx.subscribe_in(&input, window, move |view, _input, event, window, cx| match event {
                InputEvent::PressEnter { .. } => view.submit_ask_ai(window, cx),
                // Whatever went wrong last time was about the old words.
                InputEvent::Change if view.ask_ai_error.take().is_some() => cx.notify(),
                _ => {}
            });
        self.ask_ai_state = Some(input.clone());
        self.ask_ai_subscription = Some(subscription);
        input
    }

    /// Open the box, or close it and give the filter back its focus.
    pub(crate) fn toggle_ask_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ask_ai_open = !self.ask_ai_open;
        self.ask_ai_error = None;
        if self.ask_ai_open {
            let input = self.ensure_ask_ai_state(window, cx);
            let focus = input.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        } else if let Some(filter) = self.filter_state.clone() {
            let focus = filter.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    pub(super) fn submit_ask_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ask_ai_busy {
            return;
        }
        let Some(input) = self.ask_ai_state.clone() else { return };
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

    /// Put the written filter where the user was already looking. It is not run: a filter you
    /// did not write is one you want to read first, and the editor's own undo takes it back.
    fn accept_written_filter(
        &mut self,
        filter: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self.filter_state.clone() else { return };
        self.syncing_query_inputs = true;
        input.update(cx, |state, cx| {
            state.replace_all(filter, window, cx);
        });
        let formatted = format_query_editor(&input, window, cx);
        self.syncing_query_inputs = false;

        self.filter_auto_pair.sync(&formatted);
        self.filter_error_message = super::query::filter_query_validation_error(&formatted);
        self.filter_dirty = true;
        self.ask_ai_open = false;
        if let Some(state) = &self.ask_ai_state {
            state.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        }
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }
}

/// What the model is told about the collection: its name, and the fields it actually has.
fn ask_ai_context(state: &Entity<AppState>, session_key: &SessionKey, cx: &App) -> QueryContext {
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
    use super::collect_field_lines;
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
}
