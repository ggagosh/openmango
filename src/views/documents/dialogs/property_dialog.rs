//! Property-level edit dialogs for document fields.

use gpui::*;
use gpui_component::button::{Button as MenuButton, ButtonCustomVariant, ButtonVariants};
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputState};
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::{Disableable as _, Sizable as _, Size, StyledExt as _, WindowExt as _};
use mongodb::bson::{self, Bson, Document, doc, oid::ObjectId};

use crate::bson::{DocumentKey, PathSegment, parse_document_from_json};
use crate::components::Button;
use crate::state::{AppCommands, AppEvent, AppState, SessionKey};
use crate::theme::{borders, colors, spacing};
use crate::views::documents::node_meta::NodeMeta;

use super::property_dialog_support::{
    PropertyActionKind, UpdateScope, ValueType, display_path, display_segment, dot_path,
    format_bson_for_input, parent_path, parse_bool, parse_date, parse_f64, parse_i32, parse_i64,
};

pub struct PropertyActionDialog {
    state: Entity<AppState>,
    session_key: SessionKey,
    doc_key: DocumentKey,
    action: PropertyActionKind,
    path_dot: String,
    parent_dot: String,
    array_dot: String,
    allow_bulk: bool,
    scope: UpdateScope,
    value_type: ValueType,
    parent_state: Entity<InputState>,
    field_display_state: Entity<InputState>,
    field_state: Entity<InputState>,
    value_state: Entity<InputState>,
    error_message: Option<String>,
    updating: bool,
    _subscriptions: Vec<Subscription>,
}

