use std::collections::{HashMap, HashSet};

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::menu::PopupMenuItem;
use gpui_component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use mongodb::bson::{Bson, Document};

use crate::bson::{DocumentKey, bson_value_preview};
use crate::state::{AppCommands, AppState, SessionDocument, SessionKey};
use crate::theme::colors;
use crate::views::documents::CollectionView;

use super::column_schema::{
    MIN_COL_WIDTH, TableColumnDef, build_column_defs_with_overrides, discover_columns,
};

pub struct DocumentTableDelegate {
    documents: Vec<SessionDocument>,
    drafts: HashMap<DocumentKey, Document>,
    columns: Vec<TableColumnDef>,
    column_defs: Vec<Column>,
    selected_doc_keys: HashSet<DocumentKey>,
    saved_widths: HashMap<String, f32>,
    stable_column_keys: Vec<String>,
    stable_widths: HashMap<String, f32>,
    active_sort: Option<(String, ColumnSort)>,
    pinned_columns: HashSet<String>,
    hidden_columns: HashSet<String>,
    anchor_row: Option<usize>,
    state: Entity<AppState>,
    view: Entity<CollectionView>,
    session_key: Option<SessionKey>,
    is_loading: bool,
}

impl DocumentTableDelegate {
    pub fn new(
        state: Entity<AppState>,
        view: Entity<CollectionView>,
        session_key: Option<SessionKey>,
    ) -> Self {
        Self {
            documents: Vec::new(),
            drafts: HashMap::new(),
            columns: Vec::new(),
            column_defs: Vec::new(),
            selected_doc_keys: HashSet::new(),
            saved_widths: HashMap::new(),
            stable_column_keys: Vec::new(),
            stable_widths: HashMap::new(),
            active_sort: None,
            pinned_columns: HashSet::new(),
            hidden_columns: HashSet::new(),
            anchor_row: None,
            state,
            view,
            session_key,
            is_loading: false,
        }
    }

    pub fn refresh_data(
        &mut self,
        documents: Vec<SessionDocument>,
        drafts: HashMap<DocumentKey, Document>,
        session_key: Option<SessionKey>,
        is_loading: bool,
    ) {
        self.session_key = session_key;
        self.is_loading = is_loading;
        let discovered = discover_columns(&documents);
        let all = self.merge_stable_columns(discovered);
        self.columns = all.into_iter().filter(|c| !self.hidden_columns.contains(&c.key)).collect();
        self.column_defs = build_column_defs_with_overrides(
            &self.columns,
            &self.saved_widths,
            &self.active_sort,
            &self.pinned_columns,
        );
        self.documents = documents;
        self.drafts = drafts;
    }

    fn merge_stable_columns(&mut self, discovered: Vec<TableColumnDef>) -> Vec<TableColumnDef> {
        let discovered_map: HashMap<String, TableColumnDef> =
            discovered.into_iter().map(|c| (c.key.clone(), c)).collect();

        if self.stable_column_keys.is_empty() {
            let mut keys: Vec<String> = discovered_map.keys().cloned().collect();
            // Deterministic order: _id first, then alphabetical.
            keys.sort();
            if let Some(pos) = keys.iter().position(|k| k == "_id") {
                keys.swap(0, pos);
            }
            self.stable_column_keys = keys;
        } else {
            for key in discovered_map.keys() {
                if !self.stable_column_keys.contains(key) {
                    self.stable_column_keys.push(key.clone());
                }
            }
        }

        // Lock in widths: first-seen width wins, never changes on re-discovery.
        for (key, col) in &discovered_map {
            self.stable_widths.entry(key.clone()).or_insert(col.width);
        }

        self.stable_column_keys
            .iter()
            .map(|key| TableColumnDef {
                key: key.clone(),
                width: self.stable_widths.get(key).copied().unwrap_or(MIN_COL_WIDTH),
            })
            .collect()
    }

    pub fn set_selected_doc_keys(&mut self, keys: HashSet<DocumentKey>) {
        self.selected_doc_keys = keys;
    }

    pub fn set_saved_widths(&mut self, widths: HashMap<String, f32>) {
        self.saved_widths = widths;
    }

    pub fn update_saved_widths(&mut self, widths: HashMap<String, f32>) {
        self.saved_widths.extend(widths);
    }

