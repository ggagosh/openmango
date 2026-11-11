//! View-model for document tree rendering and editing behavior.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use chrono::NaiveDate;
use gpui_kit::component::calendar::{CalendarEvent, CalendarState, Date};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::table::TableState;
use gpui_kit::component::tree::{TreeItem, TreeState};
use gpui_kit::*;
use mongodb::bson::{Bson, Document};

use crate::bson::{DocumentKey, PathSegment, bson_value_for_edit, parse_edited_value};
use crate::perf::log_tabs_duration;
use crate::state::{AppState, SessionKey, TabKey};
use crate::views::documents::dialogs::property_dialog::PropertyActionDialog;
use crate::views::documents::node_meta::NodeMeta;
use crate::views::documents::table::aggregation_table_delegate::AggregationTableDelegate;
use crate::views::documents::table::document_table_delegate::DocumentTableDelegate;
use crate::views::documents::table::table_columns::TableColumns;
use crate::views::documents::tree::document_tree::build_documents_tree;

use super::CollectionView;

const MAX_CACHED_TREE_SESSIONS: usize = 12;

struct CachedTreeState {
    generation: u64,
    items: Arc<[TreeItem]>,
    meta: Arc<HashMap<String, NodeMeta>>,
    order: Arc<[String]>,
    selected_index: Option<usize>,
    cached_at: u64,
}

pub struct DocumentViewModel {
    tree_state: Entity<TreeState>,
    current_session: Option<SessionKey>,
    node_meta: Arc<HashMap<String, NodeMeta>>,
    tree_items: Arc<[TreeItem]>,
    tree_order: Arc<[String]>,
    tree_cache: HashMap<SessionKey, CachedTreeState>,
    cache_epoch: u64,
    inline_editor_state: Option<Entity<InputState>>,
    inline_value_subscription: Option<Subscription>,
    editing_node_id: Option<String>,
    editing_doc_key: Option<DocumentKey>,
    editing_path: Vec<PathSegment>,
    editing_original: Option<Bson>,
    editing_draft_before: Option<Document>,
    table_state: Option<Entity<TableState<DocumentTableDelegate>>>,
    table_generation: Option<u64>,
    /// Order-independent signature of the selection last pushed to the table,
    /// so we can skip re-pushing + re-notifying when nothing changed.
    table_selected_sig: Option<u64>,
    col_visibility_search: Option<Entity<InputState>>,
    agg_table_state: Option<Entity<TableState<AggregationTableDelegate>>>,
    agg_table_generation: Option<u64>,
    /// The calendar under a date cell while it is edited in the table.
    cell_calendar: Option<CellCalendar>,
}

/// The picker under a date cell's editor: a calendar for the day and boxes for the time. Both
/// rewrite their part of the cell's text, which stays the one value being edited.
pub struct CellCalendar {
    pub calendar: Entity<CalendarState>,
    /// Hours, minutes, seconds.
    pub time: [Entity<InputState>; 3],
    _subscriptions: Vec<Subscription>,
}

/// The `YYYY-MM-DD` a date's edit text starts with, as the date editor writes it.
fn leading_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.get(..10)?, "%Y-%m-%d").ok()
}

/// `text` with its leading day replaced by `day`, keeping the time and offset after it. Text
/// that does not start with a day becomes just the day, which the date parser also reads.
fn with_date(text: &str, day: NaiveDate) -> String {
    let day = day.format("%Y-%m-%d").to_string();
    match leading_date(text) {
        Some(_) => format!("{day}{}", &text[10..]),
        None => day,
    }
}

/// The hours, minutes and seconds of a date's edit text (`YYYY-MM-DDTHH:MM:SS…`).
fn time_parts(text: &str) -> Option<[&str; 3]> {
    leading_date(text)?;
    if !matches!(text.as_bytes().get(10), Some(b'T' | b' ')) {
        return None;
    }
    let parts = [text.get(11..13)?, text.get(14..16)?, text.get(17..19)?];
    let colons = text.get(13..14) == Some(":") && text.get(16..17) == Some(":");
    (colons && parts.iter().all(|part| part.bytes().all(|b| b.is_ascii_digit()))).then_some(parts)
}

/// `text` with its time of day replaced by `time` (`HH:MM:SS`), keeping the day and the offset.
/// Fractions of a second go, since the boxes set whole seconds. Text that does not start with
/// a day stays as it is.
fn with_time(text: &str, time: &str) -> String {
    match utc_offset(text) {
        Some(offset) => format!("{}T{time}{offset}", &text[..10]),
        None => text.to_string(),
    }
}

/// What follows the time in a date's edit text: `Z`, `+04:00`, or nothing.
fn utc_offset(text: &str) -> Option<&str> {
    leading_date(text)?;
    let rest = text[10..].strip_prefix(['T', ' ']).unwrap_or(&text[10..]);
    let offset = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == ':');
    Some(
        offset
            .strip_prefix('.')
            .map_or(offset, |fraction| fraction.trim_start_matches(|c: char| c.is_ascii_digit())),
    )
}

/// The zone the time boxes are in, beside them: the text's offset, or UTC, which is how the
/// date parser reads a time without one.
pub fn time_zone_label(text: &str) -> String {
    match utc_offset(text) {
        Some("" | "Z") | None => "UTC".into(),
        Some(offset) => offset.into(),
    }
}

/// The time the three boxes spell, once each holds a number.
fn typed_time(boxes: &[Entity<InputState>; 3], cx: &App) -> Option<String> {
    let mut parts = [0; 3];
    for ((part, input), max) in parts.iter_mut().zip(boxes).zip([23, 59, 59]) {
        let value = input.read(cx).value();
        if value.is_empty() {
            return None;
        }
        *part = super::view::parse_time_part(&value, max);
    }
    Some(format!("{:02}:{:02}:{:02}", parts[0], parts[1], parts[2]))
}

