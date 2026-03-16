use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::button::{Button as MenuButton, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::calendar::{Calendar, CalendarEvent, CalendarState, Date};
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::switch::Switch;
use gpui_component::tooltip::Tooltip;
use gpui_component::{Icon, IconName, Sizable as _, Size, StyledExt as _, WindowExt as _};
use mongodb::bson::{Bson, Document};

use crate::components::{Button, open_confirm_dialog};
use crate::state::{AppCommands, AppState, SessionKey, StatusMessage};
use crate::theme::{borders, fonts, spacing};

use super::drag::{DragField, DragValue};
use super::types::{
    Combinator, ConditionValue, DropTarget, FieldType, FilterCondition, FilterNode, FilterOperator,
    FilterTree, drag_value_for, is_valid_value_for_field_type,
};

const SAMPLE_SIZE: i64 = 500;
const MAX_SUGGESTIONS: usize = 7;
const LIST_COLLAPSE_THRESHOLD: usize = 8;
const BULK_EDITOR_HEIGHT: f32 = 280.0;

#[derive(Clone)]
pub struct FieldSuggestion {
    pub path: String,
    pub field_type: FieldType,
    pub sample_value: Option<String>,
}

#[derive(Clone)]
struct DraggedFilterNode {
    session_key: SessionKey,
    node_id: u64,
    label: String,
}

struct DraggedFilterPreview {
    label: String,
}

impl Render for DraggedFilterPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(spacing::sm())
            .py(spacing::xs())
            .rounded(borders::radius_sm())
            .bg(cx.theme().primary)
            .text_color(cx.theme().primary_foreground)
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .shadow_md()
            .child(self.label.clone())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CalendarTarget {
    Single(u64),
    RangeStart(u64),
    RangeEnd(u64),
}

struct ConditionInputs {
    field_state: Entity<InputState>,
    scalar_state: Entity<InputState>,
    list_input_state: Entity<InputState>,
    range_start_state: Entity<InputState>,
    range_end_state: Entity<InputState>,
}

type BulkValueSave = Rc<dyn Fn(Vec<String>, &mut Window, &mut App)>;

struct BulkValueEditor {
    field_type: FieldType,
    input_state: Entity<InputState>,
    initial_count: usize,
    on_save: BulkValueSave,
}

pub struct FilterBuilderPanel {
    state: Entity<AppState>,
    session_key: SessionKey,
    filter_input: Entity<InputState>,
    tree: FilterTree,
    applied_tree: FilterTree,
    unsupported_reason: Option<String>,
    suggestions: Vec<FieldSuggestion>,
    suggestions_loaded: bool,
    condition_inputs: HashMap<u64, ConditionInputs>,
    active_suggestion_row: Option<u64>,
    suppress_suggestion_row: Option<u64>,
    drag_source: Option<u64>,
    calendar_target: Option<CalendarTarget>,
    calendar_state: Option<Entity<CalendarState>>,
    _subscriptions: Vec<Subscription>,
}

impl FilterBuilderPanel {
    pub fn new(
        state: Entity<AppState>,
        session_key: SessionKey,
        filter_input: Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let blank = Self::blank_tree();
        let mut panel = Self {
            state,
            session_key,
            filter_input,
            tree: blank.clone(),
            applied_tree: blank,
            unsupported_reason: None,
            suggestions: Vec::new(),
            suggestions_loaded: false,
            condition_inputs: HashMap::new(),
            active_suggestion_row: None,
            suppress_suggestion_row: None,
            drag_source: None,
            calendar_target: None,
            calendar_state: None,
            _subscriptions: Vec::new(),
        };
        panel.load_suggestions(cx);
        panel.rebuild_condition_inputs(window, cx);
        panel
    }

    pub fn populate_from_document(
        &mut self,
        doc: &Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match FilterTree::from_document(doc) {
            Ok(tree) => {
                self.unsupported_reason = None;
                self.tree = tree.clone();
                self.applied_tree = tree;
                self.rebuild_condition_inputs(window, cx);
            }
            Err(err) => {
                self.unsupported_reason = Some(err.reason);
                let blank = Self::blank_tree();
                self.tree = blank.clone();
                self.applied_tree = blank;
                self.rebuild_condition_inputs(window, cx);
            }
        }
        cx.notify();
    }

    fn blank_tree() -> FilterTree {
        FilterTree::new()
    }

    fn load_suggestions(&mut self, cx: &mut Context<Self>) {
        let (client, database, collection, manager) = {
            let state_ref = self.state.read(cx);
            let conn_id = self.session_key.connection_id;
            let Some(conn) = state_ref.active_connection_by_id(conn_id) else {
                return;
            };
            (
                conn.client.clone(),
                self.session_key.database.clone(),
                self.session_key.collection.clone(),
                state_ref.connection_manager(),
            )
        };

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move { manager.sample_documents(&client, &database, &collection, SAMPLE_SIZE) }
        });

        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result: Result<Vec<Document>, crate::error::Error> = task.await;
            let _ = cx.update(|cx| {
                let _ = view.update(cx, |this, cx| {
                    if let Ok(documents) = result {
                        this.suggestions = build_typed_suggestions(&documents);
                        this.suggestions_loaded = true;
                        cx.notify();
                    }
                });
            });
        })
        .detach();
    }

    fn rebuild_condition_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.condition_inputs.clear();
        self._subscriptions.clear();
        for condition in self.tree.conditions() {
            self.create_condition_inputs(condition.id, window, cx);
        }
    }

    fn create_condition_inputs(
        &mut self,
        condition_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let field_state =
            cx.new(|cx| InputState::new(window, cx).placeholder("Field path").clean_on_escape());
        let scalar_state =
            cx.new(|cx| InputState::new(window, cx).placeholder("Value").clean_on_escape());
        let list_input_state = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Type and press Enter").clean_on_escape()
        });
        let range_start_state =
            cx.new(|cx| InputState::new(window, cx).placeholder("Start").clean_on_escape());
        let range_end_state =
            cx.new(|cx| InputState::new(window, cx).placeholder("End").clean_on_escape());

        let cid = condition_id;

        let field_sub = cx.subscribe_in(
            &field_state,
            window,
            move |panel, state, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let raw = state.read(cx).value().to_string();
                    let suppress_autocomplete = panel.suppress_suggestion_row == Some(cid);
                    if suppress_autocomplete {
                        panel.suppress_suggestion_row = None;
                        panel.active_suggestion_row = None;
                    } else {
                        panel.active_suggestion_row =
                            if raw.trim().is_empty() { None } else { Some(cid) };
                    }
                    let inferred = panel
                        .suggestions
                        .iter()
                        .find(|suggestion| suggestion.path == raw)
                        .map(|suggestion| suggestion.field_type)
                        .unwrap_or_else(|| FieldType::from_field_name(&raw));
                    if let Some(condition) = panel.tree.condition_mut(cid) {
                        condition.field = raw;
                        condition.set_field_type(inferred);
                    }
                    panel.sync_condition_inputs(cid, window, cx);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    if panel.active_suggestion_row == Some(cid)
                        && let Some(suggestion) =
                            panel.matching_suggestions_for_condition(cid, cx).first().cloned()
                    {
                        panel.set_field_from_suggestion(cid, &suggestion, window, cx);
                    }
                }
                InputEvent::Blur => {
                    if panel.active_suggestion_row == Some(cid) {
                        panel.active_suggestion_row = None;
                        cx.notify();
                    }
                }
                _ => {}
            },
        );

        let scalar_sub = cx.subscribe_in(
            &scalar_state,
            window,
            move |panel, state, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(condition) = panel.tree.condition_mut(cid)
                    && let Some(value) = condition.value.scalar_mut()
                {
                    *value = state.read(cx).value().to_string();
                }
            },
        );

        let list_sub = cx.subscribe_in(
            &list_input_state,
            window,
            move |panel, state, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let raw = state.read(cx).value().to_string();
                    let (tokens, remainder) = split_committed_tokens(&raw, false);
                    if !tokens.is_empty() {
                        panel.append_list_tokens(cid, tokens, cx);
                        state.update(cx, |input, cx| {
                            input.set_value(remainder, window, cx);
                        });
                        cx.notify();
                    }
                }
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    panel.commit_list_input(cid, window, cx);
                }
                _ => {}
            },
        );

        let range_start_sub = cx.subscribe_in(
            &range_start_state,
            window,
            move |panel, state, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(condition) = panel.tree.condition_mut(cid)
                    && let Some((start, _)) = condition.value.range_mut()
                {
                    *start = state.read(cx).value().to_string();
                }
            },
        );

        let range_end_sub = cx.subscribe_in(
            &range_end_state,
            window,
            move |panel, state, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(condition) = panel.tree.condition_mut(cid)
                    && let Some((_, end)) = condition.value.range_mut()
                {
                    *end = state.read(cx).value().to_string();
                }
            },
        );

        self._subscriptions.extend([
            field_sub,
            scalar_sub,
            list_sub,
            range_start_sub,
            range_end_sub,
        ]);
        self.condition_inputs.insert(
            condition_id,
            ConditionInputs {
                field_state,
                scalar_state,
                list_input_state,
                range_start_state,
                range_end_state,
            },
        );
        self.sync_condition_inputs(condition_id, window, cx);
    }

    fn sync_condition_inputs(
        &mut self,
        condition_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(condition) = self.tree.condition_mut(condition_id).cloned() else {
            return;
        };
        let Some(inputs) = self.condition_inputs.get(&condition_id) else {
            return;
        };

        let should_sync_field = inputs.field_state.read(cx).value().as_ref() != condition.field;
        if should_sync_field {
            self.suppress_suggestion_row = Some(condition_id);
            inputs.field_state.update(cx, |state, cx| {
                state.set_value(condition.field.clone(), window, cx);
            });
        }

        let scalar_value = condition.value.scalar().unwrap_or("").to_string();
        inputs.scalar_state.update(cx, |state, cx| {
            if state.value().as_ref() != scalar_value {
                state.set_value(scalar_value.clone(), window, cx);
            }
        });

        if !matches!(condition.value, ConditionValue::List(_)) {
            inputs.list_input_state.update(cx, |state, cx| {
                if !state.value().is_empty() {
                    state.set_value(String::new(), window, cx);
                }
            });
        }

        let (range_start, range_end) = condition
            .value
            .range()
            .map(|(start, end)| (start.to_string(), end.to_string()))
            .unwrap_or_default();
        inputs.range_start_state.update(cx, |state, cx| {
            if state.value().as_ref() != range_start {
                state.set_value(range_start.clone(), window, cx);
            }
        });
        inputs.range_end_state.update(cx, |state, cx| {
            if state.value().as_ref() != range_end {
                state.set_value(range_end.clone(), window, cx);
            }
        });
    }

    fn append_list_tokens(
        &mut self,
        condition_id: u64,
        tokens: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if tokens.is_empty() {
            return;
        }
        if let Some(condition) = self.tree.condition_mut(condition_id)
            && let Some(values) = condition.value.list_mut()
        {
            values.extend(tokens);
            cx.notify();
        }
    }

    fn commit_list_input(
        &mut self,
        condition_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(list_input_state) =
            self.condition_inputs.get(&condition_id).map(|inputs| inputs.list_input_state.clone())
        else {
            return;
        };
        let raw = list_input_state.read(cx).value().to_string();
        let (tokens, _) = split_committed_tokens(&raw, true);
        if !tokens.is_empty() {
            self.append_list_tokens(condition_id, tokens, cx);
        }
        list_input_state.update(cx, |state, cx| {
            if !state.value().is_empty() {
                state.set_value(String::new(), window, cx);
            }
        });
    }

    fn add_condition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.tree.add_condition();
        self.create_condition_inputs(id, window, cx);
        cx.notify();
    }

    fn add_condition_from_drag(
        &mut self,
        drag: &DragField,
        target: DropTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.tree.insert_condition_at(target);
        self.create_condition_inputs(id, window, cx);
        if let Some(condition) = self.tree.condition_mut(id) {
            condition.field = drag.path.clone();
            condition.set_field_type(drag.field_type);
            condition.set_operator(FilterOperator::Eq);
            if let Some(value) = &drag.value {
                condition.value = drag_value_for(condition.field_type, condition.operator, value);
            }
        }
        self.active_suggestion_row = None;
        self.sync_condition_inputs(id, window, cx);
        cx.notify();
    }

    fn add_group(&mut self, cx: &mut Context<Self>) {
        self.tree.add_group();
        cx.notify();
    }

    fn duplicate_node(&mut self, node_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(duplicate_id) = self.tree.duplicate_node(node_id) {
            if matches!(self.tree.find_node(duplicate_id), Some(FilterNode::Condition(_))) {
                self.create_condition_inputs(duplicate_id, window, cx);
            } else {
                self.rebuild_condition_inputs(window, cx);
            }
            cx.notify();
        }
    }

    fn remove_node(&mut self, node_id: u64, cx: &mut Context<Self>) {
        self.tree.remove_node(node_id);
        self.condition_inputs.remove(&node_id);
        cx.notify();
    }

    fn clear_list_values(
        &mut self,
        condition_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(condition) = self.tree.condition_mut(condition_id)
            && let Some(values) = condition.value.list_mut()
        {
            values.clear();
        }
        self.sync_condition_inputs(condition_id, window, cx);
        cx.notify();
    }

    fn remove_last_list_value(
        &mut self,
        condition_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(condition) = self.tree.condition_mut(condition_id)
            && let Some(values) = condition.value.list_mut()
        {
            values.pop();
        }
        self.sync_condition_inputs(condition_id, window, cx);
        cx.notify();
    }

    fn open_bulk_value_editor(
        view: Entity<Self>,
        condition: &FilterCondition,
        window: &mut Window,
        cx: &mut App,
    ) {
        let condition_id = condition.id;
        let field_type = condition.field_type;
        let values = condition.value.list().unwrap_or(&[]).to_vec();
        let dialog_view = cx.new(|cx| {
            let input_state = cx.new(|cx| {
                InputState::new(window, cx)
                    .code_editor("text")
                    .line_number(true)
                    .searchable(true)
                    .soft_wrap(false)
                    .default_value(values.join("\n"))
            });

            BulkValueEditor {
                field_type,
                input_state,
                initial_count: values.len(),
                on_save: Rc::new({
                    let view = view.clone();
                    move |next_values, window, cx| {
                        view.update(cx, |this, cx| {
                            if let Some(current) = this.tree.condition_mut(condition_id)
                                && let Some(list) = current.value.list_mut()
                            {
                                *list = next_values;
                            }
                            this.sync_condition_inputs(condition_id, window, cx);
                            cx.notify();
                        });
                    }
                }),
            }
        });

        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("Edit values").w(px(760.0)).child(dialog_view.clone())
        });
    }

    fn set_field_from_suggestion(
        &mut self,
        condition_id: u64,
        suggestion: &FieldSuggestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(condition) = self.tree.condition_mut(condition_id) {
            condition.field = suggestion.path.clone();
            condition.set_field_type(suggestion.field_type);
        }
        self.active_suggestion_row = None;
        self.sync_condition_inputs(condition_id, window, cx);
        cx.notify();
    }

    fn apply_drag_to_field(
        &mut self,
        condition_id: u64,
        drag: &DragField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let should_apply_value = self
            .tree
            .condition_mut(condition_id)
            .map(|condition| !condition.value.has_content())
            .unwrap_or(false);

        if let Some(condition) = self.tree.condition_mut(condition_id) {
            condition.field = drag.path.clone();
            condition.set_field_type(drag.field_type);
        }

        self.active_suggestion_row = None;
        if should_apply_value {
            if let Some(value) = &drag.value {
                self.apply_drag_to_value(condition_id, value, window, cx);
            } else {
                self.sync_condition_inputs(condition_id, window, cx);
                cx.notify();
            }
        } else {
            self.sync_condition_inputs(condition_id, window, cx);
            cx.notify();
        }
    }

    fn apply_drag_to_value(
        &mut self,
        condition_id: u64,
        value: &Bson,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(condition) = self.tree.condition_mut(condition_id) {
            condition.value = drag_value_for(condition.field_type, condition.operator, value);
        }
        self.sync_condition_inputs(condition_id, window, cx);
        cx.notify();
    }

    fn matching_suggestions_for_condition(
        &self,
        condition_id: u64,
        cx: &App,
    ) -> Vec<FieldSuggestion> {
        let Some(inputs) = self.condition_inputs.get(&condition_id) else {
            return Vec::new();
        };
        matching_suggestions(&self.suggestions, inputs.field_state.read(cx).value().as_ref())
    }

    fn clear_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.unsupported_reason = None;
        self.tree = Self::blank_tree();
        self.rebuild_condition_inputs(window, cx);
        cx.notify();
    }

    fn reset_to_applied(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.unsupported_reason = None;
        self.tree = self.applied_tree.clone();
        self.rebuild_condition_inputs(window, cx);
        cx.notify();
    }

    fn start_fresh_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let blank = Self::blank_tree();
        self.unsupported_reason = None;
        self.tree = blank.clone();
        self.applied_tree = blank;
        self.rebuild_condition_inputs(window, cx);
        cx.notify();
    }

    fn is_dirty(&self) -> bool {
        self.unsupported_reason.is_none() && self.tree != self.applied_tree
    }

    fn can_run(&self) -> bool {
        self.unsupported_reason.is_none() && self.tree.validation_error().is_none()
    }

    fn apply_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.unsupported_reason.is_some() {
            return;
        }
        let document = self.tree.to_document();
        let json = self.tree.to_json_string();
        let raw_store = if document.is_empty() { String::new() } else { json.clone() };
        let filter_doc = if document.is_empty() { None } else { Some(document) };

        self.filter_input.update(cx, |state, cx| {
            state.set_value(
                if raw_store.is_empty() { "{}".to_string() } else { json.clone() },
                window,
                cx,
            );
        });

        self.state.update(cx, |state, cx| {
            state.set_filter(&self.session_key, raw_store, filter_doc);
            state.set_status_message(Some(StatusMessage::info("Filter applied")));
            cx.notify();
        });
        self.applied_tree = self.tree.clone();
        AppCommands::load_documents_for_session(self.state.clone(), self.session_key.clone(), cx);
    }

    fn open_json_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let should_overwrite = self.unsupported_reason.is_none();
        let next_value = self.tree.to_json_string();

        if should_overwrite {
            self.filter_input.update(cx, |state, cx| {
                state.set_value(next_value, window, cx);
            });
        }

        let focus = self.filter_input.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.state.update(cx, |state, cx| {
            state.set_filter_builder_open(&self.session_key, false);
            cx.notify();
        });
    }

    fn attempt_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_dirty() {
            self.state.update(cx, |state, cx| {
                state.set_filter_builder_open(&self.session_key, false);
                cx.notify();
            });
            return;
        }

        let state = self.state.clone();
        let session_key = self.session_key.clone();
        open_confirm_dialog(
            window,
            cx,
            "Discard visual draft?",
            "You have unapplied filter builder changes.",
            "Discard",
            true,
            move |_window, cx| {
                state.update(cx, |state, cx| {
                    state.set_filter_builder_open(&session_key, false);
                    cx.notify();
                });
            },
        );
    }

    fn toggle_calendar(
        &mut self,
        target: CalendarTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.calendar_target == Some(target) {
            self.calendar_target = None;
            self.calendar_state = None;
            cx.notify();
            return;
        }

        let calendar_state = cx.new(|cx| CalendarState::new(window, cx));
        let sub = cx.subscribe_in(
            &calendar_state,
            window,
            move |panel: &mut Self, _calendar, event: &CalendarEvent, window, cx| {
                let CalendarEvent::Selected(date) = event;
                let Date::Single(Some(date)) = date else {
                    return;
                };
                let formatted = format!("{}T00:00:00.000Z", date.format("%Y-%m-%d"));
                panel.apply_calendar_value(target, formatted, window, cx);
                panel.calendar_target = None;
                panel.calendar_state = None;
                cx.notify();
            },
        );
        self._subscriptions.push(sub);
        self.calendar_target = Some(target);
        self.calendar_state = Some(calendar_state);
        cx.notify();
    }

    fn apply_calendar_value(
        &mut self,
        target: CalendarTarget,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let condition_id = match target {
            CalendarTarget::Single(id)
            | CalendarTarget::RangeStart(id)
            | CalendarTarget::RangeEnd(id) => id,
        };
        let Some(condition) = self.tree.condition_mut(condition_id) else {
            return;
        };

        match target {
            CalendarTarget::Single(_) => {
                condition.value = ConditionValue::Scalar(value.clone());
            }
            CalendarTarget::RangeStart(_) => {
                if let Some((start, _)) = condition.value.range_mut() {
                    *start = value.clone();
                }
            }
            CalendarTarget::RangeEnd(_) => {
                if let Some((_, end)) = condition.value.range_mut() {
                    *end = value.clone();
                }
            }
        }
        self.sync_condition_inputs(condition_id, window, cx);
    }

    fn handle_field_drop(
        &mut self,
        field: &DragField,
        target: DropTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_condition_from_drag(field, target, window, cx);
    }

    fn handle_field_surface_drop(
        &mut self,
        condition_id: u64,
        field: &DragField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_drag_to_field(condition_id, field, window, cx);
    }

    fn handle_value_surface_drop(
        &mut self,
        condition_id: u64,
        field: &DragField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(value) = &field.value {
            self.apply_drag_to_value(condition_id, value, window, cx);
        }
    }

    fn handle_dragged_value_drop(
        &mut self,
        condition_id: u64,
        value_drag: &DragValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_drag_to_value(condition_id, &value_drag.value, window, cx);
    }

    fn handle_node_drop(
        &mut self,
        node: &DraggedFilterNode,
        target: DropTarget,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if node.session_key == self.session_key && self.tree.move_node(node.node_id, target) {
            self.drag_source = None;
            cx.notify();
        }
    }

    fn handle_merge_drop(
        &mut self,
        dragged: &DraggedFilterNode,
        target_id: u64,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if dragged.session_key == self.session_key
            && self.tree.merge_into_group(dragged.node_id, target_id).is_some()
        {
            self.drag_source = None;
            cx.notify();
        }
    }

    fn handle_group_append_drop(
        &mut self,
        dragged: &DraggedFilterNode,
        group_id: u64,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if dragged.session_key == self.session_key
            && self.tree.add_node_to_group(dragged.node_id, group_id)
        {
            self.drag_source = None;
            cx.notify();
        }
    }

    fn render_root_group(
        &self,
        view: &Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active_count = self.tree.active_condition_count();
        let validation_error = self.tree.validation_error();

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::sm())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(summary_chip(
                        self.tree.combinator.label(),
                        if self.tree.combinator == Combinator::And {
                            cx.theme().primary
                        } else {
                            cx.theme().warning
                        },
                        cx,
                    ))
                    .when(active_count > 0, |this| {
                        this.child(summary_chip(
                            format!(
                                "{active_count} rule{}",
                                if active_count == 1 { "" } else { "s" }
                            ),
                            cx.theme().muted_foreground,
                            cx,
                        ))
                    })
                    .when(self.is_dirty(), |this| this.child(neutral_chip("Draft", cx)))
                    .when(validation_error.is_some(), |this| {
                        this.child(summary_chip("Needs attention", cx.theme().danger, cx))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(self.render_combinator_toggle(None, self.tree.combinator, view, cx))
                    .child(
                        Button::new("root-add-rule")
                            .ghost()
                            .compact()
                            .icon(Icon::new(IconName::Plus).xsmall())
                            .label("Rule")
                            .on_click({
                                let view = view.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    view.update(cx, |this, cx| this.add_condition(window, cx));
                                }
                            }),
                    )
                    .child(
                        Button::new("root-add-group").ghost().compact().label("Group").on_click({
                            let view = view.clone();
                            move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                view.update(cx, |this, cx| this.add_group(cx));
                            }
                        }),
                    ),
            );

        let content = if let Some(reason) = &self.unsupported_reason {
            self.render_unsupported_state(reason, view, window, cx).into_any_element()
        } else {
            self.render_node_list(&self.tree.children, None, true, view, window, cx)
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .rounded(borders::radius_md())
            .border_1()
            .border_color(cx.theme().sidebar_border.opacity(0.7))
            .bg(cx.theme().tab_bar.opacity(0.58))
            .shadow_sm()
            .px(spacing::sm())
            .py(spacing::xs())
            .child(header)
            .child(content)
    }

    fn render_unsupported_state(
        &self,
        reason: &str,
        view: &Entity<Self>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .rounded(borders::radius_md())
            .border_1()
            .border_color(cx.theme().warning.opacity(0.45))
            .bg(cx.theme().warning.opacity(0.09))
            .p(spacing::md())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(spacing::sm())
                    .child(
                        Icon::new(IconName::TriangleAlert).xsmall().text_color(cx.theme().warning),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(cx.theme().foreground)
                                    .child("This filter needs raw JSON"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(reason.to_string()),
                            ),
                    ),
            )
            .child(
                div().text_xs().text_color(cx.theme().muted_foreground).child(
                    "Start a fresh visual draft or keep editing the existing query in JSON.",
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(
                        Button::new("builder-unsupported-json")
                            .compact()
                            .primary()
                            .label("Open JSON")
                            .on_click({
                                let view = view.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    view.update(cx, |this, cx| this.open_json_editor(window, cx));
                                }
                            }),
                    )
                    .child(
                        Button::new("builder-unsupported-fresh")
                            .compact()
                            .ghost()
                            .label("Start fresh")
                            .on_click({
                                let view = view.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    view.update(cx, |this, cx| this.start_fresh_draft(window, cx));
                                }
                            }),
                    ),
            )
    }

    fn render_node_list(
        &self,
        nodes: &[FilterNode],
        parent_group_id: Option<u64>,
        _is_root: bool,
        view: &Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut column = div().flex().flex_col().gap(px(1.0));
        column = column.child(self.render_drop_slot(
            DropTarget { parent_group_id, index: 0 },
            nodes.is_empty(),
            view,
            window,
            cx,
        ));

        for (index, node) in nodes.iter().enumerate() {
            column = column.child(match node {
                FilterNode::Condition(condition) => {
                    self.render_condition_card(condition, true, view, window, cx).into_any_element()
                }
                FilterNode::Group { id, combinator, children } => self
                    .render_group_card(*id, *combinator, children, true, view, window, cx)
                    .into_any_element(),
            });
            column = column.child(self.render_drop_slot(
                DropTarget { parent_group_id, index: index + 1 },
                false,
                view,
                window,
                cx,
            ));
        }

        column
    }

    #[allow(clippy::too_many_arguments)]
    fn render_group_card(
        &self,
        group_id: u64,
        combinator: Combinator,
        children: &[FilterNode],
        can_remove: bool,
        view: &Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let accent =
            if combinator == Combinator::And { cx.theme().primary } else { cx.theme().warning };
        let drag_label = format!("Move {} group", combinator.short_label());
        let append_accent = cx.theme().primary;

        div()
            .id(SharedString::from(format!("group-card-{group_id}")))
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .rounded(borders::radius_md())
            .border_1()
            .border_color(accent.opacity(0.35))
            .bg(accent.opacity(0.06))
            .px(spacing::sm())
            .py(px(4.0))
            .opacity(if self.drag_source == Some(group_id) { 0.55 } else { 1.0 })
            .can_drop({
                let session_key = self.session_key.clone();
                move |value, _window, _cx| {
                    value
                        .downcast_ref::<DraggedFilterNode>()
                        .is_some_and(|drag| {
                            drag.session_key == session_key && drag.node_id != group_id
                        })
                }
            })
            .drag_over::<DraggedFilterNode>(move |style, _drag, _window, _cx| {
                style
                    .border_2()
                    .border_color(append_accent)
                    .bg(append_accent.opacity(0.08))
            })
            .on_drop({
                let view = view.clone();
                move |drag: &DraggedFilterNode, window, cx| {
                    view.update(cx, |this, cx| {
                        this.handle_group_append_drop(drag, group_id, window, cx)
                    });
                }
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(self.render_drag_handle(group_id, drag_label, view, cx))
                            .child(summary_chip(combinator.short_label(), accent, cx))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(cx.theme().foreground)
                                    .child(combinator.label()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.0))
                            .child(self.render_combinator_toggle(Some(group_id), combinator, view, cx))
                            .child(
                                Button::new(("group-add-rule", group_id))
                                    .ghost()
                                    .compact()
                                    .icon(Icon::new(IconName::Plus).xsmall())
                                    .label("Rule")
                                    .on_click({
                                        let view = view.clone();
                                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                            view.update(cx, |this, cx| {
                                                if let Some(id) = this.tree.add_condition_to_group(group_id) {
                                                    this.create_condition_inputs(id, window, cx);
                                                    cx.notify();
                                                }
                                            });
                                        }
                                    }),
                            )
                            .child(
                                Button::new(("group-add-group", group_id))
                                    .ghost()
                                    .compact()
                                    .label("Group")
                                    .on_click({
                                        let view = view.clone();
                                        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                            view.update(cx, |this, cx| {
                                                this.tree.add_group_to_group(group_id);
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(
                                Button::new(("group-duplicate", group_id))
                                    .ghost()
                                    .compact()
                                    .icon(Icon::new(IconName::Copy).xsmall())
                                    .tooltip("Duplicate group")
                                    .on_click({
                                        let view = view.clone();
                                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                            view.update(cx, |this, cx| this.duplicate_node(group_id, window, cx));
                                        }
                                    }),
                            )
                            .when(can_remove, |this| {
                                this.child(
                                    Button::new(("group-remove", group_id))
                                        .ghost()
                                        .compact()
                                        .icon(Icon::new(IconName::Close).xsmall())
                                        .tooltip("Remove group")
                                        .on_click({
                                            let view = view.clone();
                                            move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                                view.update(cx, |this, cx| this.remove_node(group_id, cx));
                                            }
                                        }),
                                )
                            }),
                    ),
            )
            .child(self.render_node_list(children, Some(group_id), false, view, window, cx))
    }

    fn render_condition_card(
        &self,
        condition: &FilterCondition,
        can_remove: bool,
        view: &Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cid = condition.id;
        let Some(inputs) = self.condition_inputs.get(&cid) else {
            return div().into_any_element();
        };
        let validation_error = condition.validation_error();
        let accent = field_type_color(condition.field_type, cx);
        let has_error = validation_error.is_some();
        let drag_label = format!(
            "Move {}",
            if condition.field.trim().is_empty() {
                "rule".to_string()
            } else {
                format!("rule {}", condition.field)
            }
        );
        let badge_tooltip: SharedString =
            hint_for_condition(condition).unwrap_or(field_type_label(condition.field_type)).into();

        let merge_accent = cx.theme().primary;
        let value_is_multiline = condition.value_editor_kind().is_multiline();

        let mut row = div()
            .id(SharedString::from(format!("condition-card-{cid}")))
            .flex()
            .items_center()
            .gap(px(3.0))
            .rounded(borders::radius_sm())
            .px(px(4.0))
            .py(px(2.0))
            .opacity(if self.drag_source == Some(cid) { 0.55 } else { 1.0 })
            .hover(|s| s.bg(cx.theme().list_hover))
            .can_drop({
                let session_key = self.session_key.clone();
                move |value, _window, _cx| {
                    value
                        .downcast_ref::<DraggedFilterNode>()
                        .is_some_and(|drag| drag.session_key == session_key && drag.node_id != cid)
                }
            })
            .drag_over::<DraggedFilterNode>(move |style, _drag, _window, _cx| {
                style.border_2().border_color(merge_accent).bg(merge_accent.opacity(0.08))
            })
            .on_drop({
                let view = view.clone();
                move |drag: &DraggedFilterNode, window, cx| {
                    view.update(cx, |this, cx| this.handle_merge_drop(drag, cid, window, cx));
                }
            });

        if has_error {
            row = row.border_1().border_color(cx.theme().danger.opacity(0.4));
        }

        row = row
            .child(self.render_drag_handle(cid, drag_label, view, cx))
            .child(type_badge_compact(("type-badge", cid), condition.field_type, badge_tooltip, cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(60.0))
                    .can_drop(|value, _window, _cx| value.downcast_ref::<DragField>().is_some())
                    .drag_over::<DragField>(move |style, _drag, _window, _cx| {
                        style.bg(accent.opacity(0.12)).rounded(borders::radius_sm())
                    })
                    .on_drop({
                        let view = view.clone();
                        move |drag: &DragField, window, cx| {
                            view.update(cx, |this, cx| {
                                this.handle_field_surface_drop(cid, drag, window, cx);
                            });
                        }
                    })
                    .child(
                        Input::new(&inputs.field_state)
                            .small()
                            .appearance(false)
                            .font_family(fonts::mono())
                            .w_full(),
                    ),
            )
            .child(self.render_operator_dropdown(condition, view, window, cx));

        if !value_is_multiline {
            row = row.child(
                div()
                    .flex_1()
                    .min_w(px(50.0))
                    .can_drop(|value, _window, _cx| {
                        value.downcast_ref::<DragField>().is_some()
                            || value.downcast_ref::<DragValue>().is_some()
                    })
                    .drag_over::<DragField>(move |style, _drag, _window, _cx| {
                        style.bg(accent.opacity(0.08)).rounded(borders::radius_sm())
                    })
                    .drag_over::<DragValue>(move |style, _drag, _window, _cx| {
                        style.bg(accent.opacity(0.08)).rounded(borders::radius_sm())
                    })
                    .on_drop({
                        let view = view.clone();
                        move |drag: &DragField, window, cx| {
                            view.update(cx, |this, cx| {
                                this.handle_value_surface_drop(cid, drag, window, cx);
                            });
                        }
                    })
                    .on_drop({
                        let view = view.clone();
                        move |drag: &DragValue, window, cx| {
                            view.update(cx, |this, cx| {
                                this.handle_dragged_value_drop(cid, drag, window, cx);
                            });
                        }
                    })
                    .child(self.render_value_editor(condition, inputs, view, window, cx)),
            );
        }

        row = row.child(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(1.0))
                .child(
                    Button::new(("condition-duplicate", cid))
                        .ghost()
                        .compact()
                        .icon(Icon::new(IconName::Copy).xsmall())
                        .tooltip("Duplicate rule")
                        .on_click({
                            let view = view.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                view.update(cx, |this, cx| this.duplicate_node(cid, window, cx));
                            }
                        }),
                )
                .when(can_remove, |this| {
                    this.child(
                        Button::new(("condition-remove", cid))
                            .ghost()
                            .compact()
                            .icon(Icon::new(IconName::Close).xsmall())
                            .tooltip("Remove rule")
                            .on_click({
                                let view = view.clone();
                                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                    view.update(cx, |this, cx| this.remove_node(cid, cx));
                                }
                            }),
                    )
                }),
        );

        let mut card = div().flex().flex_col().gap(px(2.0)).child(row);

        if value_is_multiline {
            card = card.child(
                div()
                    .pl(px(22.0))
                    .can_drop(|value, _window, _cx| {
                        value.downcast_ref::<DragField>().is_some()
                            || value.downcast_ref::<DragValue>().is_some()
                    })
                    .drag_over::<DragField>(move |style, _drag, _window, _cx| {
                        style.bg(accent.opacity(0.08)).rounded(borders::radius_sm())
                    })
                    .drag_over::<DragValue>(move |style, _drag, _window, _cx| {
                        style.bg(accent.opacity(0.08)).rounded(borders::radius_sm())
                    })
                    .on_drop({
                        let view = view.clone();
                        move |drag: &DragField, window, cx| {
                            view.update(cx, |this, cx| {
                                this.handle_value_surface_drop(cid, drag, window, cx);
                            });
                        }
                    })
                    .on_drop({
                        let view = view.clone();
                        move |drag: &DragValue, window, cx| {
                            view.update(cx, |this, cx| {
                                this.handle_dragged_value_drop(cid, drag, window, cx);
                            });
                        }
                    })
                    .child(self.render_value_editor(condition, inputs, view, window, cx)),
            );
        }

        if has_error && let Some(error) = validation_error {
            card = card.child(
                div()
                    .pl(px(22.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(Icon::new(IconName::CircleX).xsmall())
                    .child(error),
            );
        }

        if self.active_suggestion_row == Some(cid)
            && let Some(popup) = self.render_suggestions_popup(cid, view, window, cx)
        {
            card = card.child(div().pl(px(22.0)).child(popup));
        }

        card.into_any_element()
    }

    fn render_value_editor(
        &self,
        condition: &FilterCondition,
        inputs: &ConditionInputs,
        view: &Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match condition.value_editor_kind() {
            super::types::ValueEditorKind::None => div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("No value needed")
                .into_any_element(),
            super::types::ValueEditorKind::Toggle => {
                self.render_boolean_editor(condition, view, cx).into_any_element()
            }
            super::types::ValueEditorKind::Single => self
                .render_single_value_editor(condition, inputs, view, window, cx)
                .into_any_element(),
            super::types::ValueEditorKind::List => self
                .render_list_value_editor(condition, inputs, view, window, cx)
                .into_any_element(),
            super::types::ValueEditorKind::Range => self
                .render_range_value_editor(condition, inputs, view, window, cx)
                .into_any_element(),
        }
    }

    fn render_boolean_editor(
        &self,
        condition: &FilterCondition,
        view: &Entity<Self>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = condition.value.bool().unwrap_or(true);
        div()
            .flex()
            .items_center()
            .gap(spacing::xs())
            .child(Switch::new(("condition-bool", condition.id)).checked(current).small().on_click(
                {
                    let view = view.clone();
                    let cid = condition.id;
                    move |checked, _window, cx| {
                        view.update(cx, |this, cx| {
                            if let Some(cond) = this.tree.condition_mut(cid) {
                                cond.value = ConditionValue::Bool(*checked);
                            }
                            cx.notify();
                        });
                    }
                },
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(if current {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(if current { "true" } else { "false" }),
            )
    }

    fn render_single_value_editor(
        &self,
        condition: &FilterCondition,
        inputs: &ConditionInputs,
        view: &Entity<Self>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut input_row = div().flex().items_center().gap(spacing::xs());

        if condition.field_type == FieldType::Number || condition.operator == FilterOperator::Size {
            input_row = input_row
                .child(NumberInput::new(&inputs.scalar_state).small().appearance(false).flex_1());
        } else {
            input_row = input_row.child(
                Input::new(&inputs.scalar_state)
                    .small()
                    .appearance(false)
                    .font_family(if condition.field_type == FieldType::ObjectId {
                        fonts::mono()
                    } else {
                        fonts::ui()
                    })
                    .w_full(),
            );
        }

        if condition.field_type == FieldType::DateTime {
            input_row = input_row.child(
                Button::new(("calendar-open-single", condition.id))
                    .ghost()
                    .compact()
                    .icon(Icon::new(IconName::Calendar).xsmall())
                    .on_click({
                        let view = view.clone();
                        let cid = condition.id;
                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                            view.update(cx, |this, cx| {
                                this.toggle_calendar(CalendarTarget::Single(cid), window, cx);
                            });
                        }
                    }),
            );
        }

        let mut col = div().flex().flex_col().gap(px(2.0)).child(input_row);
        if self.calendar_target == Some(CalendarTarget::Single(condition.id))
            && let Some(calendar_state) = &self.calendar_state
        {
            col = col.child(render_calendar_panel(calendar_state, cx));
        }
        col
    }

    fn render_list_value_editor(
        &self,
        condition: &FilterCondition,
        inputs: &ConditionInputs,
        view: &Entity<Self>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let values = condition.value.list().unwrap_or(&[]);
        let cid = condition.id;
        let mut column = div().flex().flex_col().gap(px(3.0));

        if values.len() > LIST_COLLAPSE_THRESHOLD {
            let first =
                values.first().map(|value| compact_value_preview(value, condition.field_type));
            let last =
                values.last().map(|value| compact_value_preview(value, condition.field_type));
            let hidden = values.len().saturating_sub(2);
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .rounded(borders::radius_sm())
                    .border_1()
                    .border_color(cx.theme().sidebar_border.opacity(0.78))
                    .bg(cx.theme().secondary.opacity(0.18))
                    .px(spacing::xs())
                    .py(spacing::xs())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(spacing::sm())
                            .child(neutral_chip(
                                list_count_label(values.len(), condition.field_type),
                                cx,
                            ))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(2.0))
                                    .child(
                                        Button::new(("list-remove-last", cid))
                                            .ghost()
                                            .compact()
                                            .label("Remove last")
                                            .on_click({
                                                let view = view.clone();
                                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                                    view.update(cx, |this, cx| {
                                                        this.remove_last_list_value(cid, window, cx);
                                                    });
                                                }
                                            }),
                                    )
                                    .child(
                                        Button::new(("list-edit", cid))
                                            .ghost()
                                            .compact()
                                            .label("Edit values")
                                            .on_click({
                                                let view = view.clone();
                                                let condition = condition.clone();
                                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                                    FilterBuilderPanel::open_bulk_value_editor(
                                                        view.clone(),
                                                        &condition,
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            }),
                                    )
                                    .child(
                                        Button::new(("list-clear-all", cid))
                                            .ghost()
                                            .compact()
                                            .label("Clear")
                                            .on_click({
                                                let view = view.clone();
                                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                                    view.update(cx, |this, cx| {
                                                        this.clear_list_values(cid, window, cx);
                                                    });
                                                }
                                            }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(spacing::xs())
                            .flex_wrap()
                            .when_some(first, |this, first| {
                                this.child(render_preview_value_chip("First", first, cx))
                            })
                            .when(values.len() > 2, |this| {
                                this.child(neutral_chip(format!("{hidden} hidden"), cx))
                            })
                            .when(values.len() > 1, |this| {
                                this.when_some(last, |this, last| {
                                    this.child(render_preview_value_chip("Last", last, cx))
                                })
                            }),
                    ),
            );
        } else if !values.is_empty() {
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(spacing::sm())
                            .child(div().flex().items_center().gap(px(6.0)).flex_wrap().child(
                                neutral_chip(
                                    list_count_label(values.len(), condition.field_type),
                                    cx,
                                ),
                            ))
                            .child(
                                Button::new(("list-edit-inline", cid))
                                    .ghost()
                                    .compact()
                                    .label("Edit values")
                                    .on_click({
                                        let view = view.clone();
                                        let condition = condition.clone();
                                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                            FilterBuilderPanel::open_bulk_value_editor(
                                                view.clone(),
                                                &condition,
                                                window,
                                                cx,
                                            );
                                        }
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .max_h(px(148.0))
                            .overflow_y_scrollbar()
                            .rounded(borders::radius_sm())
                            .border_1()
                            .border_color(cx.theme().sidebar_border.opacity(0.55))
                            .bg(cx.theme().sidebar.opacity(0.35))
                            .px(spacing::xs())
                            .py(spacing::xs())
                            .child(div().flex().items_center().gap(px(6.0)).flex_wrap().children(
                                values.iter().enumerate().map(|(index, value)| {
                                    render_token_chip(
                                        SharedString::from(format!("token-{cid}-{index}")),
                                        compact_value_preview(value, condition.field_type),
                                        {
                                            let view = view.clone();
                                            move |window: &mut Window, cx: &mut App| {
                                                view.update(cx, |this, cx| {
                                                    if let Some(condition) =
                                                        this.tree.condition_mut(cid)
                                                        && let Some(list) =
                                                            condition.value.list_mut()
                                                        && index < list.len()
                                                    {
                                                        list.remove(index);
                                                        this.sync_condition_inputs(cid, window, cx);
                                                        cx.notify();
                                                    }
                                                });
                                            }
                                        },
                                        cx,
                                    )
                                }),
                            )),
                    ),
            );
        }

        column.child(
            div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .child(
                    Input::new(&inputs.list_input_state)
                        .small()
                        .appearance(false)
                        .font_family(fonts::ui())
                        .w_full(),
                )
                .child(Button::new(("list-commit", cid)).ghost().compact().label("Add").on_click(
                    {
                        let view = view.clone();
                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                            view.update(cx, |this, cx| this.commit_list_input(cid, window, cx));
                        }
                    },
                )),
        )
    }

    fn render_range_value_editor(
        &self,
        condition: &FilterCondition,
        inputs: &ConditionInputs,
        view: &Entity<Self>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let use_number = condition.field_type == FieldType::Number;
        let use_calendar = condition.field_type == FieldType::DateTime;
        let cid = condition.id;

        let start = if use_number {
            NumberInput::new(&inputs.range_start_state)
                .small()
                .appearance(false)
                .flex_1()
                .into_any_element()
        } else {
            Input::new(&inputs.range_start_state)
                .small()
                .appearance(false)
                .w_full()
                .into_any_element()
        };
        let end = if use_number {
            NumberInput::new(&inputs.range_end_state)
                .small()
                .appearance(false)
                .flex_1()
                .into_any_element()
        } else {
            Input::new(&inputs.range_end_state)
                .small()
                .appearance(false)
                .w_full()
                .into_any_element()
        };

        let mut row = div()
            .flex()
            .items_center()
            .gap(spacing::xs())
            .child(div().flex_1().min_w(px(0.0)).child(start))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("to"))
            .child(div().flex_1().min_w(px(0.0)).child(end));

        if use_calendar {
            row = row
                .child(
                    Button::new(("range-calendar-start", cid))
                        .ghost()
                        .compact()
                        .icon(Icon::new(IconName::Calendar).xsmall())
                        .tooltip("Pick start date")
                        .on_click({
                            let view = view.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                view.update(cx, |this, cx| {
                                    this.toggle_calendar(
                                        CalendarTarget::RangeStart(cid),
                                        window,
                                        cx,
                                    );
                                });
                            }
                        }),
                )
                .child(
                    Button::new(("range-calendar-end", cid))
                        .ghost()
                        .compact()
                        .icon(Icon::new(IconName::Calendar).xsmall())
                        .tooltip("Pick end date")
                        .on_click({
                            let view = view.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                view.update(cx, |this, cx| {
                                    this.toggle_calendar(CalendarTarget::RangeEnd(cid), window, cx);
                                });
                            }
                        }),
                );
        }

        let mut col = div().flex().flex_col().gap(px(2.0)).child(row);
        if use_calendar
            && matches!(
                self.calendar_target,
                Some(CalendarTarget::RangeStart(id) | CalendarTarget::RangeEnd(id)) if id == cid
            )
            && let Some(calendar_state) = &self.calendar_state
        {
            col = col.child(render_calendar_panel(calendar_state, cx));
        }
        col
    }

    fn render_operator_dropdown(
        &self,
        condition: &FilterCondition,
        view: &Entity<Self>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let label = condition.operator.label_for(condition.field_type);
        let field_type = condition.field_type;
        let operators = field_type.available_operators().to_vec();
        styled_dropdown_button(("op-select", condition.id), label, cx).dropdown_menu_with_anchor(
            Corner::BottomLeft,
            {
                let view = view.clone();
                let cid = condition.id;
                move |menu: PopupMenu, _window, _cx| {
                    let mut menu = menu;
                    for operator in &operators {
                        let operator_value = *operator;
                        menu = menu.item(
                            PopupMenuItem::new(operator_value.label_for(field_type)).on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        if let Some(cond) = this.tree.condition_mut(cid) {
                                            cond.set_operator(operator_value);
                                        }
                                        this.sync_condition_inputs(cid, window, cx);
                                        cx.notify();
                                    });
                                }
                            }),
                        );
                    }
                    menu
                }
            },
        )
    }

    fn render_suggestions_popup(
        &self,
        condition_id: u64,
        view: &Entity<Self>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let matches = self.matching_suggestions_for_condition(condition_id, cx);
        if matches.is_empty() {
            return None;
        }

        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .rounded(borders::radius_sm())
            .border_1()
            .border_color(cx.theme().sidebar_border.opacity(0.75))
            .bg(cx.theme().popover)
            .shadow_md()
            .p(px(4.0))
            .max_h(px(196.0))
            .overflow_y_scrollbar();

        for suggestion in matches {
            let path = suggestion.path.clone();
            let sample = suggestion.sample_value.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("field-suggestion-{condition_id}-{path}")))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(spacing::sm())
                    .rounded(borders::radius_sm())
                    .px(spacing::sm())
                    .py(px(5.0))
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().secondary.opacity(0.8)))
                    .on_mouse_down(MouseButton::Left, {
                        let view = view.clone();
                        let suggestion = suggestion.clone();
                        move |_, window, cx| {
                            view.update(cx, |this, cx| {
                                this.set_field_from_suggestion(
                                    condition_id,
                                    &suggestion,
                                    window,
                                    cx,
                                );
                            });
                        }
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(1.0))
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(fonts::mono())
                                    .text_color(cx.theme().foreground)
                                    .child(path),
                            )
                            .when_some(sample, |this, sample| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(sample),
                                )
                            }),
                    )
                    .child(type_badge(suggestion.field_type, cx)),
            );
        }

        Some(list.into_any_element())
    }

    fn render_drop_slot(
        &self,
        target: DropTarget,
        emphasize: bool,
        view: &Entity<Self>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let target_key = SharedString::from(format!(
            "builder-drop-slot-{}-{}",
            target.parent_group_id.unwrap_or(0),
            target.index
        ));
        let accent = cx.theme().primary;

        if !emphasize {
            return div()
                .id(target_key)
                .h(px(6.0))
                .mx(spacing::sm())
                .rounded_full()
                .can_drop({
                    let session_key = self.session_key.clone();
                    move |value, _window, _cx| {
                        value.downcast_ref::<DragField>().is_some()
                            || value
                                .downcast_ref::<DraggedFilterNode>()
                                .is_some_and(|drag| drag.session_key == session_key)
                    }
                })
                .drag_over::<DragField>(move |style, _drag, _window, _cx| {
                    style.bg(accent.opacity(0.5))
                })
                .drag_over::<DraggedFilterNode>(move |style, _drag, _window, _cx| {
                    style.bg(accent.opacity(0.5))
                })
                .on_drop({
                    let view = view.clone();
                    move |drag: &DragField, window, cx| {
                        view.update(cx, |this, cx| {
                            this.handle_field_drop(drag, target, window, cx)
                        });
                    }
                })
                .on_drop({
                    let view = view.clone();
                    move |drag: &DraggedFilterNode, window, cx| {
                        view.update(cx, |this, cx| this.handle_node_drop(drag, target, window, cx));
                    }
                });
        }

        let resting_border = cx.theme().sidebar_border.opacity(0.88);
        let resting_bg = cx.theme().secondary.opacity(0.26);
        let resting_text = cx.theme().foreground.opacity(0.82);

        div()
            .id(target_key)
            .flex()
            .items_center()
            .justify_center()
            .h(px(34.0))
            .rounded(borders::radius_sm())
            .border_1()
            .border_color(resting_border)
            .bg(resting_bg)
            .text_color(resting_text)
            .can_drop({
                let session_key = self.session_key.clone();
                move |value, _window, _cx| {
                    value.downcast_ref::<DragField>().is_some()
                        || value
                            .downcast_ref::<DraggedFilterNode>()
                            .is_some_and(|drag| drag.session_key == session_key)
                }
            })
            .drag_over::<DragField>(move |style, _drag, _window, _cx| {
                style.border_color(accent).bg(accent.opacity(0.2))
            })
            .drag_over::<DraggedFilterNode>(move |style, _drag, _window, _cx| {
                style.border_color(accent).bg(accent.opacity(0.2))
            })
            .on_drop({
                let view = view.clone();
                move |drag: &DragField, window, cx| {
                    view.update(cx, |this, cx| this.handle_field_drop(drag, target, window, cx));
                }
            })
            .on_drop({
                let view = view.clone();
                move |drag: &DraggedFilterNode, window, cx| {
                    view.update(cx, |this, cx| this.handle_node_drop(drag, target, window, cx));
                }
            })
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(resting_text)
                    .child("Drop a field, rule, or group here"),
            )
    }

    fn render_drag_handle(
        &self,
        node_id: u64,
        label: String,
        view: &Entity<Self>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(("builder-drag-handle", node_id))
            .flex()
            .items_center()
            .justify_center()
            .size(px(18.0))
            .rounded(borders::radius_sm())
            .cursor_move()
            .text_color(cx.theme().muted_foreground)
            .hover(|style| style.bg(cx.theme().list_hover).text_color(cx.theme().foreground))
            .on_drag(
                DraggedFilterNode {
                    session_key: self.session_key.clone(),
                    node_id,
                    label: label.clone(),
                },
                {
                    let view = view.clone();
                    move |drag: &DraggedFilterNode, _position, _window, cx| {
                        cx.stop_propagation();
                        view.update(cx, |this, _cx| {
                            this.drag_source = Some(node_id);
                        });
                        cx.new(|_| DraggedFilterPreview { label: drag.label.clone() })
                    }
                },
            )
            .child(Icon::new(IconName::ChevronsUpDown).xsmall())
    }

    fn render_combinator_toggle(
        &self,
        group_id: Option<u64>,
        current: Combinator,
        view: &Entity<Self>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .rounded(borders::radius_sm())
            .bg(cx.theme().sidebar.opacity(0.65))
            .p(px(2.0))
            .child(toggle_chip("$and", current == Combinator::And, cx).on_click({
                let view = view.clone();
                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    view.update(cx, |this, cx| {
                        match group_id {
                            Some(group_id) => {
                                if let Some(combinator) = this.tree.group_combinator_mut(group_id) {
                                    *combinator = Combinator::And;
                                }
                            }
                            None => this.tree.combinator = Combinator::And,
                        }
                        cx.notify();
                    });
                }
            }))
            .child(toggle_chip("$or", current == Combinator::Or, cx).on_click({
                let view = view.clone();
                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    view.update(cx, |this, cx| {
                        match group_id {
                            Some(group_id) => {
                                if let Some(combinator) = this.tree.group_combinator_mut(group_id) {
                                    *combinator = Combinator::Or;
                                }
                            }
                            None => this.tree.combinator = Combinator::Or,
                        }
                        cx.notify();
                    });
                }
            }))
    }
}

