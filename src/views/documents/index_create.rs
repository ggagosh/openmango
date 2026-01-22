//! Index creation dialog.

use std::collections::HashMap;

use gpui::*;
use gpui_component::button::{Button as MenuButton, ButtonCustomVariant, ButtonVariants};
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::switch::Switch;
use gpui_component::{
    Disableable as _, Icon, IconName, Sizable as _, Size, StyledExt as _, WindowExt as _,
};
use mongodb::IndexModel;
use mongodb::bson::{Bson, Document, to_bson};

use crate::bson::{document_to_relaxed_extjson_string, parse_document_from_json};
use crate::components::Button;
use crate::connection::get_connection_manager;
use crate::state::{AppCommands, AppEvent, AppState, SessionKey};
use crate::theme::{borders, colors, spacing};

const SAMPLE_SIZE: i64 = 500;
const MAX_SUGGESTIONS: usize = 12;
const MAX_ARRAY_SCAN: usize = 20;
const MAX_DEPTH: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum IndexMode {
    Form,
    Json,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IndexKeyKind {
    Asc,
    Desc,
    Text,
    Hashed,
    TwoDSphere,
    Wildcard,
}

impl IndexKeyKind {
    fn label(self) -> &'static str {
        match self {
            IndexKeyKind::Asc => "1",
            IndexKeyKind::Desc => "-1",
            IndexKeyKind::Text => "text",
            IndexKeyKind::Hashed => "hashed",
            IndexKeyKind::TwoDSphere => "2dsphere",
            IndexKeyKind::Wildcard => "wildcard ($**)",
        }
    }

    fn as_bson(self) -> Bson {
        match self {
            IndexKeyKind::Asc => Bson::Int32(1),
            IndexKeyKind::Desc => Bson::Int32(-1),
            IndexKeyKind::Text => Bson::String("text".to_string()),
            IndexKeyKind::Hashed => Bson::String("hashed".to_string()),
            IndexKeyKind::TwoDSphere => Bson::String("2dsphere".to_string()),
            IndexKeyKind::Wildcard => Bson::Int32(1),
        }
    }
}

#[derive(Default, Clone, Copy)]
struct IndexKeySummary {
    key_count: usize,
    has_text: bool,
    has_hashed: bool,
    has_wildcard: bool,
    has_special: bool,
}

#[derive(Clone)]
struct IndexKeyRow {
    id: u64,
    field_state: Entity<InputState>,
    kind: IndexKeyKind,
}

#[derive(Clone)]
struct FieldSuggestion {
    path: String,
    count: usize,
}

#[derive(Clone)]
struct IndexEditTarget {
    original_name: String,
}

#[derive(Clone)]
enum SampleStatus {
    Idle,
    Loading,
    Ready,
    Error(String),
}

