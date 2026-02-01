//! Source panel for transfer view.

use gpui::*;
use gpui_component::Sizable as _;
use gpui_component::select::Select;

use crate::state::{TransferMode, TransferScope, TransferTabState};
use crate::theme::spacing;

use super::QueryEditField;
use super::TransferView;
use super::helpers::{form_row, panel, render_query_field_row};

impl TransferView {
    /// Render the source panel with connection, database, and collection selectors.
    pub(super) fn render_source_panel(
        &self,
        transfer_state: &TransferTabState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let show_collection = matches!(transfer_state.scope, TransferScope::Collection);
        let show_query_options =
            matches!(transfer_state.mode, TransferMode::Export) && show_collection;

        // Searchable select components (states are initialized by ensure_select_states)
        let Some(ref source_conn_state) = self.source_conn_state else {
            return panel("Source", div().child("Loading...")).into_any_element();
        };
        let Some(ref source_db_state) = self.source_db_state else {
            return panel("Source", div().child("Loading...")).into_any_element();
        };

        let conn_select =
            Select::new(source_conn_state).small().w_full().placeholder("Select connection...");

        let db_select =
            Select::new(source_db_state).small().w_full().placeholder("Select database...");

        let coll_select = if show_collection {
            self.source_coll_state.as_ref().map(|coll_state| {
                Select::new(coll_state).small().w_full().placeholder("Select collection...")
            })
        } else {
            None
        };

        // Query options rows (Filter, Projection, Sort)
        let view = cx.entity();
        let state = self.state.clone();
        let query_options = if show_query_options {
            vec![
                render_query_field_row(
                    "Filter",
                    QueryEditField::Filter,
                    &transfer_state.export_filter,
                    view.clone(),
                    state.clone(),
                )
                .into_any_element(),
                render_query_field_row(
                    "Projection",
                    QueryEditField::Projection,
                    &transfer_state.export_projection,
                    view.clone(),
                    state.clone(),
                )
                .into_any_element(),
                render_query_field_row(
                    "Sort",
                    QueryEditField::Sort,
                    &transfer_state.export_sort,
                    view,
                    state,
                )
                .into_any_element(),
            ]
        } else {
            vec![]
        };

        panel(
            "Source",
            div()
                .flex()
                .flex_col()
                .gap(spacing::md())
                .child(form_row("Connection", conn_select))
                .child(form_row("Database", db_select))
                .children(coll_select.map(|s| form_row("Collection", s)))
                .children(query_options),
        )
        .into_any_element()
    }
}
