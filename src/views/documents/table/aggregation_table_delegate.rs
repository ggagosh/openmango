use std::collections::HashSet;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::table::{Column, ColumnSort, TableDelegate, TableState};
use mongodb::bson::{Bson, Document};

use crate::state::{AppState, SessionKey};

use super::cell_renderer;
use super::column_schema::discover_columns_raw;
use super::table_columns::TableColumns;

pub struct AggregationTableDelegate {
    pub table_cols: TableColumns,
    documents: Vec<Document>,
    selected_rows: HashSet<usize>,
    anchor_row: Option<usize>,
    state: Entity<AppState>,
    pub session_key: Option<SessionKey>,
}

impl AggregationTableDelegate {
    pub fn new(state: Entity<AppState>, session_key: Option<SessionKey>) -> Self {
        Self {
            table_cols: TableColumns::new(),
            documents: Vec::new(),
            selected_rows: HashSet::new(),
            anchor_row: None,
            state,
            session_key,
        }
    }

    pub fn refresh_data(&mut self, documents: Vec<Document>, session_key: Option<SessionKey>) {
        self.session_key = session_key;
        let discovered = discover_columns_raw(&documents);
        self.table_cols.refresh_columns(discovered);
        self.documents = documents;
        self.selected_rows.clear();
        self.anchor_row = None;
    }

    pub fn set_saved_widths(&mut self, widths: std::collections::HashMap<String, f32>) {
        self.table_cols.set_saved_widths(widths);
    }

    pub fn update_saved_widths(&mut self, widths: std::collections::HashMap<String, f32>) {
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

    pub fn set_pinned_columns(&mut self, pinned: HashSet<String>) {
        self.table_cols.set_pinned_columns(pinned);
    }

    pub fn column_key(&self, col_ix: usize) -> Option<String> {
        self.table_cols.column_key(col_ix)
    }

    pub fn cell_value_for_copy(&self, row_ix: usize, col_ix: usize) -> Option<String> {
        let value = self.cell_value(row_ix, col_ix)?;
        Some(crate::bson::bson_value_for_edit(value))
    }

    pub fn documents(&self) -> &[Document] {
        &self.documents
    }

    fn cell_value(&self, row_ix: usize, col_ix: usize) -> Option<&Bson> {
        let doc = self.documents.get(row_ix)?;
        let key = &self.table_cols.columns.get(col_ix)?.key;
        doc.get(key)
    }

    fn client_side_sort(&mut self, col_key: &str, sort: ColumnSort) {
        match sort {
            ColumnSort::Ascending => {
                self.documents.sort_by(|a, b| cmp_bson_opt(a.get(col_key), b.get(col_key)));
            }
            ColumnSort::Descending => {
                self.documents.sort_by(|a, b| cmp_bson_opt(b.get(col_key), a.get(col_key)));
            }
            ColumnSort::Default => {}
        }
    }
}

fn cmp_bson_opt(a: Option<&Bson>, b: Option<&Bson>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a), Some(b)) => cmp_bson(a, b),
    }
}

fn cmp_bson(a: &Bson, b: &Bson) -> std::cmp::Ordering {
    match (a, b) {
        (Bson::Int32(a), Bson::Int32(b)) => a.cmp(b),
        (Bson::Int64(a), Bson::Int64(b)) => a.cmp(b),
        (Bson::Double(a), Bson::Double(b)) => a.total_cmp(b),
        (Bson::String(a), Bson::String(b)) => a.cmp(b),
        (Bson::Boolean(a), Bson::Boolean(b)) => a.cmp(b),
        (Bson::DateTime(a), Bson::DateTime(b)) => a.timestamp_millis().cmp(&b.timestamp_millis()),
        (Bson::ObjectId(a), Bson::ObjectId(b)) => a.cmp(b),
        _ => {
            let a_str = crate::bson::bson_value_preview(a, 100);
            let b_str = crate::bson::bson_value_preview(b, 100);
            a_str.cmp(&b_str)
        }
    }
}

impl TableDelegate for AggregationTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.table_cols.columns_count()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.documents.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> &Column {
        self.table_cols.column_def(col_ix)
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

        use gpui_component::{Icon, IconName, Sizable as _};

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
            .group("agg-col-header-group")
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
                        this.group_hover("agg-col-header-group", |s: gpui::StyleRefinement| {
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
                                app_state.toggle_agg_table_pinned_column(sk, key);
                                cx.notify();
                            });
                        }
                    })),
            )
            .into_any_element()
    }

    fn loading(&self, _cx: &App) -> bool {
        false
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let is_selected = self.selected_rows.contains(&row_ix);
        let selected_bg = cx.theme().list_active;

        let mut row = div().id(("agg-row", row_ix));

        if is_selected {
            row = row.bg(selected_bg);
        }

        row = row.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |ts, event: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();

                let is_shift = event.modifiers.shift;
                let is_cmd = event.modifiers.secondary() || event.modifiers.control;

                if is_shift {
                    let anchor = ts.delegate().anchor_row.unwrap_or(0);
                    let lo = anchor.min(row_ix);
                    let hi = anchor.max(row_ix);
                    ts.delegate_mut().selected_rows = (lo..=hi).collect();
                } else if is_cmd {
                    if ts.delegate().selected_rows.contains(&row_ix) {
                        ts.delegate_mut().selected_rows.remove(&row_ix);
                    } else {
                        ts.delegate_mut().selected_rows.insert(row_ix);
                    }
                    ts.delegate_mut().anchor_row = Some(row_ix);
                } else {
                    let mut sel = HashSet::new();
                    sel.insert(row_ix);
                    ts.delegate_mut().selected_rows = sel;
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

        cell_renderer::render_cell(value, row_ix, col_ix, cx)
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
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No aggregation results"),
            )
            .into_any_element()
    }

    fn context_menu(
        &mut self,
        _row_ix: usize,
        selected_col: Option<usize>,
        menu: gpui_component::menu::PopupMenu,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> gpui_component::menu::PopupMenu {
        let Some(session_key) = self.session_key.clone() else {
            return menu;
        };

        super::column_menu::build_table_column_menu(
            menu,
            selected_col,
            &self.table_cols.columns,
            self.table_cols.pinned_columns(),
            super::column_menu::ColumnMenuKind::Aggregation,
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
        _cx: &mut Context<TableState<Self>>,
    ) {
        let Some(col) = self.table_cols.columns.get(col_ix) else {
            return;
        };

        self.table_cols.active_sort = match sort {
            ColumnSort::Default => None,
            _ => Some((col.key.clone(), sort)),
        };

        self.client_side_sort(&col.key.clone(), sort);
        self.selected_rows.clear();
        self.anchor_row = None;
    }
}
