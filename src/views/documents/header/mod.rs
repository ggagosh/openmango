//! Header bar rendering for collection view.
//!
//! This module provides the header UI for collection views, including:
//! - Collection title and breadcrumb
//! - Action buttons (varies by subview)
//! - Subview tabs (Documents/Indexes/Stats/Aggregation)
//! - Filter bar and query options (Documents subview only)

mod actions;
mod filter_bar;
mod stats_panel;
mod tabs_row;

pub use actions::{
    render_aggregation_actions, render_documents_actions, render_indexes_actions,
    render_stats_actions,
};
pub use filter_bar::{render_filter_row, render_query_options};
pub use stats_panel::render_stats_row;
pub use tabs_row::render_subview_tabs;

use gpui::*;
use gpui_component::input::InputState;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::bson::DocumentKey;
use crate::helpers::format_number;
use crate::state::{CollectionSubview, SessionKey};
use crate::theme::{colors, spacing};

use super::CollectionView;

/// Render the header bar with collection title and action buttons.
impl CollectionView {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::views::documents) fn render_header(
        &self,
        collection_name: &str,
        db_name: &str,
        total: u64,
        session_key: Option<SessionKey>,
        selected_doc: Option<DocumentKey>,
        dirty_selected: bool,
        is_loading: bool,
        filter_state: Option<Entity<InputState>>,
        filter_active: bool,
        sort_state: Option<Entity<InputState>>,
        projection_state: Option<Entity<InputState>>,
        sort_active: bool,
        projection_active: bool,
        query_options_open: bool,
        active_subview: CollectionSubview,
        stats_loading: bool,
        aggregation_loading: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();
        let connection_name = {
            let state_ref = self.state.read(cx);
            state_ref
                .selected_connection_id()
                .and_then(|id| state_ref.connection_name(id))
                .unwrap_or_else(|| "Connection".to_string())
        };

        let is_documents = active_subview == CollectionSubview::Documents;
        let is_indexes = active_subview == CollectionSubview::Indexes;
        let is_stats = active_subview == CollectionSubview::Stats;
        let is_aggregation = active_subview == CollectionSubview::Aggregation;
        let breadcrumb = format!("{connection_name} / {db_name} / {collection_name}");

        // Build action row based on active subview
        let action_row = if is_documents {
            render_documents_actions(
                view,
                self.state.clone(),
                session_key.clone(),
                selected_doc,
                dirty_selected,
                is_loading,
                filter_active,
                cx,
            )
        } else if is_indexes {
            render_indexes_actions(self.state.clone(), session_key.clone())
        } else if is_stats {
            render_stats_actions(self.state.clone(), session_key.clone(), stats_loading)
        } else if is_aggregation {
            render_aggregation_actions(self.state.clone(), session_key.clone(), aggregation_loading)
        } else {
            div().flex().items_center().gap(spacing::sm())
        };

        // Build subview tabs
        let subview_tabs =
            render_subview_tabs(self.state.clone(), session_key.clone(), active_subview);

        // Build the root header container
        let mut root = div()
            .flex()
            .flex_col()
            .px(spacing::lg())
            .py(spacing::md())
            .gap(spacing::sm())
            .bg(colors::bg_header())
            .border_b_1()
            .border_color(colors::border())
            .on_mouse_down(MouseButton::Left, |_, window, _| {
                window.blur();
            })
            .child(render_title_row(collection_name, total, &breadcrumb, action_row))
            .child(div().pl(spacing::xs()).child(subview_tabs));

        // Add filter bar for documents subview
        if is_documents {
            root = root.child(render_filter_row(
                self.state.clone(),
                session_key.clone(),
                filter_state.clone(),
                filter_active,
                sort_active,
                projection_active,
                query_options_open,
            ));

            // Add query options panel if open
            if query_options_open {
                root = root.child(render_query_options(
                    self.state.clone(),
                    session_key.clone(),
                    sort_state,
                    projection_state,
                    sort_active,
                    projection_active,
                ));
            }
        }

        root
    }
}

/// Render the title row with collection name, doc count, breadcrumb, and actions.
fn render_title_row(collection_name: &str, total: u64, breadcrumb: &str, action_row: Div) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::sm())
                        .child(
                            Icon::new(IconName::Folder).small().text_color(colors::accent_green()),
                        )
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(colors::text_primary())
                                .font_family(crate::theme::fonts::heading())
                                .child(collection_name.to_string()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors::text_muted())
                                .child(format!("({} docs)", format_number(total))),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors::text_muted())
                        .truncate()
                        .child(breadcrumb.to_string()),
                ),
        )
        .child(action_row.flex_shrink_0())
}
