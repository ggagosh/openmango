//! Header bar rendering for collection view.
//!
//! This module provides the header UI for collection views, including:
//! - Collection title and breadcrumb
//! - Action buttons (varies by subview)
//! - Subview tabs (Documents/Indexes/Stats/Aggregation/Schema)
//! - Filter bar and query options (Documents subview only)

mod actions;
mod filter_bar;
mod navigation_trail;
mod relations_chip;
mod stats_panel;
mod tabs_row;

pub use actions::{
    render_aggregation_actions, render_documents_actions, render_indexes_actions,
    render_pending_changes, render_schema_actions, render_stats_actions,
};
pub use filter_bar::render_query_options;
pub use navigation_trail::render_navigation_trail;
use relations_chip::render_relations_chip;
pub use stats_panel::render_stats_row;
pub use tabs_row::render_subview_tabs;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{EditorState, InputState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::bson::DocumentKey;
use crate::helpers::format_number;
use crate::models::CollectionDetail;
use crate::state::{AppState, CollectionSubview, SessionKey};
use crate::theme::{islands, spacing};

use super::CollectionView;

fn header_container(background: Hsla) -> Div {
    div().flex().flex_col().px(spacing::lg()).py(spacing::sm()).gap(px(2.0)).bg(background)
}

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
        selected_count: usize,
        dirty_count: usize,
        is_loading: bool,
        sort_state: Option<Entity<EditorState>>,
        projection_state: Option<Entity<EditorState>>,
        sort_valid: bool,
        projection_valid: bool,
        sort_active: bool,
        projection_active: bool,
        query_options_open: bool,
        active_subview: CollectionSubview,
        stats_loading: bool,
        aggregation_loading: bool,
        explain_loading: bool,
        schema_loading: bool,
        col_visibility_search: Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();
        let (connection_name, appearance) = {
            let state_ref = self.state.read(cx);
            (
                session_key
                    .as_ref()
                    .map(|key| key.connection_id)
                    .and_then(|id| state_ref.connection_name(id))
                    .unwrap_or_else(|| "Connection".to_string()),
                state_ref.settings.appearance.clone(),
            )
        };

        let is_documents = active_subview == CollectionSubview::Documents;
        let filter_active = session_key
            .as_ref()
            .and_then(|key| self.state.read(cx).session_data(key))
            .is_some_and(|data| data.filter.is_some());
        let is_indexes = active_subview == CollectionSubview::Indexes;
        let is_stats = active_subview == CollectionSubview::Stats;
        let is_aggregation = active_subview == CollectionSubview::Aggregation;
        let is_schema = active_subview == CollectionSubview::Schema;
        let is_history = active_subview == CollectionSubview::History;
        let breadcrumb = format!("{connection_name} / {db_name} / {collection_name}");

        // Build action row based on active subview
        let mut documents_toolbar_row: Option<Div> = None;
        let action_row = if is_documents {
            let table_column_keys: Vec<String> = self
                .view_model
                .table_state()
                .map(|ts| ts.read(cx).delegate().all_column_keys().to_vec())
                .unwrap_or_default();
            let docs_actions = render_documents_actions(
                view,
                self.state.clone(),
                session_key.clone(),
                selected_doc,
                selected_count,
                filter_active,
                table_column_keys,
                col_visibility_search,
                cx,
            );
            documents_toolbar_row = Some(docs_actions);
            render_pending_changes(
                cx.entity(),
                self.state.clone(),
                session_key.clone(),
                dirty_count,
                window,
                cx,
            )
        } else if is_indexes {
            render_indexes_actions(self.state.clone(), session_key.clone())
        } else if is_stats {
            render_stats_actions(self.state.clone(), session_key.clone(), stats_loading)
        } else if is_aggregation {
            let editing_view = session_key.as_ref().and_then(|key| {
                self.state.read(cx).session(key)?.data.aggregation.editing_view.clone()
            });
            render_aggregation_actions(
                self.state.clone(),
                session_key.clone(),
                aggregation_loading,
                explain_loading,
                editing_view,
            )
        } else if is_schema {
            render_schema_actions(self.state.clone(), session_key.clone(), schema_loading)
        } else if is_history {
            div().flex().items_center()
        } else {
            div().flex().items_center().gap(spacing::sm())
        };

        // Build subview tabs
        let subview_tabs = render_subview_tabs(
            cx.entity(),
            self.state.clone(),
            session_key.clone(),
            active_subview,
            cx,
        );

        // Build the root header container
        let mut root = header_container(islands::tool_bg(&appearance, cx))
            .child(render_title_row(
                collection_name,
                total,
                &breadcrumb,
                render_kind_badges(&self.state, session_key.as_ref(), cx),
                render_relations_chip(&self.state, session_key.as_ref(), cx),
                action_row,
                cx,
            ))
            .children(render_navigation_trail(&self.state, window, cx))
            .child(div().pl(px(0.0)).child(subview_tabs))
            .when_some(documents_toolbar_row, |s, row| {
                s.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_start()
                        .w_full()
                        .min_w(px(0.0))
                        .pt(px(1.0))
                        .pb(px(1.0))
                        .child(row),
                )
            });

        // Add filter bar for documents subview
        if is_documents {
            root = root.child(self.render_filter_row(is_loading, explain_loading, window, cx));

            // Add query options panel if open
            if query_options_open {
                root = root.child(render_query_options(
                    self.state.clone(),
                    session_key.clone(),
                    sort_state,
                    projection_state,
                    sort_valid,
                    projection_valid,
                    sort_active,
                    projection_active,
                    window,
                    cx,
                ));
            }
        }

        root
    }
}