    pub fn set_column_order(&mut self, order: Vec<String>) {
        if !order.is_empty() {
            self.stable_column_keys = order;
        }
    }

    pub fn column_order(&self) -> Vec<String> {
        self.stable_column_keys.clone()
    }

    pub fn apply_column_move(&mut self, from_ix: usize, to_ix: usize) {
        let from_key = match self.columns.get(from_ix) {
            Some(c) => c.key.clone(),
            None => return,
        };
        let to_key = match self.columns.get(to_ix) {
            Some(c) => c.key.clone(),
            None => return,
        };
        let Some(src) = self.stable_column_keys.iter().position(|k| k == &from_key) else {
            return;
        };
        let Some(dst) = self.stable_column_keys.iter().position(|k| k == &to_key) else {
            return;
        };
        let key = self.stable_column_keys.remove(src);
        self.stable_column_keys.insert(dst, key);
    }

    pub fn set_hidden_columns(&mut self, hidden: HashSet<String>) {
        self.hidden_columns = hidden;
    }

    pub fn all_column_keys(&self) -> &[String] {
        &self.stable_column_keys
    }

    pub fn is_column_hidden(&self, key: &str) -> bool {
        self.hidden_columns.contains(key)
    }

    pub fn set_pinned_columns(&mut self, pinned: HashSet<String>) {
        self.pinned_columns = pinned;
    }

    pub fn toggle_pin_column(&mut self, col_key: &str) -> bool {
        if self.pinned_columns.contains(col_key) {
            self.pinned_columns.remove(col_key);
            false
        } else {
            self.pinned_columns.insert(col_key.to_string());
            self.move_pinned_column_to_front(col_key);
            true
        }
    }

    fn move_pinned_column_to_front(&mut self, col_key: &str) {
        let Some(pos) = self.stable_column_keys.iter().position(|k| k == col_key) else {
            return;
        };
        let key = self.stable_column_keys.remove(pos);
        let insert_at = self
            .stable_column_keys
            .iter()
            .position(|k| k != "_id" && !self.pinned_columns.contains(k))
            .unwrap_or(self.stable_column_keys.len());
        self.stable_column_keys.insert(insert_at, key);
    }

    pub fn is_column_pinned(&self, col_ix: usize) -> bool {
        self.columns.get(col_ix).is_some_and(|c| self.pinned_columns.contains(&c.key))
    }

    fn rebuild_column_defs(&mut self) {
        let col_map: HashMap<String, TableColumnDef> =
            self.columns.drain(..).map(|c| (c.key.clone(), c)).collect();
        self.columns = self
            .stable_column_keys
            .iter()
            .filter(|k| !self.hidden_columns.contains(*k))
            .filter_map(|k| col_map.get(k).cloned())
            .collect();
        self.column_defs = build_column_defs_with_overrides(
            &self.columns,
            &self.saved_widths,
            &self.active_sort,
            &self.pinned_columns,
        );
    }

    pub fn cell_value_for_copy(&self, row_ix: usize, col_ix: usize) -> Option<String> {
        let value = self.cell_value(row_ix, col_ix)?;
        Some(crate::bson::bson_value_for_edit(value))
    }

    pub fn column_key(&self, col_ix: usize) -> Option<String> {
        self.columns.get(col_ix).map(|c| c.key.clone())
    }

    pub fn document_key(&self, row_ix: usize) -> Option<DocumentKey> {
        self.documents.get(row_ix).map(|item| item.key.clone())
    }

    fn resolved_doc(&self, row_ix: usize) -> Option<&Document> {
        let item = self.documents.get(row_ix)?;
        self.drafts.get(&item.key).or(Some(&item.doc))
    }

    fn cell_value(&self, row_ix: usize, col_ix: usize) -> Option<&Bson> {
        let doc = self.resolved_doc(row_ix)?;
        let key = &self.columns.get(col_ix)?.key;
        doc.get(key)
    }

    fn is_row_dirty(&self, row_ix: usize) -> bool {
        let Some(item) = self.documents.get(row_ix) else {
            return false;
        };
        self.drafts.get(&item.key).is_some_and(|draft| draft != &item.doc)
    }