impl PropertyActionDialog {
    pub fn open_edit_value(
        state: Entity<AppState>,
        session_key: SessionKey,
        meta: NodeMeta,
        allow_bulk: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view = cx.new(|cx| {
            Self::new(
                state.clone(),
                session_key,
                meta,
                PropertyActionKind::EditValue,
                allow_bulk,
                window,
                cx,
            )
        });
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("Edit Value / Type").w(px(640.0)).child(dialog_view.clone())
        });
    }

    pub fn open_add_field(
        state: Entity<AppState>,
        session_key: SessionKey,
        meta: NodeMeta,
        allow_bulk: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view = cx.new(|cx| {
            Self::new(
                state.clone(),
                session_key,
                meta,
                PropertyActionKind::AddField,
                allow_bulk,
                window,
                cx,
            )
        });
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("Add Field/Value").w(px(640.0)).child(dialog_view.clone())
        });
    }

    pub fn open_rename_field(
        state: Entity<AppState>,
        session_key: SessionKey,
        meta: NodeMeta,
        allow_bulk: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view = cx.new(|cx| {
            Self::new(
                state.clone(),
                session_key,
                meta,
                PropertyActionKind::RenameField,
                allow_bulk,
                window,
                cx,
            )
        });
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("Rename Field").w(px(600.0)).child(dialog_view.clone())
        });
    }

    pub fn open_remove_field(
        state: Entity<AppState>,
        session_key: SessionKey,
        meta: NodeMeta,
        allow_bulk: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view = cx.new(|cx| {
            Self::new(
                state.clone(),
                session_key,
                meta,
                PropertyActionKind::RemoveField,
                allow_bulk,
                window,
                cx,
            )
        });
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("Remove Field").w(px(560.0)).child(dialog_view.clone())
        });
    }

    pub fn open_add_element(
        state: Entity<AppState>,
        session_key: SessionKey,
        meta: NodeMeta,
        allow_bulk: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view = cx.new(|cx| {
            Self::new(
                state.clone(),
                session_key,
                meta,
                PropertyActionKind::AddElement,
                allow_bulk,
                window,
                cx,
            )
        });
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("Add Element").w(px(640.0)).child(dialog_view.clone())
        });
    }

    pub fn open_remove_matching(
        state: Entity<AppState>,
        session_key: SessionKey,
        meta: NodeMeta,
        allow_bulk: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view = cx.new(|cx| {
            Self::new(
                state.clone(),
                session_key,
                meta,
                PropertyActionKind::RemoveMatchingValues,
                allow_bulk,
                window,
                cx,
            )
        });
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("Remove Matching Values").w(px(640.0)).child(dialog_view.clone())
        });
    }

    fn new(
        state: Entity<AppState>,
        session_key: SessionKey,
        meta: NodeMeta,
        action: PropertyActionKind,
        allow_bulk: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut parent_path = parent_path(&meta.path);
        if action == PropertyActionKind::AddField && matches!(meta.value, Some(Bson::Document(_))) {
            parent_path = meta.path.clone();
        }
        let parent_label = if parent_path.is_empty() {
            "(document)".to_string()
        } else {
            display_path(&parent_path)
        };
        let field_label = display_segment(meta.path.last());
        let field_display_label = match action {
            PropertyActionKind::AddElement | PropertyActionKind::RemoveMatchingValues => {
                if matches!(meta.path.last(), Some(PathSegment::Index(_))) {
                    display_path(&parent_path)
                } else {
                    display_path(&meta.path)
                }
            }
            _ => {
                if matches!(meta.path.last(), Some(PathSegment::Index(_))) {
                    display_path(&meta.path)
                } else {
                    field_label.clone()
                }
            }
        };
        let path_dot = dot_path(&meta.path);
        let parent_dot = dot_path(&parent_path);
        let array_dot = if matches!(meta.path.last(), Some(PathSegment::Index(_))) {
            parent_dot.clone()
        } else {
            path_dot.clone()
        };

        let parent_state = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(parent_label.clone(), window, cx);
            state
        });
        let field_display_state = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(field_display_label.clone(), window, cx);
            state
        });
        let field_state = cx.new(|cx| InputState::new(window, cx).placeholder("Field name"));
        let value_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(ValueType::String.placeholder())
                .code_editor("json")
                .soft_wrap(true)
        });

        if action == PropertyActionKind::RenameField
            && let Some(PathSegment::Key(key)) = meta.path.last()
        {
            field_state.update(cx, |state, cx| {
                state.set_value(key.clone(), window, cx);
            });
        }

        let mut value_type = ValueType::String;
        let mut should_prefill_value = false;
        if let Some(value) = meta.value.as_ref() {
            match action {
                PropertyActionKind::EditValue => {
                    value_type = ValueType::from_bson(value);
                    should_prefill_value = true;
                }
                PropertyActionKind::RemoveMatchingValues => {
                    if matches!(meta.path.last(), Some(PathSegment::Index(_))) {
                        value_type = ValueType::from_bson(value);
                        should_prefill_value = true;
                    }
                }
                _ => {}
            }
        }

        if should_prefill_value && let Some(value) = meta.value.as_ref() {
            let raw = format_bson_for_input(value);
            value_state.update(cx, |state, cx| {
                state.set_value(raw, window, cx);
            });
        }

        let mut dialog = Self {
            state,
            session_key,
            doc_key: meta.doc_key.clone(),
            action,
            path_dot,
            parent_dot,
            array_dot,
            allow_bulk,
            scope: UpdateScope::CurrentDocument,
            value_type,
            parent_state,
            field_display_state,
            field_state: field_state.clone(),
            value_state: value_state.clone(),
            error_message: None,
            updating: false,
            _subscriptions: Vec::new(),
        };

        dialog.update_placeholder(window, cx);

        let subscription =
            cx.subscribe_in(&dialog.state, window, move |view, _state, event, window, cx| {
                match event {
                    AppEvent::DocumentsUpdated { session, .. } if session == &view.session_key => {
                        if view.updating {
                            view.updating = false;
                            view.error_message = None;
                            window.close_dialog(cx);
                        }
                    }
                    AppEvent::DocumentsUpdateFailed { session, error }
                        if session == &view.session_key =>
                    {
                        view.updating = false;
                        view.error_message = Some(error.clone());
                        cx.notify();
                    }
                    _ => {}
                }
            });
        dialog._subscriptions.push(subscription);

        let subscription = cx.intercept_keystrokes(move |event, window, cx| {
            let key = event.keystroke.key.to_ascii_lowercase();
            if key == "escape" {
                window.close_dialog(cx);
                cx.stop_propagation();
            }
        });
        dialog._subscriptions.push(subscription);

        dialog
    }

    fn update_placeholder(&self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = self.value_type.placeholder();
        self.value_state.update(cx, |state, cx| {
            state.set_placeholder(placeholder, window, cx);
        });
    }

    fn set_scope(&mut self, scope: UpdateScope, cx: &mut Context<Self>) {
        if !self.allow_bulk {
            self.scope = UpdateScope::CurrentDocument;
            return;
        }
        self.scope = scope;
        cx.notify();
    }

    fn set_value_type(&mut self, kind: ValueType, window: &mut Window, cx: &mut Context<Self>) {
        self.value_type = kind;
        self.update_placeholder(window, cx);
        cx.notify();
    }

    fn parse_value(&self, cx: &mut Context<Self>) -> Result<Bson, String> {
        let raw = self.value_state.read(cx).value().to_string();
        let trimmed = raw.trim();

        match self.value_type {
            ValueType::String => Ok(Bson::String(raw)),
            ValueType::Bool => parse_bool(trimmed),
            ValueType::Int32 => parse_i32(trimmed),
            ValueType::Int64 => parse_i64(trimmed),
            ValueType::Double => parse_f64(trimmed),
            ValueType::Null => Ok(Bson::Null),
            ValueType::ObjectId => ObjectId::parse_str(trimmed)
                .map(Bson::ObjectId)
                .map_err(|_| "Expected ObjectId hex".to_string()),
            ValueType::Date => parse_date(trimmed),
            ValueType::Document => {
                let raw = if trimmed.is_empty() { "{}" } else { trimmed };
                parse_document_from_json(raw)
                    .map(Bson::Document)
                    .map_err(|err| format!("Invalid JSON: {err}"))
            }
            ValueType::Array => {
                let raw = if trimmed.is_empty() { "[]" } else { trimmed };
                let value: serde_json::Value =
                    serde_json::from_str(raw).map_err(|e| e.to_string())?;
                let bson = bson::Bson::try_from(value).map_err(|e| e.to_string())?;
                match bson {
                    Bson::Array(arr) => Ok(Bson::Array(arr)),
                    _ => Err("Root JSON must be an array".to_string()),
                }
            }
        }
    }

    fn submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.updating {
            return;
        }

        self.error_message = None;
        let update_doc = match self.build_update_doc(cx) {
            Ok(doc) => doc,
            Err(err) => {
                self.error_message = Some(err);
                cx.notify();
                return;
            }
        };

        self.updating = true;
        cx.notify();

        match self.effective_scope() {
            UpdateScope::CurrentDocument => {
                AppCommands::update_document_by_key(
                    self.state.clone(),
                    self.session_key.clone(),
                    self.doc_key.clone(),
                    update_doc,
                    cx,
                );
            }
            UpdateScope::MatchQuery => {
                let filter = self.current_filter(cx);
                AppCommands::update_documents_by_filter(
                    self.state.clone(),
                    self.session_key.clone(),
                    filter,
                    update_doc,
                    cx,
                );
            }
            UpdateScope::AllDocuments => {
                AppCommands::update_documents_by_filter(
                    self.state.clone(),
                    self.session_key.clone(),
                    Document::new(),
                    update_doc,
                    cx,
                );
            }
        }
    }

    fn effective_scope(&self) -> UpdateScope {
        if self.allow_bulk { self.scope } else { UpdateScope::CurrentDocument }
    }

    fn current_filter(&self, cx: &mut Context<Self>) -> Document {
        self.state.read(cx).session_filter(&self.session_key).unwrap_or_default()
    }

    fn build_update_doc(&self, cx: &mut Context<Self>) -> Result<Document, String> {
        match self.action {
            PropertyActionKind::EditValue => {
                let value = self.parse_value(cx)?;
                Ok(doc! { "$set": { self.path_dot.clone(): value } })
            }
            PropertyActionKind::AddField => {
                let field_name = self.field_state.read(cx).value().to_string();
                let field_name = field_name.trim();
                if field_name.is_empty() {
                    return Err("Field name is required.".to_string());
                }
                if field_name.contains('.') || field_name.contains('$') {
                    return Err("Field name cannot contain '.' or '$'.".to_string());
                }
                let value = self.parse_value(cx)?;
                let full_path = if self.parent_dot.is_empty() {
                    field_name.to_string()
                } else {
                    format!("{}.{}", self.parent_dot, field_name)
                };
                Ok(doc! { "$set": { full_path: value } })
            }
            PropertyActionKind::RenameField => {
                let new_name = self.field_state.read(cx).value().to_string();
                let new_name = new_name.trim();
                if new_name.is_empty() {
                    return Err("New field name is required.".to_string());
                }
                if new_name.contains('.') || new_name.contains('$') {
                    return Err("Field name cannot contain '.' or '$'.".to_string());
                }
                let new_path = if self.parent_dot.is_empty() {
                    new_name.to_string()
                } else {
                    format!("{}.{}", self.parent_dot, new_name)
                };
                Ok(doc! { "$rename": { self.path_dot.clone(): new_path } })
            }
            PropertyActionKind::RemoveField => Ok(doc! { "$unset": { self.path_dot.clone(): "" } }),
            PropertyActionKind::AddElement => {
                let value = self.parse_value(cx)?;
                Ok(doc! { "$push": { self.array_dot.clone(): value } })
            }
            PropertyActionKind::RemoveMatchingValues => {
                let value = self.parse_value(cx)?;
                Ok(doc! { "$pull": { self.array_dot.clone(): value } })
            }
        }
    }

    fn scope_button(&self, view: Entity<Self>, cx: &mut Context<Self>) -> impl IntoElement {
        let variant = ButtonCustomVariant::new(cx)
            .color(colors::bg_button_secondary().into())
            .foreground(colors::text_primary().into())
            .border(colors::border_subtle().into())
            .hover(colors::bg_button_secondary_hover().into())
            .active(colors::bg_button_secondary_hover().into())
            .shadow(false);

        MenuButton::new("property-scope")
            .compact()
            .label(self.effective_scope().label())
            .dropdown_caret(true)
            .custom(variant)
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
            )
            .disabled(!self.allow_bulk)
            .dropdown_menu_with_anchor(Corner::BottomLeft, {
                let view = view.clone();
                move |menu, _window, _cx| {
                    menu.item(PopupMenuItem::new(UpdateScope::CurrentDocument.label()).on_click({
                        let view = view.clone();
                        move |_, _, cx| {
                            view.update(cx, |this, cx| {
                                this.set_scope(UpdateScope::CurrentDocument, cx);
                            });
                        }
                    }))
                    .item(PopupMenuItem::new(UpdateScope::MatchQuery.label()).on_click({
                        let view = view.clone();
                        move |_, _, cx| {
                            view.update(cx, |this, cx| {
                                this.set_scope(UpdateScope::MatchQuery, cx);
                            });
                        }
                    }))
                    .item(
                        PopupMenuItem::new(UpdateScope::AllDocuments.label()).on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_scope(UpdateScope::AllDocuments, cx);
                                });
                            }
                        }),
                    )
                }
            })
    }

    fn type_button(&self, view: Entity<Self>, cx: &mut Context<Self>) -> impl IntoElement {
        let variant = ButtonCustomVariant::new(cx)
            .color(colors::bg_button_secondary().into())
            .foreground(colors::text_primary().into())
            .border(colors::border_subtle().into())
            .hover(colors::bg_button_secondary_hover().into())
            .active(colors::bg_button_secondary_hover().into())
            .shadow(false);

        MenuButton::new("property-type")
            .compact()
            .label(self.value_type.label())
            .dropdown_caret(true)
            .custom(variant)
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
            )
            .dropdown_menu_with_anchor(Corner::BottomLeft, {
                let view = view.clone();
                move |menu, _window, _cx| {
                    let mut menu = menu;
                    for kind in [
                        ValueType::Document,
                        ValueType::Array,
                        ValueType::ObjectId,
                        ValueType::String,
                        ValueType::Bool,
                        ValueType::Int32,
                        ValueType::Int64,
                        ValueType::Double,
                        ValueType::Date,
                        ValueType::Null,
                    ] {
                        menu = menu.item(PopupMenuItem::new(kind.label()).on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_value_type(kind, window, cx);
                                });
                            }
                        }));
                    }
                    menu
                }
            })
    }
}