/// Order-independent signature of a selection set: two independent accumulators
/// (XOR + sum) over per-key hashes plus the count, so a changed selection is
/// detected cheaply each frame without cloning the set.
fn selection_signature(set: &HashSet<DocumentKey>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut xor: u64 = 0;
    let mut sum: u64 = 0;
    for key in set {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        let h = hasher.finish();
        xor ^= h;
        sum = sum.wrapping_add(h);
    }
    xor ^ sum.rotate_left(32) ^ (set.len() as u64)
}

impl DocumentViewModel {
    pub fn new(cx: &mut Context<CollectionView>) -> Self {
        Self {
            tree_state: cx.new(|cx| TreeState::new(cx)),
            current_session: None,
            node_meta: Arc::new(HashMap::new()),
            tree_items: Arc::from(Vec::<TreeItem>::new()),
            tree_order: Arc::from(Vec::<String>::new()),
            tree_cache: HashMap::new(),
            cache_epoch: 0,
            inline_editor_state: None,
            inline_value_subscription: None,
            editing_node_id: None,
            editing_doc_key: None,
            editing_path: Vec::new(),
            editing_original: None,
            editing_draft_before: None,
            table_state: None,
            table_generation: None,
            table_selected_sig: None,
            col_visibility_search: None,
            agg_table_state: None,
            agg_table_generation: None,
            cell_calendar: None,
        }
    }

    pub fn tree_state(&self) -> Entity<TreeState> {
        self.tree_state.clone()
    }

    pub fn table_state(&self) -> Option<&Entity<TableState<DocumentTableDelegate>>> {
        self.table_state.as_ref()
    }

    pub fn node_meta(&self) -> Arc<HashMap<String, NodeMeta>> {
        self.node_meta.clone()
    }

    pub fn tree_order(&self) -> &[String] {
        &self.tree_order
    }

    pub fn tree_order_snapshot(&self) -> Arc<[String]> {
        self.tree_order.clone()
    }

    pub fn editing_node_id(&self) -> Option<String> {
        self.editing_node_id.clone()
    }

    pub fn inline_state(&self) -> Option<Entity<InputState>> {
        self.inline_editor_state.clone()
    }

    pub fn current_session(&self) -> Option<SessionKey> {
        self.current_session.clone()
    }

    pub fn is_current_session(&self, session: &SessionKey) -> bool {
        self.current_session.as_ref() == Some(session)
    }

    pub fn is_editing_doc(&self, doc_key: &DocumentKey) -> bool {
        self.editing_doc_key.as_ref() == Some(doc_key)
    }

    pub fn set_current_session(
        &mut self,
        next: Option<SessionKey>,
        state: &Entity<AppState>,
        cx: &mut Context<CollectionView>,
    ) -> bool {
        let start = Instant::now();
        let prev = self.current_session.clone();
        if self.current_session == next {
            log_tabs_duration("documents.set_current_session.noop", start, || {
                "unchanged=true".to_string()
            });
            return false;
        }

        let next_loaded = next
            .as_ref()
            .and_then(|key| state.read(cx).session_data(key).map(|data| data.loaded))
            .unwrap_or(false);

        // Save current tree state to cache before switching away.
        if let Some(prev_key) = self.current_session.clone() {
            let generation = state.read(cx).session(&prev_key).map(|s| s.generation).unwrap_or(0);
            let selected_index = self.tree_state.read(cx).selected_index();
            self.cache_epoch = self.cache_epoch.saturating_add(1);
            self.tree_cache.insert(
                prev_key,
                CachedTreeState {
                    generation,
                    items: self.tree_items.clone(),
                    meta: self.node_meta.clone(),
                    order: std::mem::replace(&mut self.tree_order, Arc::from(Vec::<String>::new())),
                    selected_index,
                    cached_at: self.cache_epoch,
                },
            );
            self.enforce_tree_cache_limit();
        }
        self.current_session = next;

        // For loaded sessions, avoid rendering an intermediate empty tree during tab switches.
        if next_loaded {
            self.node_meta = Arc::new(HashMap::new());
            self.tree_items = Arc::from(Vec::<TreeItem>::new());
            self.tree_order = Arc::from(Vec::<String>::new());
            self.inline_editor_state = None;
            self.clear_inline_edit();
        } else {
            self.reset_view_state(cx);
        }
        self.sync_dirty_state(state, cx);
        log_tabs_duration("documents.set_current_session", start, || {
            let from = prev
                .as_ref()
                .map(|s| format!("{}/{}", s.database, s.collection))
                .unwrap_or_else(|| "-".to_string());
            let to = self
                .current_session
                .as_ref()
                .map(|s| format!("{}/{}", s.database, s.collection))
                .unwrap_or_else(|| "-".to_string());
            format!("from={from} to={to} next_loaded={next_loaded}")
        });
        true
    }

    pub fn prune_tree_cache(&mut self, state: &AppState) {
        let mut live_sessions = HashSet::new();

        for tab in state.open_tabs() {
            if let TabKey::Collection(session_key) = tab {
                live_sessions.insert(session_key.clone());
            }
        }

        if let Some(preview) = state.preview_tab() {
            live_sessions.insert(preview.clone());
        }

        if let Some(current) = &self.current_session {
            live_sessions.insert(current.clone());
        }

        self.tree_cache.retain(|session_key, _| live_sessions.contains(session_key));
        self.enforce_tree_cache_limit();
    }

