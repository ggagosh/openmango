//! Index creation dialog.

mod form_builder;
mod key_rows;
mod render;
pub(super) mod support;

use gpui::*;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::InputState;
use mongodb::IndexModel;
use mongodb::bson::{Bson, Document, to_bson};

use crate::bson::{document_to_relaxed_extjson_string, parse_document_from_json};
use crate::state::{AppEvent, AppState, SessionKey};

use key_rows::IndexKeyRow;
use support::{
    FieldSuggestion, IndexEditTarget, IndexKeyKind, IndexMode, SAMPLE_SIZE, SampleStatus,
    build_field_suggestions, index_kind_from_bson,
};

pub struct IndexCreateDialog {
    state: Entity<AppState>,
    session_key: SessionKey,
    pub(super) mode: IndexMode,
    rows: Vec<IndexKeyRow>,
    pub(super) next_row_id: u64,
    pub(super) active_row_id: Option<u64>,
    pub(super) sample_status: SampleStatus,
    pub(super) suggestions: Vec<FieldSuggestion>,
    pub(super) name_state: Entity<InputState>,
    pub(super) ttl_state: Entity<InputState>,
    pub(super) partial_state: Entity<InputState>,
    pub(super) collation_state: Entity<InputState>,
    pub(super) json_state: Entity<InputState>,
    pub(super) unique: bool,
    pub(super) sparse: bool,
    pub(super) hidden: bool,
    pub(super) error_message: Option<String>,
    pub(super) creating: bool,
    pub(super) edit_target: Option<IndexEditTarget>,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl IndexCreateDialog {
    pub fn open(
        state: Entity<AppState>,
        session_key: SessionKey,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view =
            cx.new(|cx| IndexCreateDialog::new(state.clone(), session_key, window, cx));
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("Create Index").w(px(912.0)).child(dialog_view.clone())
        });
    }

    pub fn open_edit(
        state: Entity<AppState>,
        session_key: SessionKey,
        model: IndexModel,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view = cx.new(|cx| {
            IndexCreateDialog::new_with_index(state.clone(), session_key, model, window, cx)
        });
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("Edit Index").w(px(912.0)).child(dialog_view.clone())
        });
    }

    fn new(
        state: Entity<AppState>,
        session_key: SessionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_state = cx.new(|cx| InputState::new(window, cx).placeholder("Index name"));
        let ttl_state = cx.new(|cx| InputState::new(window, cx).placeholder("TTL seconds"));
        let partial_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Partial filter (JSON)")
                .code_editor("json")
                .soft_wrap(true)
        });
        let collation_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Collation (JSON)")
                .code_editor("json")
                .soft_wrap(true)
        });
        let json_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("{ \"key\": { \"field\": 1 } }")
                .code_editor("json")
                .line_number(true)
                .soft_wrap(true)
        });

        let mut dialog = Self {
            state,
            session_key,
            mode: IndexMode::Form,
            rows: Vec::new(),
            next_row_id: 1,
            active_row_id: None,
            sample_status: SampleStatus::Idle,
            suggestions: Vec::new(),
            name_state,
            ttl_state,
            partial_state,
            collation_state,
            json_state,
            unique: false,
            sparse: false,
            hidden: false,
            error_message: None,
            creating: false,
            edit_target: None,
            _subscriptions: Vec::new(),
        };

        let state_subscription =
            cx.subscribe_in(&dialog.state, window, move |view, _state, event, window, cx| {
                match event {
                    AppEvent::IndexCreated { session, .. } if session == &view.session_key => {
                        view.creating = false;
                        view.error_message = None;
                        window.close_dialog(cx);
                    }
                    AppEvent::IndexCreateFailed { session, error }
                        if session == &view.session_key =>
                    {
                        view.creating = false;
                        view.error_message = Some(error.clone());
                        cx.notify();
                    }
                    _ => {}
                }
            });
        dialog._subscriptions.push(state_subscription);

        dialog.add_row(window, cx);
        dialog.load_sample_fields(cx);

        dialog
    }

    fn new_with_index(
        state: Entity<AppState>,
        session_key: SessionKey,
        model: IndexModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut dialog = Self::new(state, session_key, window, cx);
        dialog.apply_index_model(model, window, cx);
        dialog
    }

    fn apply_index_model(
        &mut self,
        model: IndexModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rows.clear();
        self.next_row_id = 1;
        self.active_row_id = None;

        let original_name = model
            .options
            .as_ref()
            .and_then(|options| options.name.clone())
            .unwrap_or_else(|| "unnamed".to_string());
        self.edit_target = Some(IndexEditTarget { original_name: original_name.clone() });

        let mut found_keys = false;
        for (key, value) in model.keys.iter() {
            found_keys = true;
            self.add_row(window, cx);
            if let Some(row) = self.rows.last_mut() {
                let kind = index_kind_from_bson(key, value);
                row.kind = kind;
                let field_value = if kind == IndexKeyKind::Wildcard {
                    "$**".to_string()
                } else {
                    key.to_string()
                };
                row.field_state.update(cx, |state, cx| {
                    state.set_value(field_value, window, cx);
                });
            }
        }

        if !found_keys {
            self.add_row(window, cx);
        }

        if let Some(options) = model.options.as_ref() {
            if let Some(name) = options.name.as_ref() {
                self.name_state.update(cx, |state, cx| {
                    state.set_value(name.clone(), window, cx);
                });
            }
            self.unique = options.unique.unwrap_or(false);
            self.sparse = options.sparse.unwrap_or(false);
            self.hidden = options.hidden.unwrap_or(false);

            if let Some(expire_after) = options.expire_after {
                let seconds = expire_after.as_secs();
                self.ttl_state.update(cx, |state, cx| {
                    state.set_value(seconds.to_string(), window, cx);
                });
            }

            if let Some(partial) = options.partial_filter_expression.as_ref() {
                let formatted = document_to_relaxed_extjson_string(partial);
                self.partial_state.update(cx, |state, cx| {
                    state.set_value(formatted, window, cx);
                });
            }

            if let Some(collation) = options.collation.as_ref()
                && let Ok(bson) = to_bson(collation)
                && let Bson::Document(doc) = bson
            {
                let formatted = document_to_relaxed_extjson_string(&doc);
                self.collation_state.update(cx, |state, cx| {
                    state.set_value(formatted, window, cx);
                });
            }
        }

        let mut json_doc = Document::new();
        json_doc.insert("key", model.keys.clone());
        json_doc.insert("name", original_name);
        if self.unique {
            json_doc.insert("unique", true);
        }
        if self.sparse {
            json_doc.insert("sparse", true);
        }
        if self.hidden {
            json_doc.insert("hidden", true);
        }
        let ttl_raw = self.ttl_state.read(cx).value().to_string();
        if let Ok(ttl) = ttl_raw.trim().parse::<i64>()
            && ttl > 0
        {
            json_doc.insert("expireAfterSeconds", ttl);
        }
        let partial_raw = self.partial_state.read(cx).value().to_string();
        if let Ok(doc) = parse_document_from_json(partial_raw.trim())
            && !doc.is_empty()
        {
            json_doc.insert("partialFilterExpression", doc);
        }
        let collation_raw = self.collation_state.read(cx).value().to_string();
        if let Ok(doc) = parse_document_from_json(collation_raw.trim())
            && !doc.is_empty()
        {
            json_doc.insert("collation", doc);
        }

        let json_text = document_to_relaxed_extjson_string(&json_doc);
        self.json_state.update(cx, |state, cx| {
            state.set_value(json_text, window, cx);
        });

        self.mode = IndexMode::Form;
        self.enforce_guardrails(window, cx);
    }

    fn load_sample_fields(&mut self, cx: &mut Context<Self>) {
        let (client, database, collection, manager) = {
            let state_ref = self.state.read(cx);
            let conn_id = self.session_key.connection_id;
            let Some(conn) = state_ref.active_connection_by_id(conn_id) else {
                self.sample_status = SampleStatus::Error("No active connection".to_string());
                return;
            };
            (
                conn.client.clone(),
                self.session_key.database.clone(),
                self.session_key.collection.clone(),
                state_ref.connection_manager(),
            )
        };

        self.sample_status = SampleStatus::Loading;
        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move { manager.sample_documents(&client, &database, &collection, SAMPLE_SIZE) }
        });

        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result: Result<Vec<Document>, crate::error::Error> = task.await;
            let _ = cx.update(|cx| {
                let _ = view.update(cx, |this, cx: &mut Context<Self>| match result {
                    Ok(docs) => {
                        this.suggestions = build_field_suggestions(&docs);
                        this.sample_status = SampleStatus::Ready;
                        cx.notify();
                    }
                    Err(err) => {
                        this.sample_status = SampleStatus::Error(err.to_string());
                        cx.notify();
                    }
                });
            });
        })
        .detach();
    }
}
