use std::collections::{HashMap, HashSet};

use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mongodb::bson::{Bson, Document};

use crate::bson::DocumentKey;
use crate::state::{AppCommands, AppState, SessionDocument, SessionKey};
use crate::theme::colors;
use crate::views::documents::CollectionView;

use super::cell_renderer;
use super::column_menu;
use super::column_schema::discover_columns;
use super::table_columns::TableColumns;

pub struct DocumentTableDelegate {
    pub table_cols: TableColumns,
    documents: Vec<SessionDocument>,
    drafts: HashMap<DocumentKey, Document>,
    selected_doc_keys: HashSet<DocumentKey>,
    anchor_row: Option<usize>,
    context_column: Option<usize>,
    state: Entity<AppState>,
    view: Entity<CollectionView>,
    pub session_key: Option<SessionKey>,
    is_loading: bool,
}

impl DocumentTableDelegate {
    pub fn new(
        state: Entity<AppState>,
        view: Entity<CollectionView>,
        session_key: Option<SessionKey>,
    ) -> Self {
        Self {
            table_cols: TableColumns::new(),
            documents: Vec::new(),
            drafts: HashMap::new(),
            selected_doc_keys: HashSet::new(),
            anchor_row: None,
            context_column: None,
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
        self.table_cols.refresh_columns(discovered);
        self.documents = documents;
        self.drafts = drafts;
    }

    pub fn set_selected_doc_keys(&mut self, keys: HashSet<DocumentKey>) {
        self.selected_doc_keys = keys;
    }

    pub fn set_saved_widths(&mut self, widths: HashMap<String, f32>) {
        self.table_cols.set_saved_widths(widths);
    }

    pub fn update_saved_widths(&mut self, widths: HashMap<String, f32>) {
        self.table_cols.update_saved_widths(widths);
    }

    pub fn set_column_order(&mut self, order: Vec<String>) {
        self.table_cols.set_column_order(order);
    }

    pub fn column_order(&self) -> Vec<String> {
        self.table_cols.column_order()
    }

    pub fn apply_column_move(&mut self, from_ix: usize, to_ix: usize) {
        self.table_cols.apply_column_move(from_ix, to_ix);
    }

    pub fn set_hidden_columns(&mut self, hidden: HashSet<String>) {
        self.table_cols.set_hidden_columns(hidden);
    }

    pub fn all_column_keys(&self) -> &[String] {
        self.table_cols.all_column_keys()
    }

    pub fn is_column_hidden(&self, key: &str) -> bool {
        self.table_cols.is_column_hidden(key)
    }

    pub fn set_pinned_columns(&mut self, pinned: HashSet<String>) {
        self.table_cols.set_pinned_columns(pinned);
    }

    pub fn toggle_pin_column(&mut self, col_key: &str) -> bool {
        self.table_cols.toggle_pin_column(col_key)
    }

    pub fn is_column_pinned(&self, col_ix: usize) -> bool {
        self.table_cols.is_column_pinned(col_ix)
    }

    pub fn cell_value_for_copy(&self, row_ix: usize, col_ix: usize) -> Option<String> {
        let value = self.cell_value(row_ix, col_ix)?;
        Some(crate::bson::bson_value_for_edit(value))
    }

    pub fn column_key(&self, col_ix: usize) -> Option<String> {
        self.table_cols.column_key(col_ix)
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
        let key = &self.table_cols.columns.get(col_ix)?.key;
        doc.get(key)
    }

    fn is_row_dirty(&self, row_ix: usize) -> bool {
        let Some(item) = self.documents.get(row_ix) else {
            return false;
        };
        self.drafts.get(&item.key).is_some_and(|draft| draft != &item.doc)
    }
}

impl TableDelegate for DocumentTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.table_cols.columns_count()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.documents.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        self.table_cols.column_def(col_ix).clone()
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let name = self.table_cols.column_key(col_ix).unwrap_or_default();
        let is_pinned = self.table_cols.is_column_pinned(col_ix);
        let col_key = name.clone();
        let state = self.state.clone();
        let session_key = self.session_key.clone();

        let pin_icon =
            if is_pinned { crate::assets::AppIcon::Pin } else { crate::assets::AppIcon::PinOff };
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
                    .hover(|s: gpui_kit::StyleRefinement| s.opacity(1.0).bg(muted_bg))
                    .when(!is_pinned, |this: Stateful<Div>| {
                        this.group_hover("col-header-group", |s: gpui_kit::StyleRefinement| {
                            s.opacity(0.5)
                        })
                    })
                    .child(Icon::new(pin_icon).with_size(px(12.0)).text_color(icon_color))
                    .on_mouse_down(MouseButton::Left, |_, _: &mut Window, cx: &mut App| {
                        cx.stop_propagation()
                    })
                    .on_click(cx.listener(move |ts, _, _window, cx| {
                        ts.delegate_mut().table_cols.toggle_pin_column(&col_key);
                        ts.delegate_mut().table_cols.rebuild_column_defs();
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
        self.is_loading && self.documents.is_empty()
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

        let mut row = div().id(("row", row_ix)).capture_any_mouse_down(cx.listener(
            |table, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Right {
                    table.delegate_mut().context_column = None;
                }
            },
        ));

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
        let content = self
            .cell_value(row_ix, col_ix)
            .map(|value| cell_renderer::render_cell(value, row_ix, col_ix, cx));
        div()
            .size_full()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |table, _, _, _| {
                    table.delegate_mut().context_column = Some(col_ix);
                }),
            )
            .children(content)
            .into_any_element()
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let message = self
            .session_key
            .as_ref()
            .and_then(|key| self.state.read(cx).session_data(key))
            .map(|data| {
                super::super::query::document_empty_message(
                    data.loaded,
                    data.query_error.is_some(),
                    data.filter.is_some(),
                )
            })
            .unwrap_or("No results yet");
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(message))
            .into_any_element()
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: gpui_kit::component::menu::PopupMenu,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> gpui_kit::component::menu::PopupMenu {
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

        column_menu::build_table_column_menu(
            menu,
            self.context_column,
            &self.table_cols.columns,
            self.table_cols.pinned_columns(),
            column_menu::ColumnMenuKind::Document,
            &self.state,
            &session_key,
            cx,
        )
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        let Some(col) = self.table_cols.columns.get(col_ix) else {
            return;
        };
        let Some(session_key) = self.session_key.clone() else {
            return;
        };

        // Track active sort so it survives column rebuilds.
        self.table_cols.active_sort = match sort {
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
