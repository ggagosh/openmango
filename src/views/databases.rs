use gpui::*;
use gpui_component::Sizable as _;
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;

use crate::components::Button;
use crate::helpers::{format_bytes, format_number};
use crate::state::{
    AppCommands, AppEvent, AppState, CollectionOverview, DatabaseKey, DatabaseStats, View,
};
use crate::theme::{borders, colors, spacing, sizing};

/// Database overview view (stats + collections list)
pub struct DatabaseView {
    state: Entity<AppState>,
    last_database_key: Option<DatabaseKey>,
    _subscriptions: Vec<Subscription>,
}

impl DatabaseView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];
        let current_key = state.read(cx).current_database_key();

        if let Some(key) = current_key.clone() {
            AppCommands::load_database_overview(state.clone(), key, false, cx);
        }

        subscriptions.push(cx.subscribe(&state, |this, state, event, cx| match event {
            AppEvent::ViewChanged | AppEvent::Connected(_) => {
                let state_ref = state.read(cx);
                if matches!(state_ref.current_view, View::Database) {
                    let key = state_ref.current_database_key();
                    if key != this.last_database_key {
                        if let Some(key) = key.clone() {
                            AppCommands::load_database_overview(state.clone(), key, false, cx);
                        }
                        this.last_database_key = key;
                    }
                }
                cx.notify();
            }
            _ => {}
        }));

        Self { state, last_database_key: current_key, _subscriptions: subscriptions }
    }
}

impl Render for DatabaseView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_ref = self.state.read(cx);
        let database_name =
            state_ref.conn.selected_database.clone().unwrap_or_else(|| "Database".to_string());
        let database_key = state_ref.current_database_key();

        if database_key.is_none() {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(colors::text_muted())
                .child("Select a database to view overview details")
                .into_any_element();
        }

        let (
            stats,
            stats_loading,
            stats_error,
            collections,
            collections_loading,
            collections_error,
        ) = if let Some(key) = database_key.as_ref()
            && let Some(session) = state_ref.database_session(key)
        {
            (
                session.data.stats.clone(),
                session.data.stats_loading,
                session.data.stats_error.clone(),
                session.data.collections.clone(),
                session.data.collections_loading,
                session.data.collections_error.clone(),
            )
        } else {
            (
                None,
                false,
                None,
                Vec::new(),
                false,
                None,
            )
        };

        let state = self.state.clone();
        let refresh_button = Button::new("refresh-db")
            .ghost()
            .compact()
            .label("Refresh")
            .disabled(database_key.is_none())
            .on_click({
                let state = state.clone();
                let key = database_key.clone();
                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    let Some(key) = key.clone() else {
                        return;
                    };
                    AppCommands::load_database_overview(state.clone(), key, true, cx);
                }
            });

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .h(sizing::header_height())
            .px(spacing::lg())
            .bg(colors::bg_header())
            .border_b_1()
            .border_color(colors::border())
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors::text_primary())
                    .child(database_name.clone()),
            )
            .child(refresh_button);

        let content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .overflow_y_scrollbar()
            .child(Self::render_stats_section(
                stats,
                stats_loading,
                stats_error,
                database_key.clone(),
                state.clone(),
            ))
            .child(Self::render_collections_section(
                collections,
                collections_loading,
                collections_error,
                database_name,
                state.clone(),
            ));

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .bg(colors::bg_app())
            .child(header)
            .child(content)
            .into_any_element()
    }
}

impl DatabaseView {
    fn render_stats_section(
        stats: Option<DatabaseStats>,
        stats_loading: bool,
        stats_error: Option<String>,
        database_key: Option<crate::state::DatabaseKey>,
        state: Entity<AppState>,
    ) -> AnyElement {
        let mut section = div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .px(spacing::lg())
            .pt(spacing::lg());

        section = section.child(
            div()
                .text_xs()
                .text_color(colors::text_muted())
                .child("Database stats"),
        );

        let mut row = div()
            .flex()
            .items_center()
            .gap(spacing::lg())
            .px(spacing::lg())
            .py(spacing::sm())
            .bg(colors::bg_header())
            .border_1()
            .border_color(colors::border())
            .rounded(borders::radius_sm());

        if stats_loading {
            row = row
                .child(Spinner::new().small())
                .child(div().text_sm().text_color(colors::text_muted()).child("Loading stats..."));
            return section.child(row).into_any_element();
        }

        if let Some(_error) = stats_error {
            row = row
                .child(
                    div()
                        .text_sm()
                        .text_color(colors::text_error())
                        .child("Database stats failed. See banner for details."),
                )
                .child(
                    Button::new("retry-db-stats")
                        .ghost()
                        .compact()
                        .label("Retry")
                        .disabled(database_key.is_none())
                        .on_click({
                            let state = state.clone();
                            let key = database_key.clone();
                            move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                let Some(key) = key.clone() else {
                                    return;
                                };
                                AppCommands::load_database_overview(state.clone(), key, true, cx);
                            }
                        }),
                );
            return section.child(row).into_any_element();
        }