impl Render for FilterBuilderPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag() {
            self.drag_source = None;
        }

        let view = cx.entity();
        let validation_error = self.tree.validation_error();
        let dirty = self.is_dirty();
        let can_run = self.can_run();

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::sm())
            .px(spacing::sm())
            .py(spacing::xs())
            .border_b_1()
            .border_color(cx.theme().sidebar_border.opacity(0.65))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child("Filter Builder"),
                    )
                    .when(dirty, |this| this.child(neutral_chip("Draft", cx))),
            )
            .child(
                Button::new("close-filter-builder")
                    .ghost()
                    .compact()
                    .icon(Icon::new(IconName::Close).xsmall())
                    .tooltip("Close filter builder")
                    .on_click({
                        let view = view.clone();
                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                            view.update(cx, |this, cx| this.attempt_close(window, cx));
                        }
                    }),
            );

        let body = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scrollbar()
            .px(spacing::sm())
            .py(spacing::sm())
            .gap(spacing::sm())
            .child(self.render_root_group(&view, window, cx));

        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::sm())
            .px(spacing::md())
            .py(spacing::sm())
            .border_t_1()
            .border_color(cx.theme().sidebar_border.opacity(0.65))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(
                        Button::new("builder-run")
                            .primary()
                            .compact()
                            .label("Run")
                            .icon(Icon::new(IconName::Search).xsmall())
                            .disabled(!can_run)
                            .on_click({
                                let view = view.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    view.update(cx, |this, cx| this.apply_filter(window, cx));
                                }
                            }),
                    )
                    .child(Button::new("builder-clear").ghost().compact().label("Clear").on_click(
                        {
                            let view = view.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                view.update(cx, |this, cx| this.clear_draft(window, cx));
                            }
                        },
                    ))
                    .child(
                        Button::new("builder-reset")
                            .ghost()
                            .compact()
                            .label("Reset")
                            .disabled(!dirty)
                            .on_click({
                                let view = view.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    view.update(cx, |this, cx| this.reset_to_applied(window, cx));
                                }
                            }),
                    )
                    .child(
                        Button::new("builder-open-json")
                            .ghost()
                            .compact()
                            .icon(Icon::new(IconName::Braces).xsmall())
                            .label("Open JSON")
                            .on_click({
                                let view = view.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    view.update(cx, |this, cx| this.open_json_editor(window, cx));
                                }
                            }),
                    ),
            )
            .child(
                div().flex().items_center().gap(spacing::sm()).child(
                    div()
                        .text_xs()
                        .text_color(
                            validation_error
                                .as_ref()
                                .map(|_| cx.theme().danger)
                                .unwrap_or(cx.theme().muted_foreground),
                        )
                        .child(
                            validation_error.unwrap_or_else(|| "Cmd/Ctrl+Enter to run".to_string()),
                        ),
                ),
            );

        div()
            .id("filter-builder-root")
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .bg(cx.theme().sidebar)
            .border_l_1()
            .border_color(cx.theme().sidebar_border)
            .on_key_down({
                let view = view.clone();
                move |event: &KeyDownEvent, window, cx| match &event.keystroke.key {
                    key if key == "enter" && event.keystroke.modifiers.secondary() => {
                        view.update(cx, |this, cx| this.apply_filter(window, cx));
                    }
                    key if key == "escape" => {
                        view.update(cx, |this, cx| this.attempt_close(window, cx));
                    }
                    _ => {}
                }
            })
            .child(header)
            .child(body)
            .child(footer)
    }
}