    fn value_color(value: &Bson, cx: &App) -> Hsla {
        match value {
            Bson::String(_) | Bson::Symbol(_) => colors::syntax_string(cx),
            Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_) => {
                colors::syntax_number(cx)
            }
            Bson::Boolean(_) => colors::syntax_boolean(cx),
            Bson::Null | Bson::Undefined => colors::syntax_null(cx),
            Bson::ObjectId(_) => colors::syntax_object_id(cx),
            Bson::DateTime(_) | Bson::Timestamp(_) => colors::syntax_date(cx),
            Bson::RegularExpression(_)
            | Bson::JavaScriptCode(_)
            | Bson::JavaScriptCodeWithScope(_) => colors::syntax_comment(cx),
            Bson::Document(_) | Bson::Array(_) | Bson::Binary(_) => cx.theme().muted_foreground,
            _ => cx.theme().foreground,
        }
    }

    fn format_nested_preview(value: &Bson) -> String {
        match value {
            Bson::Document(doc) => {
                let ext = Bson::Document(doc.clone()).into_relaxed_extjson();
                crate::bson::format_relaxed_json_value(&ext)
            }
            Bson::Array(arr) => {
                let ext = Bson::Array(arr.clone()).into_relaxed_extjson();
                crate::bson::format_relaxed_json_value(&ext)
            }
            _ => bson_value_preview(value, 500),
        }
    }

    fn build_table_column_menu(
        &self,
        mut menu: gpui_component::menu::PopupMenu,
        selected_col: Option<usize>,
        session_key: &SessionKey,
        _cx: &App,
    ) -> gpui_component::menu::PopupMenu {
        let col_key = selected_col.and_then(|c| self.columns.get(c).map(|col| col.key.clone()));

        let Some(col_key) = col_key else {
            return menu;
        };

        let is_pinned = self.pinned_columns.contains(&col_key);
        let pin_label = if is_pinned { "Unpin Column" } else { "Pin Column" };
        let pin_icon = if is_pinned { IconName::PinOff } else { IconName::Pin };

        menu = menu.separator();
        menu = menu.item(
            PopupMenuItem::new("Sort Ascending").icon(Icon::new(IconName::ArrowUp)).on_click({
                let state = self.state.clone();
                let sk = session_key.clone();
                let key = col_key.clone();
                move |_, _window, cx| {
                    let raw = format!("{{\"{}\": 1}}", key);
                    let doc = mongodb::bson::doc! { &key: 1 };
                    let state = state.clone();
                    let sk = sk.clone();
                    state.update(cx, |s, cx| {
                        let proj_raw = s
                            .session_data(&sk)
                            .map(|d| d.projection_raw.clone())
                            .unwrap_or_default();
                        let proj = s.session_data(&sk).and_then(|d| d.projection.clone());
                        s.set_sort_projection(&sk, raw, Some(doc), proj_raw, proj);
                        cx.notify();
                    });
                    AppCommands::load_documents_for_session(state, sk, cx);
                }
            }),
        );
        menu = menu.item(
            PopupMenuItem::new("Sort Descending").icon(Icon::new(IconName::ArrowDown)).on_click({
                let state = self.state.clone();
                let sk = session_key.clone();
                let key = col_key.clone();
                move |_, _window, cx| {
                    let raw = format!("{{\"{}\": -1}}", key);
                    let doc = mongodb::bson::doc! { &key: -1 };
                    let state = state.clone();
                    let sk = sk.clone();
                    state.update(cx, |s, cx| {
                        let proj_raw = s
                            .session_data(&sk)
                            .map(|d| d.projection_raw.clone())
                            .unwrap_or_default();
                        let proj = s.session_data(&sk).and_then(|d| d.projection.clone());
                        s.set_sort_projection(&sk, raw, Some(doc), proj_raw, proj);
                        cx.notify();
                    });
                    AppCommands::load_documents_for_session(state, sk, cx);
                }
            }),
        );
        menu = menu.separator();
        menu = menu.item(PopupMenuItem::new(pin_label).icon(Icon::new(pin_icon)).on_click({
            let state = self.state.clone();
            let sk = session_key.clone();
            let key = col_key.clone();
            move |_, _window, cx| {
                state.update(cx, |s, cx| {
                    s.toggle_table_pinned_column(&sk, key.clone());
                    cx.notify();
                });
            }
        }));
        menu = menu.item(
            PopupMenuItem::new("Hide Column").icon(Icon::new(IconName::EyeOff)).on_click({
                let state = self.state.clone();
                let sk = session_key.clone();
                let key = col_key.clone();
                move |_, _window, cx| {
                    state.update(cx, |s, cx| {
                        s.toggle_table_hidden_column(&sk, key.clone());
                        cx.notify();
                    });
                }
            }),
        );

        menu
    }
}