impl Render for PropertyActionDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();

        let show_value = matches!(
            self.action,
            PropertyActionKind::EditValue
                | PropertyActionKind::AddField
                | PropertyActionKind::AddElement
                | PropertyActionKind::RemoveMatchingValues
        );

        let show_type = show_value;

        let show_field_input =
            matches!(self.action, PropertyActionKind::AddField | PropertyActionKind::RenameField);
        let show_field_readonly = matches!(
            self.action,
            PropertyActionKind::EditValue
                | PropertyActionKind::RemoveField
                | PropertyActionKind::RenameField
                | PropertyActionKind::AddElement
                | PropertyActionKind::RemoveMatchingValues
        );

        let action_label = match self.action {
            PropertyActionKind::EditValue => "Set Value",
            PropertyActionKind::AddField => "Add Field",
            PropertyActionKind::RenameField => "Rename",
            PropertyActionKind::RemoveField => "Remove",
            PropertyActionKind::AddElement => "Add Element",
            PropertyActionKind::RemoveMatchingValues => "Remove",
        };

        let status_text = if let Some(error) = &self.error_message {
            (error.clone(), colors::text_error())
        } else if self.updating {
            ("Applying update...".to_string(), colors::text_muted())
        } else if !self.allow_bulk {
            ("Scope locked to current document.".to_string(), colors::text_muted())
        } else {
            ("".to_string(), colors::text_muted())
        };

        let scope_row = if show_type {
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .child(div().text_xs().text_color(colors::text_secondary()).child("Type"))
                        .child(self.type_button(view.clone(), cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .child(div().text_xs().text_color(colors::text_secondary()).child("Scope"))
                        .child(self.scope_button(view.clone(), cx)),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .child(div().text_xs().text_color(colors::text_secondary()).child("Scope"))
                .child(self.scope_button(view.clone(), cx))
                .into_any_element()
        };

        let field_row = if show_field_readonly {
            let label = match self.action {
                PropertyActionKind::AddElement | PropertyActionKind::RemoveMatchingValues => {
                    "Array"
                }
                _ => "Field",
            };
            div()
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .child(div().text_xs().text_color(colors::text_secondary()).child(label))
                .child(Input::new(&self.field_display_state).disabled(true))
                .into_any_element()
        } else {
            div().into_any_element()
        };

        let field_input = if show_field_input {
            let label = if self.action == PropertyActionKind::RenameField {
                "New Field Name"
            } else {
                "Field"
            };
            div()
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .child(div().text_xs().text_color(colors::text_secondary()).child(label))
                .child(Input::new(&self.field_state).font_family(crate::theme::fonts::mono()))
                .into_any_element()
        } else {
            div().into_any_element()
        };

        let value_row = if show_value {
            div()
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .child(div().text_xs().text_color(colors::text_secondary()).child("Value"))
                .child(
                    Input::new(&self.value_state)
                        .font_family(crate::theme::fonts::mono())
                        .h(px(160.0))
                        .w_full(),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .p(spacing::md())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_xs().text_color(colors::text_secondary()).child("Parent"))
                    .child(Input::new(&self.parent_state).disabled(true)),
            )
            .child(field_row)
            .child(field_input)
            .child(scope_row)
            .child(value_row)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt(spacing::xs())
                    .child(
                        div()
                            .min_h(px(18.0))
                            .text_sm()
                            .text_color(status_text.1)
                            .child(status_text.0),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(Button::new("cancel-property").label("Cancel").on_click(
                                |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    window.close_dialog(cx);
                                },
                            ))
                            .child(
                                Button::new("apply-property")
                                    .primary()
                                    .label(action_label)
                                    .disabled(self.updating)
                                    .on_click({
                                        let view = view.clone();
                                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                            view.update(cx, |this, cx| {
                                                this.submit(window, cx);
                                            });
                                        }
                                    }),
                            ),
                    ),
            )
    }
}