    fn enforce_tree_cache_limit(&mut self) {
        if self.tree_cache.len() <= MAX_CACHED_TREE_SESSIONS {
            return;
        }

        let mut cached_entries: Vec<(SessionKey, u64)> = self
            .tree_cache
            .iter()
            .map(|(session_key, cached)| (session_key.clone(), cached.cached_at))
            .collect();
        cached_entries.sort_by_key(|(_, cached_at)| *cached_at);

        let remove_count = self.tree_cache.len().saturating_sub(MAX_CACHED_TREE_SESSIONS);
        for (session_key, _) in cached_entries.into_iter().take(remove_count) {
            self.tree_cache.remove(&session_key);
        }
    }

    pub fn reset_view_state(&mut self, cx: &mut Context<CollectionView>) {
        self.node_meta = Arc::new(HashMap::new());
        self.tree_items = Arc::from(Vec::<TreeItem>::new());
        self.tree_order = Arc::from(Vec::<String>::new());
        self.inline_editor_state = None;
        self.clear_inline_edit();

        self.tree_state.update(cx, |tree, cx| {
            tree.set_items(Vec::<TreeItem>::new(), cx);
            tree.set_selected_index(None, cx);
        });
    }

    pub fn rebuild_tree(&mut self, state: &Entity<AppState>, cx: &mut Context<CollectionView>) {
        let start = Instant::now();
        let Some(session_key) = self.current_session.clone() else {
            return;
        };
        let state_ref = state.read(cx);
        let Some(session) = state_ref.session(&session_key) else {
            return;
        };

        // Try to restore from cache if the underlying data hasn't changed.
        let generation = session.generation;
        if let Some(cached) = self.tree_cache.remove(&session_key)
            && cached.generation == generation
        {
            self.node_meta = cached.meta;
            self.tree_items = cached.items.clone();
            self.tree_order = cached.order;

            self.tree_state.update(cx, |tree, cx| {
                tree.set_items(cached.items.to_vec(), cx);
                tree.set_selected_index(cached.selected_index, cx);
            });
            let items = self.tree_order.len();
            log_tabs_duration("documents.rebuild_tree", start, || {
                format!(
                    "cache=hit generation={generation} items={items} session={}/{}",
                    session_key.database, session_key.collection
                )
            });
            return;
        }

        let data = &session.data;
        let view = &session.view;

        let (items, meta, order) =
            build_documents_tree(&data.items, &view.drafts, &view.expanded_nodes, cx);
        let selected_index = view
            .selected_node_id
            .as_ref()
            .and_then(|id| order.iter().position(|entry| entry == id));

        // Cache only the materialized nodes; descendants are built when expanded.
        let items: Arc<[TreeItem]> = Arc::from(items);
        self.node_meta = Arc::new(meta);
        self.tree_items = items.clone();
        self.tree_order = Arc::from(order);

        self.tree_state.update(cx, |tree, cx| {
            tree.set_items(items.to_vec(), cx);
            tree.set_selected_index(selected_index, cx);
        });
        let items = self.tree_order.len();
        log_tabs_duration("documents.rebuild_tree", start, || {
            format!(
                "cache=miss generation={generation} items={items} session={}/{}",
                session_key.database, session_key.collection
            )
        });
    }

    pub fn sync_dirty_state(&self, state: &Entity<AppState>, cx: &mut Context<CollectionView>) {
        let Some(session) = self.current_session.clone() else {
            return;
        };
        let dirty =
            state.read(cx).session_view(&session).is_some_and(|view| !view.dirty.is_empty());
        state.update(cx, |state, cx| {
            state.set_collection_dirty(session.clone(), dirty, cx);
        });
    }

    pub fn clear_inline_edit(&mut self) {
        self.editing_node_id = None;
        self.editing_doc_key = None;
        self.editing_path.clear();
        self.editing_original = None;
        self.editing_draft_before = None;
        self.inline_editor_state = None;
        self.inline_value_subscription = None;
        self.cell_calendar = None;
        // The table draws drafts from its last rebuild; an edit that ends may have changed one.
        self.invalidate_table();
    }

    /// The document and top-level field being edited: a table cell, or a top-level tree value.
    pub fn editing_field(&self) -> Option<(DocumentKey, String)> {
        let [PathSegment::Key(field)] = self.editing_path.as_slice() else { return None };
        Some((self.editing_doc_key.clone()?, field.clone()))
    }

    /// The inline editor, when it is editing this top-level field of this document: a table cell.
    pub fn inline_editor_for(
        &self,
        doc_key: &DocumentKey,
        field: &str,
    ) -> Option<&Entity<InputState>> {
        let editing = self.editing_field().is_some_and(|(editing_doc, editing_field)| {
            &editing_doc == doc_key && editing_field == field
        });
        self.inline_editor_state.as_ref().filter(|_| editing)
    }

    pub fn cell_calendar(&self) -> Option<&CellCalendar> {
        self.cell_calendar.as_ref()
    }

    /// Whether focus is anywhere in the cell editor: its text, or a time box under it.
    pub fn inline_editor_focused(&self, window: &Window, cx: &App) -> bool {
        self.inline_input_focused(window, cx)
            || self.cell_calendar.as_ref().is_some_and(|picker| {
                picker.time.iter().any(|input| input.read(cx).focus_handle(cx).is_focused(window))
            })
    }

    /// Tab and Shift+Tab in the time boxes: hours, minutes, seconds, and Shift+Tab from the
    /// hours back to the text. False when Tab should leave the cell instead.
    pub fn tab_time_box(&self, backwards: bool, window: &mut Window, cx: &mut App) -> bool {
        let (Some(text), Some(picker)) = (&self.inline_editor_state, &self.cell_calendar) else {
            return false;
        };
        let Some(at) =
            picker.time.iter().position(|input| input.read(cx).focus_handle(cx).is_focused(window))
        else {
            return false;
        };
        let target = match (backwards, at) {
            (true, 0) => text,
            (true, at) => &picker.time[at - 1],
            (false, 2) => return false,
            (false, at) => &picker.time[at + 1],
        };
        let focus = target.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        true
    }