struct KindBadges {
    is_view: bool,
    element: AnyElement,
}

/// What this namespace is and why it refuses writes, said beside its name. "View" and
/// "read-only" are separate facts with separate tags: a plain collection on a read-only
/// connection gets the second without the first. A view names its source as a link, because
/// that is where an edit has to go.
fn render_kind_badges(
    state: &Entity<AppState>,
    session_key: Option<&SessionKey>,
    cx: &App,
) -> Option<KindBadges> {
    let key = session_key?;
    let state_ref = state.read(cx);
    let detail = state_ref.collection_detail(key);
    let read_only_reason = state_ref.session_read_only_reason(key);
    if detail.is_none() && read_only_reason.is_none() {
        return None;
    }

    let source = state_ref.view_source(key).map(str::to_owned);
    let row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(spacing::xs())
        .min_w(px(0.0))
        .when(matches!(detail, Some(CollectionDetail::Timeseries)), |row| {
            row.child(Tag::secondary().xsmall().child("TIME SERIES"))
        })
        .when_some(source.clone(), |row, source| {
            let state_for_link = state.clone();
            let database = key.database.clone();
            row.child(Tag::info().xsmall().child("VIEW"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child("on"))
                .child(
                    div()
                        .id("view-source-link")
                        .text_sm()
                        .text_color(cx.theme().link)
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .tooltip({
                            let source = source.clone();
                            move |window, cx| {
                                Tooltip::new(format!(
                                    "Open {source}, the collection this view reads"
                                ))
                                .build(window, cx)
                            }
                        })
                        .on_click({
                            let source = source.clone();
                            move |_, _, cx| {
                                state_for_link.update(cx, |state, cx| {
                                    state.select_collection(database.clone(), source.clone(), cx);
                                });
                            }
                        })
                        .child(source),
                )
                .child(
                    crate::components::Button::new("edit-view-definition")
                        .xsmall()
                        .ghost()
                        .label("Edit definition")
                        .tooltip("Open this view's pipeline in the aggregation builder")
                        .on_click({
                            let state = state.clone();
                            let key = key.clone();
                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                crate::state::AppCommands::edit_view_definition(
                                    state.clone(),
                                    key.connection_id,
                                    key.database.clone(),
                                    key.collection.clone(),
                                    cx,
                                );
                            }
                        }),
                )
        })
        .when_some(read_only_reason, |row, reason| {
            row.child(
                div()
                    .id("read-only-badge")
                    .tooltip(move |window, cx| Tooltip::new(reason.clone()).build(window, cx))
                    .child(Tag::secondary().xsmall().child("READ-ONLY")),
            )
        });

    Some(KindBadges { is_view: source.is_some(), element: row.into_any_element() })
}