impl BulkValueEditor {
    fn parsed_values(&self, cx: &App) -> Vec<String> {
        parse_bulk_values(self.input_state.read(cx).value().as_ref())
    }

    fn invalid_count(&self, cx: &App) -> usize {
        self.parsed_values(cx)
            .iter()
            .filter(|value| !is_valid_value_for_field_type(self.field_type, value))
            .count()
    }
}

impl Render for BulkValueEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let values = self.parsed_values(cx);
        let invalid_count = self.invalid_count(cx);
        let count = values.len();
        let field_type = self.field_type;

        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .p(spacing::md())
            .on_key_down({
                let save_focus = cx.focus_handle();
                move |event: &KeyDownEvent, window, cx| {
                    let key = event.keystroke.key.to_ascii_lowercase();
                    if key == "escape" {
                        cx.stop_propagation();
                        window.close_dialog(cx);
                    } else if key == "enter" && event.keystroke.modifiers.secondary() {
                        cx.stop_propagation();
                        window.focus(&save_focus);
                    }
                }
            })
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(cx.theme().foreground)
                                    .child("Edit values"),
                            )
                            .child(
                                div().text_xs().text_color(cx.theme().muted_foreground).child(
                                    "One value per line. Commas are also accepted on paste.",
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(neutral_chip(list_count_label(count, field_type), cx))
                            .when(invalid_count > 0, |this| {
                                this.child(summary_chip(
                                    format!("{invalid_count} invalid"),
                                    cx.theme().danger,
                                    cx,
                                ))
                            })
                            .when(self.initial_count != count, |this| {
                                this.child(neutral_chip(format!("was {}", self.initial_count), cx))
                            }),
                    ),
            )
            .child(
                Input::new(&self.input_state)
                    .font_family(fonts::mono())
                    .h(px(BULK_EDITOR_HEIGHT))
                    .w_full(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .text_xs()
                            .text_color(if invalid_count > 0 {
                                cx.theme().danger
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(match field_type {
                                FieldType::ObjectId => "Expected 24-character hex ObjectIds",
                                FieldType::DateTime => "Expected ISO timestamps or yyyy-mm-dd",
                                FieldType::Number => "Expected numeric values",
                                _ => "Press Cmd/Ctrl+Enter to save",
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(Button::new("bulk-values-cancel").label("Cancel").on_click(
                                |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    window.close_dialog(cx);
                                },
                            ))
                            .child(
                                Button::new("bulk-values-save")
                                    .primary()
                                    .label("Save")
                                    .disabled(invalid_count > 0)
                                    .on_click({
                                        let input_state = self.input_state.clone();
                                        let on_save = self.on_save.clone();
                                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                            let values = parse_bulk_values(
                                                input_state.read(cx).value().as_ref(),
                                            );
                                            (on_save)(values, window, cx);
                                            window.close_dialog(cx);
                                        }
                                    }),
                            ),
                    ),
            )
    }
}