pub struct IndexCreateDialog {
    state: Entity<AppState>,
    session_key: SessionKey,
    mode: IndexMode,
    rows: Vec<IndexKeyRow>,
    next_row_id: u64,
    active_row_id: Option<u64>,
    sample_status: SampleStatus,
    suggestions: Vec<FieldSuggestion>,
    name_state: Entity<InputState>,
    ttl_state: Entity<InputState>,
    partial_state: Entity<InputState>,
    collation_state: Entity<InputState>,
    json_state: Entity<InputState>,
    unique: bool,
    sparse: bool,
    hidden: bool,
    error_message: Option<String>,
    creating: bool,
    edit_target: Option<IndexEditTarget>,
    _subscriptions: Vec<Subscription>,
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

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row_id = self.next_row_id;
        self.next_row_id += 1;
        let field_state = cx.new(|cx| InputState::new(window, cx).placeholder("Field"));
        let subscription = cx.subscribe_in(
            &field_state,
            window,
            move |view, _state, event, window, cx| match event {
                InputEvent::Focus => {
                    view.active_row_id = Some(row_id);
                    cx.notify();
                }
                InputEvent::Change => {
                    view.enforce_guardrails(window, cx);
                    cx.notify();
                }
                _ => {}
            },
        );
        self._subscriptions.push(subscription);
        self.rows.push(IndexKeyRow { id: row_id, field_state, kind: IndexKeyKind::Asc });
        self.enforce_guardrails(window, cx);
    }

    fn remove_row(&mut self, row_id: u64) {
        self.rows.retain(|row| row.id != row_id);
        if self.active_row_id == Some(row_id) {
            self.active_row_id = None;
        }
    }

    fn key_summary(&self, cx: &mut Context<Self>) -> IndexKeySummary {
        let mut summary = IndexKeySummary::default();

        for row in &self.rows {
            let value = row.field_state.read(cx).value().to_string();
            if value.trim().is_empty() {
                continue;
            }
            summary.key_count += 1;
            match row.kind {
                IndexKeyKind::Text => {
                    summary.has_text = true;
                    summary.has_special = true;
                }
                IndexKeyKind::Hashed => {
                    summary.has_hashed = true;
                    summary.has_special = true;
                }
                IndexKeyKind::TwoDSphere => {
                    summary.has_special = true;
                }
                IndexKeyKind::Wildcard => {
                    summary.has_wildcard = true;
                }
                _ => {}
            }
        }

        summary
    }

    fn enforce_guardrails(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let summary = self.key_summary(cx);
        if summary.has_hashed || summary.has_text || summary.has_wildcard {
            self.unique = false;
        }

        let ttl_disabled = summary.key_count != 1 || summary.has_special || summary.has_wildcard;
        if ttl_disabled {
            self.ttl_state.update(cx, |state, cx| {
                state.set_value(String::new(), window, cx);
            });
        }

        if summary.has_wildcard {
            for row in &self.rows {
                if row.kind == IndexKeyKind::Wildcard {
                    row.field_state.update(cx, |state, cx| {
                        state.set_value("$**".to_string(), window, cx);
                    });
                }
            }
        }
    }

    fn set_row_kind(
        &mut self,
        row_id: u64,
        kind: IndexKeyKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == row_id) {
            row.kind = kind;
            if kind == IndexKeyKind::Wildcard {
                row.field_state.update(cx, |state, cx| {
                    state.set_value("$**".to_string(), window, cx);
                });
            }
        }
        self.enforce_guardrails(window, cx);
        cx.notify();
    }

    fn load_sample_fields(&mut self, cx: &mut Context<Self>) {
        let (client, database, collection) = {
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
            )
        };

        self.sample_status = SampleStatus::Loading;
        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move {
                let manager = get_connection_manager();
                manager.sample_documents(&client, &database, &collection, SAMPLE_SIZE)
            }
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

    fn suggestions_for_row(&self, row_id: u64, cx: &mut Context<Self>) -> Vec<FieldSuggestion> {
        if self.active_row_id != Some(row_id) {
            return Vec::new();
        }
        let row = self.rows.iter().find(|row| row.id == row_id);
        let Some(row) = row else {
            return Vec::new();
        };
        let query = row.field_state.read(cx).value().to_string().to_lowercase();
        let mut suggestions = self
            .suggestions
            .iter()
            .filter(|entry| {
                if query.is_empty() { true } else { entry.path.to_lowercase().contains(&query) }
            })
            .cloned()
            .collect::<Vec<_>>();
        suggestions.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.path.cmp(&b.path)));
        suggestions.truncate(MAX_SUGGESTIONS);
        suggestions
    }

    fn build_index_from_form(&mut self, cx: &mut Context<Self>) -> Option<Document> {
        self.error_message = None;

        let mut keys = Document::new();
        let mut has_hashed = false;
        let mut has_text = false;
        let mut has_wildcard = false;
        let mut has_special = false;
        let mut wildcard_rows = 0;
        let mut key_count = 0;

        for row in &self.rows {
            let field = row.field_state.read(cx).value().to_string();
            let field = field.trim();
            if field.is_empty() {
                continue;
            }
            let key = if row.kind == IndexKeyKind::Wildcard { "$**" } else { field };
            if row.kind == IndexKeyKind::Wildcard {
                has_wildcard = true;
                wildcard_rows += 1;
            }
            if row.kind == IndexKeyKind::Hashed {
                has_hashed = true;
            }
            if row.kind == IndexKeyKind::Text {
                has_text = true;
            }
            if matches!(
                row.kind,
                IndexKeyKind::Text | IndexKeyKind::Hashed | IndexKeyKind::TwoDSphere
            ) {
                has_special = true;
            }
            keys.insert(key, row.kind.as_bson());
            key_count += 1;
        }

        if keys.is_empty() {
            self.error_message = Some("Add at least one index field.".to_string());
            return None;
        }

        if has_wildcard && keys.len() > 1 {
            self.error_message = Some("Wildcard indexes must contain only $**.".to_string());
            return None;
        }

        if has_wildcard && wildcard_rows == 0 {
            self.error_message = Some("Wildcard indexes must use $** as the field.".to_string());
            return None;
        }

        if (has_hashed || has_text || has_wildcard) && self.unique {
            self.error_message =
                Some("Unique indexes cannot be hashed, text, or wildcard.".to_string());
            return None;
        }

        let mut index_doc = Document::new();
        index_doc.insert("key", keys);

        let name = self.name_state.read(cx).value().to_string();
        if !name.trim().is_empty() {
            index_doc.insert("name", name.trim());
        } else if self.edit_target.is_some() {
            self.error_message = Some("Index name is required for replace.".to_string());
            return None;
        }

        if self.unique {
            index_doc.insert("unique", true);
        }
        if self.sparse {
            index_doc.insert("sparse", true);
        }
        if self.hidden {
            index_doc.insert("hidden", true);
        }

        let ttl_raw = self.ttl_state.read(cx).value().to_string();
        if !ttl_raw.trim().is_empty() {
            if key_count != 1 || has_special || has_wildcard {
                self.error_message =
                    Some("TTL requires a single ascending/descending field.".to_string());
                return None;
            }
            match ttl_raw.trim().parse::<i64>() {
                Ok(value) if value > 0 => {
                    index_doc.insert("expireAfterSeconds", value);
                }
                _ => {
                    self.error_message = Some("TTL must be a positive number.".to_string());
                    return None;
                }
            }
        }

        let partial_raw = self.partial_state.read(cx).value().to_string();
        if !partial_raw.trim().is_empty() && partial_raw.trim() != "{}" {
            match parse_document_from_json(partial_raw.trim()) {
                Ok(doc) => {
                    index_doc.insert("partialFilterExpression", doc);
                }
                Err(err) => {
                    self.error_message = Some(format!("Invalid partial filter JSON: {err}"));
                    return None;
                }
            }
        }

        let collation_raw = self.collation_state.read(cx).value().to_string();
        if !collation_raw.trim().is_empty() && collation_raw.trim() != "{}" {
            match parse_document_from_json(collation_raw.trim()) {
                Ok(doc) => {
                    index_doc.insert("collation", doc);
                }
                Err(err) => {
                    self.error_message = Some(format!("Invalid collation JSON: {err}"));
                    return None;
                }
            }
        }

        Some(index_doc)
    }

    fn build_index_from_json(&mut self, cx: &mut Context<Self>) -> Option<Document> {
        self.error_message = None;
        let raw = self.json_state.read(cx).value().to_string();
        match parse_document_from_json(raw.trim()) {
            Ok(doc) => match doc.get_document("key") {
                Ok(keys) if !keys.is_empty() => {
                    if self.edit_target.is_some() && doc.get("name").is_none() {
                        self.error_message =
                            Some("Index JSON must include `name` when replacing.".to_string());
                        return None;
                    }
                    Some(doc)
                }
                _ => {
                    self.error_message =
                        Some("Index JSON must include a non-empty `key` document.".to_string());
                    None
                }
            },
            Err(err) => {
                self.error_message = Some(format!("Invalid JSON: {err}"));
                None
            }
        }
    }

    fn render_suggestions(
        &self,
        view: Entity<Self>,
        row_id: u64,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let suggestions = self.suggestions_for_row(row_id, cx);
        if suggestions.is_empty() {
            return None;
        }

        let mut row_children = Vec::new();
        for (index, suggestion) in suggestions.into_iter().enumerate() {
            let label = format!("{} ({})", suggestion.path, suggestion.count);
            let target = suggestion.path.clone();
            row_children.push(
                Button::new((SharedString::from(format!("index-suggestion-{row_id}")), index))
                    .ghost()
                    .compact()
                    .label(label)
                    .on_click({
                        let target = target.clone();
                        let view = view.clone();
                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                            view.update(cx, |this, cx| {
                                if let Some(row) = this.rows.iter().find(|row| row.id == row_id) {
                                    row.field_state.update(cx, |state, cx| {
                                        state.set_value(target.clone(), window, cx);
                                    });
                                    this.active_row_id = Some(row_id);
                                    cx.notify();
                                }
                            });
                        }
                    })
                    .into_any_element(),
            );
        }

        Some(
            div()
                .flex()
                .flex_wrap()
                .gap(spacing::xs())
                .px(spacing::sm())
                .py(px(2.0))
                .child(div().text_xs().text_color(colors::text_muted()).child("Suggestions"))
                .children(row_children)
                .into_any_element(),
        )
    }
}