    /// Opens the day and time picker under the date being edited. Picking a day rewrites the
    /// date part of the text and the time boxes rewrite its time; typing in the text moves the
    /// picker along with it.
    pub fn open_cell_calendar(&mut self, window: &mut Window, cx: &mut Context<CollectionView>) {
        let Some(input) = self.inline_editor_state.clone() else { return };
        let text = input.read(cx).value().to_string();
        let calendar = cx.new(|cx| CalendarState::new(window, cx));
        if let Some(day) = leading_date(&text) {
            calendar.update(cx, |calendar, cx| calendar.set_date(day, window, cx));
        }
        let time = ["HH", "MM", "SS"]
            .map(|placeholder| cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)));
        if let Some(parts) = time_parts(&text) {
            for (time_input, part) in time.iter().zip(parts) {
                time_input.update(cx, |time_input, cx| time_input.set_value(part, window, cx));
            }
        }

        // `set_value` emits no change, so none of these rewrites sets off another.
        let mut subscriptions = vec![cx.subscribe_in(&calendar, window, {
            let input = input.clone();
            move |view, _, event: &CalendarEvent, window, cx| {
                let CalendarEvent::Selected(Date::Single(Some(day))) = event else { return };
                let text = with_date(&input.read(cx).value(), *day);
                let state = view.state.clone();
                view.view_model.set_cell_text(&state, &input, text, window, cx);
                // Back to the text, so Enter keeps the date.
                let focus = input.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
        })];
        for (time_input, max) in time.iter().zip([23, 59, 59]) {
            // Digits only, padded and clamped on blur: the filter bar's time boxes.
            subscriptions.push(super::view::subscribe_time_input(cx, window, time_input, max));
            subscriptions.push(cx.subscribe_in(time_input, window, {
                let input = input.clone();
                let time = time.clone();
                move |view, _, event: &InputEvent, window, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let Some(time_of_day) = typed_time(&time, cx) else { return };
                    let text = with_time(&input.read(cx).value(), &time_of_day);
                    let state = view.state.clone();
                    view.view_model.set_cell_text(&state, &input, text, window, cx);
                }
            }));
        }
        subscriptions.push(cx.subscribe_in(&input, window, {
            let time = time.clone();
            move |_, input, event: &InputEvent, window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let Some(parts) = time_parts(&input.read(cx).value()).map(|p| p.map(String::from))
                else {
                    return;
                };
                for (time_input, part) in time.iter().zip(parts) {
                    if time_input.read(cx).value() != part {
                        time_input
                            .update(cx, |time_input, cx| time_input.set_value(part, window, cx));
                    }
                }
            }
        }));
        self.cell_calendar = Some(CellCalendar { calendar, time, _subscriptions: subscriptions });
    }

    /// Puts the picker's result in the cell's text and stages it, as typing there would.
    fn set_cell_text(
        &mut self,
        state: &Entity<AppState>,
        input: &Entity<InputState>,
        text: String,
        window: &mut Window,
        cx: &mut Context<CollectionView>,
    ) {
        if input.read(cx).value() == text {
            return;
        }
        input.update(cx, |input, cx| input.set_value(text, window, cx));
        self.sync_inline_edit_draft(state, cx);
        cx.notify();
    }

    pub fn begin_inline_edit(
        &mut self,
        node_id: String,
        meta: &NodeMeta,
        window: &mut Window,
        state: &Entity<AppState>,
        cx: &mut Context<CollectionView>,
    ) {
        if !meta.is_editable {
            return;
        }
        if self.editing_node_id.is_some() {
            self.commit_inline_edit(state, cx);
            if self.editing_node_id.is_some() {
                return;
            }
        }
        if let Some(reason) = self
            .current_session
            .as_ref()
            .and_then(|key| state.read(cx).document_field_edit_restriction(key))
        {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(crate::state::StatusMessage::error(reason)));
                cx.notify();
            });
            return;
        }
        let value = meta.value.as_ref();
        if let Some(Bson::String(text)) = value
            && (text.contains('\n') || text.contains('\r'))
        {
            let Some(session_key) = self.current_session.clone() else {
                return;
            };
            let allow_bulk =
                !meta.path.iter().any(|segment| matches!(segment, PathSegment::Index(_)));
            PropertyActionDialog::open_edit_value(
                state.clone(),
                session_key,
                meta.clone(),
                allow_bulk,
                window,
                cx,
            );
            return;
        }
        // Every editable type uses the same text input and the same parsing contract:
        // see `parse_edited_value`.
        let placeholder = value.map(crate::bson::value_input_placeholder).unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        if let Some(text) = value.map(bson_value_for_edit) {
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }

        self.editing_node_id = Some(node_id);
        self.editing_doc_key = Some(meta.doc_key.clone());
        self.editing_path = meta.path.clone();
        self.editing_original = meta.value.clone();
        self.editing_draft_before = self
            .current_session
            .as_ref()
            .and_then(|session_key| state.read(cx).session_draft(session_key, &meta.doc_key));
        if let Some(session_key) = self.current_session.clone() {
            state.update(cx, |state, cx| {
                state.expand_path(&session_key, &meta.doc_key, &meta.path);
                cx.notify();
            });
        }

        let app_state = state.clone();
        self.inline_value_subscription =
            Some(cx.subscribe_in(&input, window, move |view, _state, event, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.view_model.sync_inline_edit_draft(&app_state, cx);
                }
            }));
        let focus = input.read(cx).focus_handle(cx);
        self.inline_editor_state = Some(input);
        window.defer(cx, move |window, cx| {
            window.focus(&focus, cx);
        });
    }

    fn inline_edited_value(&self, cx: &App) -> Result<Bson, String> {
        let original = self
            .editing_original
            .as_ref()
            .ok_or_else(|| "Inline editor has no original value".to_string())?;
        let editor = self
            .inline_editor_state
            .as_ref()
            .ok_or_else(|| "Inline editor is unavailable".to_string())?;
        parse_edited_value(original, editor.read(cx).value().as_ref())
    }

    pub fn inline_edit_error(&self, cx: &App) -> Option<String> {
        self.inline_editor_state.as_ref()?;
        self.inline_edited_value(cx).err()
    }

    pub fn inline_input_focused(&self, window: &Window, cx: &App) -> bool {
        self.inline_editor_state
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window))
    }

    pub(crate) fn sync_inline_edit_draft(
        &mut self,
        state: &Entity<AppState>,
        cx: &mut Context<CollectionView>,
    ) {
        let Some(session_key) = self.current_session.clone() else {
            return;
        };
        let Some(doc_key) = self.editing_doc_key.clone() else {
            return;
        };
        match self.inline_edited_value(cx) {
            Ok(value) => {
                state.update(cx, |state, _| {
                    state.set_invalid_inline_edit(session_key, false);
                });
                let path = self.editing_path.clone();
                self.update_draft_value(state, &doc_key, &path, value, cx);
            }
            Err(_) => {
                state.update(cx, |state, cx| {
                    state.set_invalid_inline_edit(session_key, true);
                    cx.notify();
                });
            }
        }
    }

    pub fn cancel_inline_edit(
        &mut self,
        state: &Entity<AppState>,
        cx: &mut Context<CollectionView>,
    ) {
        if let (Some(session_key), Some(doc_key)) =
            (self.current_session.clone(), self.editing_doc_key.clone())
        {
            let previous = self.editing_draft_before.clone();
            state.update(cx, |state, cx| {
                state.set_invalid_inline_edit(session_key.clone(), false);
                if let Some(previous) = previous {
                    state.set_draft(&session_key, doc_key.clone(), previous);
                } else {
                    state.clear_draft(&session_key, &doc_key);
                }
                cx.notify();
            });
            self.sync_dirty_state(state, cx);
        }
        self.clear_inline_edit();
        self.rebuild_tree(state, cx);
    }

    pub fn commit_inline_edit(
        &mut self,
        state: &Entity<AppState>,
        cx: &mut Context<CollectionView>,
    ) {
        let Some(doc_key) = self.editing_doc_key.clone() else {
            return;
        };
        let Some(original) = self.editing_original.clone() else {
            return;
        };
        let path = self.editing_path.clone();
        let result = self.inline_edited_value(cx);

        match result {
            Ok(new_value) => {
                if let Some(session_key) = self.current_session.clone() {
                    state.update(cx, |state, _| {
                        state.set_invalid_inline_edit(session_key, false);
                    });
                }
                if new_value == original {
                    self.clear_inline_edit();
                    return;
                }

                if self.update_draft_value(state, &doc_key, &path, new_value, cx) {
                    self.clear_inline_edit();
                    self.rebuild_tree(state, cx);
                }
            }
            Err(err) => {
                if let Some(session_key) = self.current_session.clone() {
                    state.update(cx, |state, cx| {
                        state.set_invalid_inline_edit(session_key, true);
                        // The row shows this under the field being edited.
                        state.record_error(
                            crate::error::ErrorReport::new(
                                "Invalid field value",
                                crate::error::sentence(&err.to_string()),
                            )
                            .kind(crate::error::ErrorKind::Validation),
                        );
                        cx.notify();
                    });
                }
                log::warn!("Inline edit failed: {err}");
            }
        }
    }

    pub fn update_draft_value(
        &mut self,
        state: &Entity<AppState>,
        doc_key: &DocumentKey,
        path: &[PathSegment],
        new_value: Bson,
        cx: &mut Context<CollectionView>,
    ) -> bool {
        let Some(session_key) = self.current_session.clone() else {
            return false;
        };
        let original = {
            let state_ref = state.read(cx);
            state_ref.document_for_key(&session_key, doc_key)
        };
        let Some(original) = original else {
            return false;
        };

        let mut updated = false;
        state.update(cx, |state, cx| {
            updated = state.update_draft_value(&session_key, doc_key, &original, path, new_value);
            cx.notify();
        });

        if updated {
            self.sync_dirty_state(state, cx);
        }
        updated
    }

    pub fn ensure_table_state(
        &mut self,
        state: &Entity<AppState>,
        view: &Entity<CollectionView>,
        window: &mut Window,
        cx: &mut Context<CollectionView>,
    ) -> Entity<TableState<DocumentTableDelegate>> {
        if let Some(table_state) = &self.table_state {
            return table_state.clone();
        }
        let delegate =
            DocumentTableDelegate::new(state.clone(), view.clone(), self.current_session.clone());
        let table_state = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .col_selectable(false)
                .col_movable(true)
                .row_selectable(false)
        });

        // Subscribe to table events for row selection and double-click.
        cx.subscribe_in(&table_state, window, move |cv, ts, event, window, cx| {
            use gpui_kit::component::table::TableEvent;
            match event {
                TableEvent::SelectRow(row_ix) => {
                    let row_ix = *row_ix;
                    let session_key = cv.view_model.current_session();
                    let doc_key = ts.read(cx).delegate().document_key(row_ix);
                    if let (Some(sk), Some(dk)) = (session_key, doc_key) {
                        cv.state.update(cx, |s, cx| {
                            s.select_single_doc(&sk, dk, String::new());
                            cx.notify();
                        });
                        ts.update(cx, |ts, _cx| {
                            let mut keys = std::collections::HashSet::new();
                            if let Some(dk) = ts.delegate().document_key(row_ix) {
                                keys.insert(dk);
                            }
                            ts.delegate_mut().set_selected_doc_keys(keys);
                        });
                        cx.notify();
                    }
                }
                TableEvent::DoubleClickedRow(row_ix) => {
                    let row_ix = *row_ix;
                    let session_key = cv.view_model.current_session();
                    let doc_key = ts.read(cx).delegate().document_key(row_ix);
                    if let (Some(sk), Some(dk)) = (session_key, doc_key) {
                        cv.open_document_json(sk, dk, window, cx);
                    }
                }
                TableEvent::ColumnWidthsChanged(widths) => {
                    let col_widths: HashMap<String, f32> = {
                        let delegate = ts.read(cx).delegate();
                        widths
                            .iter()
                            .enumerate()
                            .filter_map(|(i, w)| delegate.column_key(i).map(|k| (k, f32::from(*w))))
                            .collect()
                    };
                    ts.update(cx, |ts, _cx| {
                        ts.delegate_mut().update_saved_widths(col_widths.clone());
                    });
                    if let Some(sk) = cv.view_model.current_session() {
                        cv.state.update(cx, |state, cx| {
                            state.set_table_column_widths(&sk, col_widths);
                            cx.notify();
                        });
                    }
                }
                TableEvent::MoveColumn(from_ix, to_ix) => {
                    let from_ix = *from_ix;
                    let to_ix = *to_ix;
                    let order = {
                        ts.update(cx, |ts, _cx| {
                            ts.delegate_mut().apply_column_move(from_ix, to_ix);
                            ts.delegate_mut().column_order()
                        })
                    };
                    if let Some(sk) = cv.view_model.current_session() {
                        cv.state.update(cx, |state, cx| {
                            state.set_table_column_order(&sk, order);
                            cx.notify();
                        });
                    }
                    cv.view_model.invalidate_table();
                    cx.notify();
                }
                _ => {}
            }
        })
        .detach();

        self.table_state = Some(table_state.clone());
        table_state
    }

    pub fn rebuild_table(
        &mut self,
        state: &Entity<AppState>,
        view: &Entity<CollectionView>,
        window: &mut Window,
        cx: &mut Context<CollectionView>,
    ) {
        let Some(session_key) = self.current_session.clone() else {
            return;
        };
        let state_ref = state.read(cx);
        let Some(session) = state_ref.session(&session_key) else {
            return;
        };
        let generation = session.generation;
        let generation_changed =
            self.table_generation != Some(generation) || self.table_state.is_none();
        let selected_sig = selection_signature(&session.view.selected_docs);
        let selection_changed = self.table_selected_sig != Some(selected_sig);

        // Nothing the table depends on changed → leave it untouched. Avoids a
        // per-frame HashSet clone and an unconditional `cx.notify()` that would
        // otherwise re-render every visible cell on every frame.
        if !generation_changed && !selection_changed {
            return;
        }

        let selected_docs = session.view.selected_docs.clone();

        if generation_changed {
            let documents = session.data.items.clone();
            let drafts = session.view.drafts.clone();
            let is_loading = session.data.is_loading;
            let saved_widths = session.view.table_column_widths.clone();
            let saved_order = session.view.table_column_order.clone();
            let pinned = session.view.table_pinned_columns.clone();
            let hidden = session.view.table_hidden_columns.clone();

            let table_state = self.ensure_table_state(state, view, window, cx);
            table_state.update(cx, |ts, cx| {
                // One table serves every collection tab, and its columns accumulate as documents
                // are seen. Start over when the collection changes, or the last one's columns stay.
                if ts.delegate().session_key.as_ref() != Some(&session_key) {
                    ts.delegate_mut().table_cols = TableColumns::new();
                }
                ts.delegate_mut().set_saved_widths(saved_widths);
                ts.delegate_mut().set_column_order(saved_order);
                ts.delegate_mut().set_pinned_columns(pinned);
                ts.delegate_mut().set_hidden_columns(hidden);
                ts.delegate_mut().set_selected_doc_keys(selected_docs);
                ts.delegate_mut().refresh_data(documents, drafts, Some(session_key), is_loading);
                ts.refresh(cx);
            });
            self.table_generation = Some(generation);
        } else {
            let table_state = self.ensure_table_state(state, view, window, cx);
            table_state.update(cx, |ts, cx| {
                ts.delegate_mut().set_selected_doc_keys(selected_docs);
                cx.notify();
            });
        }
        self.table_selected_sig = Some(selected_sig);
    }

    pub fn invalidate_table(&mut self) {
        self.table_generation = None;
    }

    /// Drop every cached tree. Their rows hold formatted text, which a display setting such as
    /// the date zone makes stale without the session's data changing.
    pub fn clear_tree_cache(&mut self) {
        self.tree_cache.clear();
    }

    // ── Aggregation table ────────────────────────────────────────────

    pub fn agg_table_state(&self) -> Option<&Entity<TableState<AggregationTableDelegate>>> {
        self.agg_table_state.as_ref()
    }

    pub fn ensure_agg_table_state(
        &mut self,
        state: &Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<CollectionView>,
    ) -> Entity<TableState<AggregationTableDelegate>> {
        if let Some(agg_table) = &self.agg_table_state {
            return agg_table.clone();
        }
        let delegate = AggregationTableDelegate::new(state.clone(), self.current_session.clone());
        let agg_table = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .col_selectable(false)
                .col_movable(true)
                .row_selectable(false)
        });

        let state_clone = state.clone();
        cx.subscribe_in(&agg_table, window, move |cv, ts, event, _window, cx| {
            use gpui_kit::component::table::TableEvent;
            match event {
                TableEvent::ColumnWidthsChanged(widths) => {
                    let col_widths: HashMap<String, f32> = {
                        let delegate = ts.read(cx).delegate();
                        widths
                            .iter()
                            .enumerate()
                            .filter_map(|(i, w)| delegate.column_key(i).map(|k| (k, f32::from(*w))))
                            .collect()
                    };
                    ts.update(cx, |ts, _cx| {
                        ts.delegate_mut().update_saved_widths(col_widths.clone());
                    });
                    if let Some(sk) = cv.view_model.current_session() {
                        state_clone.update(cx, |state, cx| {
                            state.set_agg_table_column_widths(&sk, col_widths);
                            cx.notify();
                        });
                    }
                }
                TableEvent::MoveColumn(from_ix, to_ix) => {
                    let from_ix = *from_ix;
                    let to_ix = *to_ix;
                    let order = {
                        ts.update(cx, |ts, _cx| {
                            ts.delegate_mut().apply_column_move(from_ix, to_ix);
                            ts.delegate_mut().column_order()
                        })
                    };
                    if let Some(sk) = cv.view_model.current_session() {
                        state_clone.update(cx, |state, cx| {
                            state.set_agg_table_column_order(&sk, order);
                            cx.notify();
                        });
                    }
                    cv.view_model.invalidate_agg_table();
                    cx.notify();
                }
                _ => {}
            }
        })
        .detach();

        self.agg_table_state = Some(agg_table.clone());
        agg_table
    }

    pub fn rebuild_agg_table(
        &mut self,
        state: &Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<CollectionView>,
    ) {
        let Some(session_key) = self.current_session.clone() else {
            return;
        };
        let state_ref = state.read(cx);
        let Some(session) = state_ref.session(&session_key) else {
            return;
        };
        // Keyed by the results allocation: edits bump the run generation but keep results.
        let run_gen = session
            .data
            .aggregation
            .results
            .as_ref()
            .map_or(0, |results| std::sync::Arc::as_ptr(results) as u64);
        let gen_changed =
            self.agg_table_generation != Some(run_gen) || self.agg_table_state.is_none();

        if !gen_changed {
            return;
        }

        let documents =
            session.data.aggregation.results.as_ref().map(|r| (**r).clone()).unwrap_or_default();
        let saved_widths = session.view.agg_table_column_widths.clone();
        let saved_order = session.view.agg_table_column_order.clone();
        let pinned = session.view.agg_table_pinned_columns.clone();
        let hidden = session.view.agg_table_hidden_columns.clone();

        let agg_table = self.ensure_agg_table_state(state, window, cx);
        agg_table.update(cx, |ts, cx| {
            ts.delegate_mut().set_saved_widths(saved_widths);
            ts.delegate_mut().set_column_order(saved_order);
            ts.delegate_mut().set_pinned_columns(pinned);
            ts.delegate_mut().set_hidden_columns(hidden);
            ts.delegate_mut().refresh_data(documents, Some(session_key));
            ts.refresh(cx);
        });
        self.agg_table_generation = Some(run_gen);
    }

    pub fn invalidate_agg_table(&mut self) {
        self.agg_table_generation = None;
    }

    pub fn ensure_col_visibility_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<CollectionView>,
    ) -> Entity<InputState> {
        if let Some(ref state) = self.col_visibility_search {
            return state.clone();
        }
        let state = cx.new(|cx| InputState::new(window, cx).placeholder("Search columns…"));
        self.col_visibility_search = Some(state.clone());
        state
    }
}