fn split_committed_tokens(raw: &str, commit_all: bool) -> (Vec<String>, String) {
    let ended_with_delimiter = raw.ends_with(',') || raw.ends_with('\n') || raw.ends_with('\r');
    let parts =
        raw.split([',', '\n', '\r']).map(|part| part.trim().to_string()).collect::<Vec<_>>();

    if parts.is_empty() {
        return (Vec::new(), String::new());
    }

    if commit_all || ended_with_delimiter {
        return (parts.into_iter().filter(|part| !part.is_empty()).collect(), String::new());
    }

    let mut parts = parts;
    let remainder = parts.pop().unwrap_or_default();
    (parts.into_iter().filter(|part| !part.is_empty()).collect(), remainder)
}

fn matching_suggestions(suggestions: &[FieldSuggestion], raw: &str) -> Vec<FieldSuggestion> {
    let query = raw.trim().to_ascii_lowercase();
    if query.is_empty() {
        return suggestions.iter().take(MAX_SUGGESTIONS).cloned().collect();
    }

    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    let mut contains = Vec::new();

    for suggestion in suggestions {
        let path_lower = suggestion.path.to_ascii_lowercase();
        if path_lower == query {
            exact.push(suggestion.clone());
        } else if path_lower.starts_with(&query) {
            prefix.push(suggestion.clone());
        } else if path_lower.contains(&query) {
            contains.push(suggestion.clone());
        }
    }

    exact.extend(prefix);
    exact.extend(contains);
    exact.truncate(MAX_SUGGESTIONS);
    exact
}