impl Render for IndexCreateDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let mut rows = Vec::new();
        let summary = self.key_summary(cx);
        let wildcard_selected = summary.has_wildcard;
        let unique_disabled = summary.has_hashed || summary.has_text || summary.has_wildcard;
        let ttl_disabled = summary.key_count != 1 || summary.has_special || summary.has_wildcard;

        for row in &self.rows {
            let row_id = row.id;
            let field_state = row.field_state.clone();
            let kind_label = row.kind.label();
            let show_remove = self.rows.len() > 1;
            let allow_wildcard = summary.key_count <= 1 || row.kind == IndexKeyKind::Wildcard;

            let kind_variant = ButtonCustomVariant::new(cx)
                .color(colors::bg_button_secondary().into())
                .foreground(colors::text_primary().into())
                .border(colors::border_subtle().into())
                .hover(colors::bg_button_secondary_hover().into())
                .active(colors::bg_button_secondary_hover().into())
                .shadow(false);

            let kind_button = MenuButton::new(("index-kind", row_id))
                .compact()
                .label(kind_label)
                .dropdown_caret(true)
                .custom(kind_variant)
                .rounded(borders::radius_sm())
                .with_size(Size::XSmall)
                .refine_style(
                    &StyleRefinement::default()
                        .font_family(crate::theme::fonts::ui())
                        .font_weight(FontWeight::NORMAL)
                        .text_size(crate::theme::typography::text_xs())
                        .h(px(22.0))
                        .px(spacing::sm())
                        .py(px(2.0)),
                );

            let row_view = div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .child(
                    Input::new(&field_state)
                        .font_family(crate::theme::fonts::mono())
                        .w(px(280.0))
                        .disabled(row.kind == IndexKeyKind::Wildcard),
                )
                .child(kind_button.dropdown_menu_with_anchor(Corner::BottomLeft, {
                    let view = view.clone();
                    move |menu, _window, _cx| {
                        menu.item(PopupMenuItem::new("1").on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_row_kind(row_id, IndexKeyKind::Asc, window, cx);
                                });
                            }
                        }))
                        .item(PopupMenuItem::new("-1").on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_row_kind(row_id, IndexKeyKind::Desc, window, cx);
                                });
                            }
                        }))
                        .item(PopupMenuItem::new("text").on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_row_kind(row_id, IndexKeyKind::Text, window, cx);
                                });
                            }
                        }))
                        .item(PopupMenuItem::new("hashed").on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_row_kind(row_id, IndexKeyKind::Hashed, window, cx);
                                });
                            }
                        }))
                        .item(PopupMenuItem::new("2dsphere").on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_row_kind(row_id, IndexKeyKind::TwoDSphere, window, cx);
                                });
                            }
                        }))
                        .item(
                            PopupMenuItem::new("wildcard ($**)")
                                .disabled(!allow_wildcard)
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        view.update(cx, |this, cx| {
                                            this.set_row_kind(
                                                row_id,
                                                IndexKeyKind::Wildcard,
                                                window,
                                                cx,
                                            );
                                        });
                                    }
                                }),
                        )
                    }
                }))
                .child(
                    Button::new(("remove-index-row", row_id))
                        .ghost()
                        .compact()
                        .icon(Icon::new(IconName::Close).xsmall())
                        .disabled(!show_remove)
                        .on_click({
                            let view = view.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                view.update(cx, |this, cx| {
                                    this.remove_row(row_id);
                                    this.enforce_guardrails(window, cx);
                                    cx.notify();
                                });
                            }
                        }),
                );

            rows.push(row_view.into_any_element());
            if let Some(suggestions) = self.render_suggestions(view.clone(), row_id, cx) {
                rows.push(suggestions);
            }
        }

        let sample_label = match &self.sample_status {
            SampleStatus::Idle => "Sampling fields...".to_string(),
            SampleStatus::Loading => format!("Sampling {SAMPLE_SIZE} docs..."),
            SampleStatus::Ready => format!("Sampled {} docs", SAMPLE_SIZE),
            SampleStatus::Error(message) => format!("Sample failed: {message}"),
        };

        let can_add_row = !wildcard_selected;
        let form_view = div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .child(div().text_sm().text_color(colors::text_secondary()).child("Index keys"))
            .child(div().flex().flex_col().gap(spacing::xs()).children(rows))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        Button::new("add-index-row")
                            .ghost()
                            .compact()
                            .label("Add field")
                            .disabled(!can_add_row)
                            .on_click({
                                let view = view.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    view.update(cx, |this, cx| {
                                        this.add_row(window, cx);
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(div().text_xs().text_color(colors::text_muted()).child(sample_label)),
            )
            .child(div().h(px(1.0)).bg(colors::border_subtle()))
            .child(div().text_sm().text_color(colors::text_secondary()).child("Options"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Input::new(&self.name_state)
                            .font_family(crate::theme::fonts::mono())
                            .w(px(260.0)),
                    )
                    .child(
                        NumberInput::new(&self.ttl_state)
                            .font_family(crate::theme::fonts::mono())
                            .w(px(160.0))
                            .disabled(ttl_disabled),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                Switch::new("unique-index")
                                    .checked(self.unique)
                                    .small()
                                    .disabled(unique_disabled)
                                    .on_click({
                                        let view = view.clone();
                                        move |checked, _window, cx| {
                                            if unique_disabled {
                                                return;
                                            }
                                            view.update(cx, |this, cx| {
                                                this.unique = *checked;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors::text_secondary())
                                    .child("Unique"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                Switch::new("sparse-index").checked(self.sparse).small().on_click(
                                    {
                                        let view = view.clone();
                                        move |checked, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.sparse = *checked;
                                                cx.notify();
                                            });
                                        }
                                    },
                                ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors::text_secondary())
                                    .child("Sparse"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                Switch::new("hidden-index").checked(self.hidden).small().on_click(
                                    {
                                        let view = view.clone();
                                        move |checked, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.hidden = *checked;
                                                cx.notify();
                                            });
                                        }
                                    },
                                ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors::text_secondary())
                                    .child("Hidden"),
                            ),
                    ),
            )
            .child({
                let mut notes = Vec::new();
                if unique_disabled {
                    notes.push("Unique is unavailable for text/hashed/wildcard indexes.");
                }
                if ttl_disabled {
                    notes.push("TTL requires a single ascending/descending field.");
                }
                if notes.is_empty() {
                    div().into_any_element()
                } else {
                    div()
                        .text_xs()
                        .text_color(colors::text_muted())
                        .child(notes.join(" "))
                        .into_any_element()
                }
            })
            .child(
                div()
                    .flex()
                    .gap(spacing::sm())
                    .child(
                        Input::new(&self.partial_state)
                            .font_family(crate::theme::fonts::mono())
                            .h(px(120.0))
                            .w_full(),
                    )
                    .child(
                        Input::new(&self.collation_state)
                            .font_family(crate::theme::fonts::mono())
                            .h(px(120.0))
                            .w_full(),
                    ),
            );

        let json_view = div().flex().flex_col().gap(spacing::sm()).child(
            Input::new(&self.json_state)
                .font_family(crate::theme::fonts::mono())
                .h(px(360.0))
                .w_full(),
        );

        let form_button = {
            let base = Button::new("index-mode-form").compact().label("Form").on_click({
                let view = view.clone();
                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    view.update(cx, |this, cx| {
                        this.mode = IndexMode::Form;
                        cx.notify();
                    });
                }
            });
            if self.mode == IndexMode::Form { base.primary() } else { base.ghost() }
        };

        let json_button = {
            let base = Button::new("index-mode-json").compact().label("JSON").on_click({
                let view = view.clone();
                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    view.update(cx, |this, cx| {
                        this.mode = IndexMode::Json;
                        cx.notify();
                    });
                }
            });
            if self.mode == IndexMode::Json { base.primary() } else { base.ghost() }
        };

        let tabs = div().flex().gap(spacing::xs()).child(form_button).child(json_button);

        let is_edit = self.edit_target.is_some();
        let (status_text, status_color) = if let Some(error) = &self.error_message {
            (error.clone(), colors::text_error())
        } else if self.creating {
            (
                if is_edit {
                    "Replacing index...".to_string()
                } else {
                    "Creating index...".to_string()
                },
                colors::text_muted(),
            )
        } else if is_edit {
            ("Save will drop and recreate this index.".to_string(), colors::text_muted())
        } else {
            ("".to_string(), colors::text_muted())
        };

        let action_row = div()
            .flex()
            .items_center()
            .justify_between()
            .pt(spacing::xs())
            .child(div().min_h(px(18.0)).text_sm().text_color(status_color).child(status_text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(Button::new("cancel-index").label("Cancel").on_click(
                        |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                            window.close_dialog(cx);
                        },
                    ))
                    .child({
                        let label = if is_edit { "Save & Replace" } else { "Create" };
                        Button::new("create-index")
                            .primary()
                            .label(if self.creating {
                                if is_edit { "Replacing..." } else { "Creating..." }
                            } else {
                                label
                            })
                            .disabled(self.creating)
                            .on_click({
                                let state = self.state.clone();
                                let session_key = self.session_key.clone();
                                let view = view.clone();
                                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                    view.update(cx, |this, cx| {
                                        if this.creating {
                                            return;
                                        }
                                        let index_doc = match this.mode {
                                            IndexMode::Form => this.build_index_from_form(cx),
                                            IndexMode::Json => this.build_index_from_json(cx),
                                        };
                                        let Some(index_doc) = index_doc else {
                                            cx.notify();
                                            return;
                                        };
                                        this.error_message = None;
                                        this.creating = true;
                                        cx.notify();

                                        if let Some(edit_target) = this.edit_target.as_ref() {
                                            AppCommands::replace_collection_index(
                                                state.clone(),
                                                session_key.clone(),
                                                edit_target.original_name.clone(),
                                                index_doc,
                                                cx,
                                            );
                                        } else {
                                            AppCommands::create_collection_index(
                                                state.clone(),
                                                session_key.clone(),
                                                index_doc,
                                                cx,
                                            );
                                        }
                                    });
                                }
                            })
                    }),
            );

        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .p(spacing::md())
            .child(tabs)
            .child(if self.mode == IndexMode::Form { form_view } else { json_view })
            .child(action_row)
    }
}