/// Render the title row with collection name, doc count, breadcrumb, and actions.
fn render_title_row(
    collection_name: &str,
    total: u64,
    breadcrumb: &str,
    kind: Option<KindBadges>,
    relations: Option<AnyElement>,
    action_row: Div,
    cx: &App,
) -> Div {
    // Narrow windows: the title keeps its natural width as its flex basis, so when it and the
    // actions no longer fit side by side the actions wrap below instead of covering it. What
    // still doesn't fit wraps in turn: chips under the name, buttons under buttons.
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_x(spacing::md())
        .gap_y(spacing::sm())
        .child(
            div()
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .flex_auto()
                .min_w(px(0.0))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(spacing::sm())
                        .gap_y(spacing::xs())
                        .min_w(px(0.0))
                        .child(
                            Icon::new(if kind.as_ref().is_some_and(|kind| kind.is_view) {
                                IconName::Eye
                            } else {
                                IconName::Folder
                            })
                            .small()
                            .text_color(cx.theme().primary),
                        )
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(cx.theme().foreground)
                                .font_family(crate::theme::fonts::heading())
                                .min_w(px(0.0))
                                .max_w_full()
                                .truncate()
                                .debug_selector(|| "collection-title".into())
                                .child(collection_name.to_string()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("({} docs)", format_number(total))),
                        )
                        .children(kind.map(|kind| kind.element))
                        .children(relations),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(breadcrumb.to_string()),
                ),
        )
        .child(action_row.flex_wrap().gap_y(spacing::xs()).max_w_full())
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::Root;
    use gpui_kit::{
        AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, ParentElement as _,
        Pixels, Render, Styled as _, TestAppContext, Window, div, px,
    };

    use super::render_title_row;
    use crate::components::Button;

    struct TitleRow(f32);

    impl Render for TitleRow {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let actions = div()
                .flex()
                .items_center()
                .debug_selector(|| "title-actions".into())
                .child(Button::new("run").label("Run"))
                .child(Button::new("explain").label("Explain"))
                .child(Button::new("update").label("Update view auditlogs_by_month"));
            let chip = div().child("2 relations").into_any_element();
            div().w(px(self.0)).child(render_title_row(
                "auditlogs",
                3229,
                "Local / au_new / auditlogs",
                None,
                Some(chip),
                actions,
                cx,
            ))
        }
    }

    fn bounds_at(width: f32, cx: &mut TestAppContext) -> (Bounds<Pixels>, Bounds<Pixels>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply_design_tokens(cx);
        });
        let (_, cx) = cx.add_window_view(|window, cx| {
            let row = cx.new(|_| TitleRow(width));
            Root::new(row, window, cx).bordered(false)
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (
            cx.debug_bounds("collection-title").expect("title"),
            cx.debug_bounds("title-actions").expect("actions"),
        )
    }

    /// The width of the window in the bug report: the buttons used to be painted over the name.
    #[gpui_kit::test]
    fn a_narrow_header_puts_its_actions_below_the_title(cx: &mut TestAppContext) {
        let (title, actions) = bounds_at(430.0, cx);
        assert!(!title.intersects(&actions), "{title:?} overlaps {actions:?}");
        assert!(actions.top() >= title.bottom(), "actions should wrap below the title");
        assert!(actions.right() <= px(430.0), "actions must stay inside the header");
    }

    #[gpui_kit::test]
    fn a_wide_header_keeps_its_actions_beside_the_title(cx: &mut TestAppContext) {
        let (title, actions) = bounds_at(1200.0, cx);
        assert!(actions.top() < title.bottom(), "actions should share the title's line");
        assert!(actions.left() > title.right());
    }
}