fn hint_for_condition(condition: &FilterCondition) -> Option<&'static str> {
    match condition.field_type {
        FieldType::ObjectId => Some("24-character hex string"),
        FieldType::DateTime => Some("ISO timestamp or yyyy-mm-dd"),
        FieldType::Number => Some("Arrow keys step numeric inputs"),
        FieldType::Boolean => Some("Use the toggle"),
        _ => None,
    }
}

fn render_calendar_panel(calendar_state: &Entity<CalendarState>, cx: &App) -> Div {
    div()
        .mt(px(2.0))
        .rounded(borders::radius_sm())
        .border_1()
        .border_color(cx.theme().sidebar_border.opacity(0.7))
        .bg(cx.theme().popover)
        .shadow_md()
        .p(spacing::sm())
        .child(Calendar::new(calendar_state).number_of_months(1))
}

fn parse_bulk_values(raw: &str) -> Vec<String> {
    raw.lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn compact_value_preview(value: &str, field_type: FieldType) -> String {
    let max = match field_type {
        FieldType::ObjectId => 15,
        FieldType::DateTime => 24,
        _ => 22,
    };
    if value.chars().count() <= max {
        return value.to_string();
    }

    let (prefix, suffix) = match field_type {
        FieldType::ObjectId => (8, 6),
        FieldType::DateTime => (12, 8),
        _ => (12, 6),
    };
    let head = value.chars().take(prefix).collect::<String>();
    let tail =
        value.chars().rev().take(suffix).collect::<Vec<_>>().into_iter().rev().collect::<String>();
    format!("{head}…{tail}")
}

fn render_token_chip(
    id: impl Into<ElementId>,
    label: String,
    on_remove: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded(px(999.0))
        .border_1()
        .border_color(cx.theme().primary.opacity(0.25))
        .bg(cx.theme().primary.opacity(0.12))
        .px(spacing::xs())
        .py(px(2.0))
        .child(div().text_xs().text_color(cx.theme().primary).child(label))
        .child(
            Button::new("remove-token")
                .ghost()
                .compact()
                .icon(Icon::new(IconName::Close).xsmall())
                .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    on_remove(window, cx);
                }),
        )
}