impl TableDelegate for DocumentTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.column_defs.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.documents.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> &Column {
        &self.column_defs[col_ix]
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let name = self.columns.get(col_ix).map(|c| c.key.clone()).unwrap_or_default();
        let is_pinned = self.is_column_pinned(col_ix);
        let col_key = name.clone();
        let state = self.state.clone();
        let session_key = self.session_key.clone();

        let pin_icon = if is_pinned { IconName::Pin } else { IconName::PinOff };
        let pin_opacity: f32 = if is_pinned { 1.0 } else { 0.0 };
        let muted_bg = cx.theme().muted;
        let icon_color = if is_pinned { cx.theme().primary } else { cx.theme().muted_foreground };

        div()
            .id(("th-pin", col_ix))
            .size_full()
            .flex()
            .items_center()
            .gap_1()
            .group("col-header-group")
            .child(name)
            .child(
                div()
                    .id(("pin-btn", col_ix))
                    .flex_shrink_0()
                    .cursor_pointer()
                    .rounded_sm()
                    .p(px(1.0))
                    .opacity(pin_opacity)
                    .hover(|s: gpui::StyleRefinement| s.opacity(1.0).bg(muted_bg))
                    .when(!is_pinned, |this: Stateful<Div>| {
                        this.group_hover("col-header-group", |s: gpui::StyleRefinement| {
                            s.opacity(0.5)
                        })
                    })
                    .child(Icon::new(pin_icon).with_size(px(12.0)).text_color(icon_color))
                    .on_mouse_down(MouseButton::Left, |_, _: &mut Window, cx: &mut App| {
                        cx.stop_propagation()
                    })
                    .on_click(cx.listener(move |ts, _, _window, cx| {
                        ts.delegate_mut().toggle_pin_column(&col_key);
                        ts.delegate_mut().rebuild_column_defs();
                        ts.refresh(cx);
                        if let Some(sk) = session_key.as_ref() {
                            let key = col_key.clone();
                            state.update(cx, |app_state, cx| {
                                app_state.toggle_table_pinned_column(sk, key);
                                cx.notify();
                            });
                        }
                    })),
            )
            .into_any_element()
    }

    fn loading(&self, _cx: &App) -> bool {
        self.is_loading
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let is_selected =
            self.documents.get(row_ix).is_some_and(|d| self.selected_doc_keys.contains(&d.key));
        let is_dirty = self.is_row_dirty(row_ix);

        let selected_bg = cx.theme().list_active;

        let mut row = div().id(("row", row_ix));

        if is_dirty {
            row = row.bg(colors::bg_dirty(cx));
        } else if is_selected {
            row = row.bg(selected_bg);
        }

        row = row.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |ts, event: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();

                let is_shift = event.modifiers.shift;
                let is_cmd = event.modifiers.secondary() || event.modifiers.control;

                let doc_key = ts.delegate().document_key(row_ix);
                let session_key = ts.delegate().session_key.clone();

                let (Some(dk), Some(sk)) = (doc_key, session_key) else {
                    return;
                };

                let state = ts.delegate().state.clone();

                if is_shift {
                    let anchor = ts.delegate().anchor_row.unwrap_or(0);
                    let lo = anchor.min(row_ix);
                    let hi = anchor.max(row_ix);
                    let doc_keys: HashSet<DocumentKey> =
                        (lo..=hi).filter_map(|i| ts.delegate().document_key(i)).collect();
                    let selected_keys = doc_keys.clone();
                    state.update(cx, |s, cx| {
                        s.select_doc_range(&sk, doc_keys, dk.clone(), String::new());
                        cx.notify();
                    });
                    ts.delegate_mut().selected_doc_keys = selected_keys;
                } else if is_cmd {
                    state.update(cx, |s, cx| {
                        s.toggle_doc_selection(&sk, &dk);
                        cx.notify();
                    });
                    if ts.delegate().selected_doc_keys.contains(&dk) {
                        ts.delegate_mut().selected_doc_keys.remove(&dk);
                    } else {
                        ts.delegate_mut().selected_doc_keys.insert(dk);
                    }
                    ts.delegate_mut().anchor_row = Some(row_ix);
                } else {
                    state.update(cx, |s, cx| {
                        s.select_single_doc(&sk, dk.clone(), String::new());
                        cx.notify();
                    });
                    let mut keys = HashSet::new();
                    keys.insert(dk);
                    ts.delegate_mut().selected_doc_keys = keys;
                    ts.delegate_mut().anchor_row = Some(row_ix);
                }

                cx.notify();
            }),
        );

        row
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(value) = self.cell_value(row_ix, col_ix) else {
            return div().text_xs().text_color(cx.theme().muted_foreground).into_any_element();
        };

        let text = bson_value_preview(value, 80);
        let color = Self::value_color(value, cx);

        let is_nested = matches!(value, Bson::Document(_) | Bson::Array(_));

        if is_nested {
            let preview = Self::format_nested_preview(value);
            div()
                .id(ElementId::Name(format!("cell-{}-{}", row_ix, col_ix).into()))
                .text_xs()
                .text_color(color)
                .cursor_pointer()
                .tooltip(move |_window, cx| {
                    cx.new(|_cx| gpui_component::tooltip::Tooltip::new(preview.clone())).into()
                })
                .child(text)
                .into_any_element()
        } else {
            div()
                .text_xs()
                .text_color(color)
                .whitespace_nowrap()
                .text_ellipsis()
                .overflow_x_hidden()
                .child(text)
                .into_any_element()
        }
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div().text_sm().text_color(cx.theme().muted_foreground).child("No documents found"),
            )
            .into_any_element()
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        selected_col: Option<usize>,
        menu: gpui_component::menu::PopupMenu,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> gpui_component::menu::PopupMenu {
        let Some(item) = self.documents.get(row_ix) else {
            return menu;
        };
        let Some(session_key) = self.session_key.clone() else {
            return menu;
        };
        let doc_key = item.key.clone();
        let is_dirty = self.is_row_dirty(row_ix);
        let selected_count = {
            let state_ref = self.state.read(cx);
            state_ref.session_view(&session_key).map(|v| v.selected_docs.len().max(1)).unwrap_or(1)
        };

        let menu = crate::views::documents::tree::tree_menus::build_document_menu(
            menu,
            self.state.clone(),
            self.view.clone(),
            session_key.clone(),
            doc_key,
            is_dirty,
            selected_count,
            crate::state::DocumentViewMode::Table,
            _window,
            &mut *cx,
        );

        self.build_table_column_menu(menu, selected_col, &session_key, cx)
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        let Some(col) = self.columns.get(col_ix) else {
            return;
        };
        let Some(session_key) = self.session_key.clone() else {
            return;
        };

        // Track active sort so it survives column rebuilds.
        self.active_sort = match sort {
            ColumnSort::Default => None,
            _ => Some((col.key.clone(), sort)),
        };

        let (sort_raw, sort_doc) = match sort {
            ColumnSort::Ascending => {
                let raw = format!("{{\"{}\": 1}}", col.key);
                let doc = mongodb::bson::doc! { &col.key: 1 };
                (raw, Some(doc))
            }
            ColumnSort::Descending => {
                let raw = format!("{{\"{}\": -1}}", col.key);
                let doc = mongodb::bson::doc! { &col.key: -1 };
                (raw, Some(doc))
            }
            ColumnSort::Default => ("{}".to_string(), None),
        };

        let state = self.state.clone();
        state.update(cx, |state, cx| {
            let projection_raw = state
                .session_data(&session_key)
                .map(|d| d.projection_raw.clone())
                .unwrap_or_default();
            let projection = state.session_data(&session_key).and_then(|d| d.projection.clone());
            state.set_sort_projection(&session_key, sort_raw, sort_doc, projection_raw, projection);
            cx.notify();
        });
        AppCommands::load_documents_for_session(state, session_key, cx);
    }
}