fn build_field_suggestions(docs: &[Document]) -> Vec<FieldSuggestion> {
    let mut counts: HashMap<String, usize> = HashMap::new();

    for doc in docs {
        for (key, value) in doc {
            let path = key.to_string();
            *counts.entry(path.clone()).or_insert(0) += 1;
            collect_paths(value, &path, &mut counts, 1);
        }
    }

    let mut suggestions =
        counts.into_iter().map(|(path, count)| FieldSuggestion { path, count }).collect::<Vec<_>>();

    suggestions.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.path.cmp(&b.path)));
    suggestions.truncate(200);
    suggestions
}

fn index_kind_from_bson(key: &str, value: &Bson) -> IndexKeyKind {
    if key == "$**" {
        return IndexKeyKind::Wildcard;
    }

    match value {
        Bson::String(text) => match text.as_str() {
            "text" => IndexKeyKind::Text,
            "hashed" => IndexKeyKind::Hashed,
            "2dsphere" => IndexKeyKind::TwoDSphere,
            "-1" => IndexKeyKind::Desc,
            _ => IndexKeyKind::Asc,
        },
        Bson::Int32(val) => {
            if *val < 0 {
                IndexKeyKind::Desc
            } else {
                IndexKeyKind::Asc
            }
        }
        Bson::Int64(val) => {
            if *val < 0 {
                IndexKeyKind::Desc
            } else {
                IndexKeyKind::Asc
            }
        }
        Bson::Double(val) => {
            if *val < 0.0 {
                IndexKeyKind::Desc
            } else {
                IndexKeyKind::Asc
            }
        }
        _ => IndexKeyKind::Asc,
    }
}

fn collect_paths(value: &Bson, prefix: &str, counts: &mut HashMap<String, usize>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }

    match value {
        Bson::Document(doc) => {
            for (key, value) in doc {
                let path = format!("{prefix}.{key}");
                *counts.entry(path.clone()).or_insert(0) += 1;
                collect_paths(value, &path, counts, depth + 1);
            }
        }
        Bson::Array(values) => {
            if prefix.is_empty() {
                return;
            }
            let array_path = format!("{prefix}[]");
            *counts.entry(array_path.clone()).or_insert(0) += 1;

            for value in values.iter().take(MAX_ARRAY_SCAN) {
                match value {
                    Bson::Document(doc) => {
                        for (key, value) in doc {
                            let path = format!("{array_path}.{key}");
                            *counts.entry(path.clone()).or_insert(0) += 1;
                            collect_paths(value, &path, counts, depth + 1);
                        }
                    }
                    Bson::Array(_) => {
                        collect_paths(value, &array_path, counts, depth + 1);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}