fn render_preview_value_chip(label: &'static str, value: String, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded(px(999.0))
        .border_1()
        .border_color(cx.theme().sidebar_border.opacity(0.8))
        .bg(cx.theme().background.opacity(0.55))
        .px(spacing::xs())
        .py(px(2.0))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .text_xs()
                .font_family(fonts::mono())
                .text_color(cx.theme().foreground)
                .child(value),
        )
}

fn summary_chip(label: impl Into<SharedString>, accent: Hsla, _cx: &App) -> Div {
    div()
        .px(spacing::xs())
        .py(px(2.0))
        .rounded(px(999.0))
        .border_1()
        .border_color(accent.opacity(0.28))
        .bg(accent.opacity(0.12))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(accent)
        .child(label.into())
}

fn neutral_chip(label: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .px(spacing::xs())
        .py(px(2.0))
        .rounded(px(999.0))
        .border_1()
        .border_color(cx.theme().sidebar_border.opacity(0.8))
        .bg(cx.theme().secondary.opacity(0.9))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().foreground)
        .child(label.into())
}

fn list_count_label(count: usize, field_type: FieldType) -> String {
    let kind = match field_type {
        FieldType::ObjectId => "ObjectIds",
        FieldType::Number => "numbers",
        FieldType::DateTime => "dates",
        FieldType::Boolean => "booleans",
        _ => "values",
    };
    format!("{count} {kind}")
}