        let Some(stats) = stats else {
            row = row.child(
                div().text_sm().text_color(colors::text_muted()).child("No stats available"),
            );
            return section.child(row).into_any_element();
        };

        row = row
            .child(stat_cell("Collections", format_number(stats.collections)))
            .child(stat_cell("Objects", format_number(stats.objects)))
            .child(stat_cell("Avg size", format_bytes(stats.avg_obj_size)))
            .child(stat_cell("Data size", format_bytes(stats.data_size)))
            .child(stat_cell("Storage", format_bytes(stats.storage_size)))
            .child(stat_cell("Indexes", format_number(stats.indexes)))
            .child(stat_cell("Index size", format_bytes(stats.index_size)));

        section.child(row).into_any_element()
    }

    fn render_collections_section(
        collections: Vec<CollectionOverview>,
        collections_loading: bool,
        collections_error: Option<String>,
        database_name: String,
        state: Entity<AppState>,
    ) -> AnyElement {
        let mut section = div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .px(spacing::lg())
            .pt(spacing::lg())
            .pb(spacing::lg());

        section = section.child(
            div()
                .text_xs()
                .text_color(colors::text_muted())
                .child("Collections"),
        );

        if collections_loading {
            return section
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::sm())
                        .child(Spinner::new().small())
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors::text_muted())
                                .child("Loading collections..."),
                        ),
                )
                .into_any_element();
        }

        if let Some(_error) = collections_error {
            return section
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::sm())
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors::text_error())
                                .child("Collections failed. See banner for details."),
                        )
                        .child(
                            Button::new("retry-db-collections")
                                .ghost()
                                .compact()
                                .label("Retry")
                                .on_click({
                                    let state = state.clone();
                                    let database_name = database_name.clone();
                                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                        if let Some(active) = state.read(cx).conn.active.as_ref()
                                        {
                                            let key = crate::state::DatabaseKey::new(
                                                active.config.id,
                                                database_name.clone(),
                                            );
                                            AppCommands::load_database_overview(
                                                state.clone(),
                                                key,
                                                true,
                                                cx,
                                            );
                                        }
                                    }
                                }),
                        ),
                )
                .into_any_element();
        }

        if collections.is_empty() {
            return section
                .child(
                    div()
                        .text_sm()
                        .text_color(colors::text_muted())
                        .child("No collections yet. Use the sidebar menu to create one."),
                )
                .into_any_element();
        }

        let header_row = div()
            .flex()
            .items_center()
            .px(spacing::lg())
            .py(spacing::xs())
            .bg(colors::bg_header())
            .border_b_1()
            .border_color(colors::border())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_xs()
                    .text_color(colors::text_muted())
                    .child("Collection"),
            )
            .child(
                div()
                    .w(px(120.0))
                    .text_xs()
                    .text_color(colors::text_muted())
                    .child("Type"),
            )
            .child(
                div()
                    .w(px(100.0))
                    .text_xs()
                    .text_color(colors::text_muted())
                    .child("Capped"),
            )
            .child(
                div()
                    .w(px(120.0))
                    .text_xs()
                    .text_color(colors::text_muted())
                    .child("Read only"),
            );

        let rows = collections
            .into_iter()
            .enumerate()
            .map(|(index, overview)| {
                let database = database_name.clone();
                let collection_name = overview.name.clone();
                let state = state.clone();
                div()
                    .id(("db-collection-row", index))
                    .flex()
                    .items_center()
                    .px(spacing::lg())
                    .py(spacing::xs())
                    .border_b_1()
                    .border_color(colors::border_subtle())
                    .hover(|s| s.bg(colors::list_hover()))
                    .cursor_pointer()
                    .on_click(move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        state.update(cx, |state, cx| {
                            state.preview_collection(
                                database.clone(),
                                collection_name.clone(),
                                cx,
                            );
                        });
                    })
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .text_color(colors::text_primary())
                            .child(overview.name.clone()),
                    )
                    .child(
                        div()
                            .w(px(120.0))
                            .text_sm()
                            .text_color(colors::text_secondary())
                            .child(overview.collection_type.clone()),
                    )
                    .child(
                        div()
                            .w(px(100.0))
                            .text_sm()
                            .text_color(colors::text_secondary())
                            .child(if overview.capped { "Yes" } else { "No" }),
                    )
                    .child(
                        div()
                            .w(px(120.0))
                            .text_sm()
                            .text_color(colors::text_secondary())
                            .child(if overview.read_only { "Yes" } else { "No" }),
                    )
            })
            .collect::<Vec<_>>();

        section
            .child(header_row)
            .child(div().flex().flex_col().min_w(px(0.0)).children(rows))
            .into_any_element()
    }
}

fn stat_cell(label: &str, value: String) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(div().text_xs().text_color(colors::text_muted()).child(label.to_string()))
        .child(div().text_sm().text_color(colors::text_primary()).child(value))
        .into_any_element()
}