#[cfg(test)]
mod inline_edit_tests {
    use gpui_kit::AppContext as _;
    use gpui_kit::component::input::InputState;
    use mongodb::bson::{Bson, doc};

    use super::CollectionView;
    use crate::bson::{DocumentKey, PathSegment, path_to_id};
    use crate::state::{AppState, SessionDocument, SessionKey};

    #[gpui_kit::test]
    fn cancel_restores_the_prior_draft_without_losing_other_field_changes(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let key = SessionKey::new(uuid::Uuid::new_v4(), "db", "collection");
            let original = doc! { "_id": 1, "enabled": false, "name": "original" };
            let document = DocumentKey::from_document(&original, 0);
            let previous = doc! { "_id": 1, "enabled": false, "name": "staged" };
            let state = cx.new(|_| AppState::new());
            state.update(cx, |state, _| {
                let session = state.ensure_session(key.clone());
                session.data.index_by_key.insert(document.clone(), 0);
                session.data.items.push(SessionDocument { key: document.clone(), doc: original });
                state.set_draft(&key, document.clone(), previous.clone());
            });
            let view = cx.new(|cx| CollectionView::new(state.clone(), cx));
            let input = cx.new(|cx| InputState::new(window, cx));
            input.update(cx, |input, cx| input.set_value("true", window, cx));
            view.update(cx, |view, cx| {
                let model = &mut view.view_model;
                let path = vec![PathSegment::Key("enabled".into())];
                model.current_session = Some(key.clone());
                model.editing_node_id = Some(path_to_id(&document, &path));
                model.editing_doc_key = Some(document.clone());
                model.editing_path = path;
                model.editing_original = Some(Bson::Boolean(false));
                model.editing_draft_before = Some(previous.clone());
                model.inline_editor_state = Some(input.clone());
                model.sync_inline_edit_draft(&state, cx);
                assert!(
                    state
                        .read(cx)
                        .session_draft(&key, &document)
                        .unwrap()
                        .get_bool("enabled")
                        .unwrap()
                );
                model.cancel_inline_edit(&state, cx);
                assert_eq!(state.read(cx).session_draft(&key, &document), Some(previous));
                assert!(model.editing_node_id().is_none());
                assert!(!state.read(cx).session_has_invalid_edit(&key));
            });
        });
    }

    #[gpui_kit::test]
    fn table_columns_do_not_carry_over_to_another_collection(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let connection = uuid::Uuid::new_v4();
            let first = SessionKey::new(connection, "db", "first");
            let second = SessionKey::new(connection, "db", "second");
            let state = cx.new(|_| AppState::new());
            state.update(cx, |state, _| {
                for (key, doc) in [
                    (&first, doc! { "_id": 1, "active": true, "color": "red" }),
                    (&second, doc! { "_id": 2, "name": "b" }),
                ] {
                    let doc_key = DocumentKey::from_document(&doc, 0);
                    let session = state.ensure_session(key.clone());
                    session.data.index_by_key.insert(doc_key.clone(), 0);
                    session.data.items.push(SessionDocument { key: doc_key, doc });
                }
            });
            let view = cx.new(|cx| CollectionView::new(state.clone(), cx));
            view.update(cx, |view, cx| {
                let entity = cx.entity();
                let model = &mut view.view_model;
                for key in [&first, &second] {
                    model.current_session = Some(key.clone());
                    model.invalidate_table();
                    model.rebuild_table(&state, &entity, window, cx);
                }
                let table = model.table_state().unwrap().read(cx);
                assert_eq!(table.delegate().all_column_keys(), ["_id", "name"]);
            });
        });
    }

    #[gpui_kit::test]
    fn table_cells_flip_booleans_select_text_and_tab_over_cells_that_do_not_edit_in_place(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let key = SessionKey::new(uuid::Uuid::new_v4(), "db", "accounts");
            let doc = doc! { "_id": 1, "active": true, "name": "Acme", "seats": 3, "tags": ["a"] };
            let doc_key = DocumentKey::from_document(&doc, 0);
            let state = cx.new(|_| AppState::new());
            state.update(cx, |state, _| {
                let session = state.ensure_session(key.clone());
                session.data.index_by_key.insert(doc_key.clone(), 0);
                session.data.items.push(SessionDocument { key: doc_key.clone(), doc });
            });
            let view = cx.new(|cx| CollectionView::new(state.clone(), cx));
            view.update(cx, |view, cx| {
                let entity = cx.entity();
                view.view_model.current_session = Some(key.clone());
                view.view_model.rebuild_table(&state, &entity, window, cx);
                let active = |cx: &gpui_kit::App| {
                    let draft = state.read(cx).session_draft(&key, &doc_key)?;
                    draft.get_bool("active").ok()
                };

                // A boolean flips; flipping it back leaves nothing to save.
                view.edit_table_cell(doc_key.clone(), "active".into(), window, cx);
                assert_eq!(active(cx), Some(false));
                view.edit_table_cell(doc_key.clone(), "active".into(), window, cx);
                assert_eq!(active(cx), None);

                // A string opens in its cell with the value selected, so typing replaces it.
                view.edit_table_cell(doc_key.clone(), "name".into(), window, cx);
                let input = view.view_model.inline_editor_for(&doc_key, "name").unwrap();
                assert_eq!(input.read(cx).selected_value().as_ref(), "Acme");

                // Tab moves on to the number, then finds nothing after the array and stops.
                assert!(view.tab_table_cell(false, window, cx));
                assert!(view.view_model.inline_editor_for(&doc_key, "seats").is_some());
                assert!(view.tab_table_cell(false, window, cx));
                assert!(view.view_model.inline_state().is_none());

                // Shift+Tab goes back over the boolean and stops before _id.
                view.edit_table_cell(doc_key.clone(), "seats".into(), window, cx);
                assert!(view.tab_table_cell(true, window, cx));
                assert!(view.view_model.inline_editor_for(&doc_key, "name").is_some());
                assert!(view.tab_table_cell(true, window, cx));
                assert!(view.view_model.inline_state().is_none());
            });
        });
    }

    #[test]
    fn picking_a_day_keeps_the_typed_time_and_offset() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 12, 24).unwrap();
        assert_eq!(super::with_date("2026-11-03T09:30:00+04:00", day), "2026-12-24T09:30:00+04:00");
        assert_eq!(super::with_date("2026-11-03", day), "2026-12-24");
        assert_eq!(super::with_date("not a date", day), "2026-12-24");
        assert_eq!(super::with_date("", day), "2026-12-24");
    }

    #[test]
    fn typing_a_time_keeps_the_day_and_offset() {
        use super::{time_parts, with_time};
        assert_eq!(time_parts("2026-11-03T09:30:05.120Z"), Some(["09", "30", "05"]));
        assert_eq!(time_parts("2026-11-03 09:30:05+04:00"), Some(["09", "30", "05"]));
        assert_eq!(time_parts("2026-11-03"), None);
        assert_eq!(time_parts("2026-11-03T9:30"), None);
        assert_eq!(with_time("2026-11-03T09:30:00+04:00", "18:05:59"), "2026-11-03T18:05:59+04:00");
        assert_eq!(with_time("2026-11-03T09:30:00.120Z", "18:05:59"), "2026-11-03T18:05:59Z");
        assert_eq!(with_time("2026-11-03 09:30", "18:05:59"), "2026-11-03T18:05:59");
        assert_eq!(with_time("2026-11-03", "18:05:59"), "2026-11-03T18:05:59");
        assert_eq!(with_time("not a date", "18:05:59"), "not a date");
        assert_eq!(super::time_zone_label("2026-11-03T09:30:00+04:00"), "+04:00");
        assert_eq!(super::time_zone_label("2026-11-03T09:30:00.5Z"), "UTC");
    }
}