fn toggle_chip(label: &'static str, active: bool, cx: &App) -> Button {
    let mut button =
        Button::new(SharedString::from(format!("toggle-chip-{label}"))).compact().label(label);
    if active {
        button = button.active_style(cx.theme().primary.opacity(0.25));
    } else {
        button = button.ghost();
    }
    button
}

fn field_type_color(field_type: FieldType, cx: &App) -> Hsla {
    match field_type {
        FieldType::String => cx.theme().green,
        FieldType::Number => cx.theme().yellow,
        FieldType::Boolean => cx.theme().blue,
        FieldType::ObjectId => cx.theme().cyan,
        FieldType::DateTime => cx.theme().magenta,
        FieldType::Array => cx.theme().red,
        FieldType::Document => cx.theme().muted_foreground,
        FieldType::Null => cx.theme().muted_foreground,
        FieldType::Unknown => cx.theme().muted_foreground,
    }
}

fn type_badge(field_type: FieldType, cx: &App) -> impl IntoElement {
    let label = match field_type {
        FieldType::String => "String",
        FieldType::Number => "Number",
        FieldType::Boolean => "Bool",
        FieldType::ObjectId => "ObjectId",
        FieldType::DateTime => "Date",
        FieldType::Array => "Array",
        FieldType::Document => "Doc",
        FieldType::Null => "Null",
        FieldType::Unknown => "Unknown",
    };
    let color = field_type_color(field_type, cx);

    div()
        .px(px(6.0))
        .py(px(2.0))
        .rounded(px(999.0))
        .bg(color.opacity(0.12))
        .border_1()
        .border_color(color.opacity(0.22))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(label)
}

fn field_type_label(field_type: FieldType) -> &'static str {
    match field_type {
        FieldType::String => "String field",
        FieldType::Number => "Number field",
        FieldType::Boolean => "Boolean field",
        FieldType::ObjectId => "ObjectId field",
        FieldType::DateTime => "Date/time field",
        FieldType::Array => "Array field",
        FieldType::Document => "Document field",
        FieldType::Null => "Null field",
        FieldType::Unknown => "Unknown type",
    }
}

fn type_badge_compact(
    id: impl Into<ElementId>,
    field_type: FieldType,
    tooltip: SharedString,
    cx: &App,
) -> impl IntoElement {
    let label = match field_type {
        FieldType::String => "Str",
        FieldType::Number => "Num",
        FieldType::Boolean => "Bool",
        FieldType::ObjectId => "OId",
        FieldType::DateTime => "Date",
        FieldType::Array => "Arr",
        FieldType::Document => "Doc",
        FieldType::Null => "Null",
        FieldType::Unknown => "?",
    };
    let color = field_type_color(field_type, cx);

    div()
        .id(id.into())
        .px(px(4.0))
        .py(px(1.0))
        .rounded(px(999.0))
        .bg(color.opacity(0.12))
        .border_1()
        .border_color(color.opacity(0.22))
        .font_family(fonts::mono())
        .text_color(color)
        .line_height(rems(1.0))
        .child(
            div()
                .text_size(crate::theme::typography::text_2xs())
                .font_weight(FontWeight::SEMIBOLD)
                .child(label),
        )
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
}

fn dropdown_variant(cx: &mut App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .color(cx.theme().secondary)
        .foreground(cx.theme().foreground)
        .border(cx.theme().sidebar_border)
        .hover(cx.theme().secondary_hover)
        .active(cx.theme().secondary_hover)
        .shadow(false)
}

fn styled_dropdown_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &mut App,
) -> MenuButton {
    MenuButton::new(id)
        .compact()
        .label(label)
        .dropdown_caret(true)
        .custom(dropdown_variant(cx))
        .rounded(borders::radius_sm())
        .with_size(Size::XSmall)
        .refine_style(
            &StyleRefinement::default()
                .font_family(fonts::ui())
                .font_weight(FontWeight::NORMAL)
                .text_size(crate::theme::typography::text_xs())
                .h(px(22.0))
                .px(spacing::sm())
                .py(px(2.0)),
        )
}

fn build_typed_suggestions(documents: &[Document]) -> Vec<FieldSuggestion> {
    let mut seen: HashMap<String, (FieldType, Option<String>)> = HashMap::new();

    for document in documents {
        collect_typed_paths(document, "", &mut seen, 0);
    }

    let mut suggestions = seen
        .into_iter()
        .map(|(path, (field_type, sample_value))| FieldSuggestion {
            path,
            field_type,
            sample_value,
        })
        .collect::<Vec<_>>();
    suggestions.sort_by(|a, b| a.path.cmp(&b.path));
    suggestions.truncate(200);
    suggestions
}

fn collect_typed_paths(
    document: &Document,
    prefix: &str,
    seen: &mut HashMap<String, (FieldType, Option<String>)>,
    depth: usize,
) {
    if depth > 6 {
        return;
    }

    for (key, value) in document {
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        seen.entry(path.clone()).or_insert_with(|| {
            (
                FieldType::from_bson(value),
                Some(crate::bson::format_relaxed_json_value(&value.clone().into_relaxed_extjson())),
            )
        });

        if let Bson::Document(inner) = value {
            collect_typed_paths(inner, &path, seen, depth + 1);
        }
        if let Bson::Array(items) = value {
            for item in items.iter().take(5) {
                if let Bson::Document(inner) = item {
                    collect_typed_paths(inner, &path, seen, depth + 1);
                }
            }
        }
    }
}
