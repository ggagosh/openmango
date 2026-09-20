use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::component::{Selectable as _, Sizable as _};
use gpui_kit::*;

use crate::components::{Button, ErrorCallout, request_preview_collection};
use crate::error::ErrorReport;
use crate::helpers::{format_bytes, format_number};
use crate::state::{
    AppCommands, AppEvent, AppState, CollectionOverview, DatabaseKey, DatabaseStats, View,
};
use crate::theme::{borders, sizing, spacing};

/// Database overview view (stats + collections list)
pub struct DatabaseView {
    state: Entity<AppState>,
    last_database_key: Option<DatabaseKey>,
    /// What the Relations section is showing: nothing, the diagram, or the review list.
    relations_pane: RelationsPane,
    /// The collection the diagram is centred on. Defaults to the most-referenced one, which is
    /// the one the question is usually about.
    diagram_focus: Option<String>,
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
                } else {
                    // Clear so re-entering the same database triggers a reload
                    this.last_database_key = None;
                }
                cx.notify();
            }
            _ => {}
        }));

        Self {
            state,
            last_database_key: current_key,
            relations_pane: RelationsPane::default(),
            diagram_focus: None,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for DatabaseView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_ref = self.state.read(cx);
        let database_name =
            state_ref.selected_database_name().unwrap_or_else(|| "Database".to_string());
        let database_key = state_ref.current_database_key();

        if database_key.is_none() {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
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
            (None, false, None, Vec::new(), false, None)
        };

        let state = self.state.clone();
        let refresh_button = Button::new("refresh-db")
            .ghost()
            .xsmall()
            .label("Refresh")
            .disabled(database_key.is_none())
            .on_click({
                let state = state.clone();
                let key = database_key.clone();
                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    let Some(key) = key.clone() else {
                        return;
                    };
                    AppCommands::reload_database(state.clone(), key, cx);
                }
            });
        let transfer_button = Button::new("open-transfer-db")
            .xsmall()
            .label("Transfer")
            .disabled(database_key.is_none())
            .on_click({
                let state = state.clone();
                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    state.update(cx, |state, cx| {
                        state.open_transfer_tab(cx);
                    });
                }
            });

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .h(sizing::header_height())
            .px(spacing::lg())
            .bg(cx.theme().tab_bar)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(cx.theme().foreground)
                    .child(database_name.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(transfer_button)
                    .child(refresh_button),
            );

        let content = div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .child(Self::render_stats_section(
                stats,
                stats_loading,
                stats_error,
                database_key.clone(),
                state.clone(),
                cx,
            ))
            .child(Self::render_relations_section(
                &database_name,
                self.relations_pane,
                self.diagram_focus.clone(),
                cx.entity(),
                state.clone(),
                cx,
            ))
            .child(Self::render_collections_section(
                collections,
                collections_loading,
                collections_error,
                database_name,
                database_key.clone(),
                state.clone(),
                cx,
            ));

        div()
            .key_context("Database")
            .flex()
            .flex_col()
            .flex_1()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(cx.theme().background)
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
        cx: &App,
    ) -> AnyElement {
        let mut section =
            div().flex().flex_col().gap(spacing::sm()).px(spacing::lg()).pt(spacing::lg());

        section = section
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Database stats"));

        let mut row = div()
            .flex()
            .items_center()
            .gap(spacing::lg())
            .px(spacing::lg())
            .py(spacing::sm())
            .bg(cx.theme().tab_bar)
            .border_1()
            .border_color(cx.theme().border)
            .rounded(borders::radius_sm());

        if stats_loading {
            row = row.child(Spinner::new().small()).child(
                div().text_sm().text_color(cx.theme().muted_foreground).child("Loading stats…"),
            );
            return section.child(row).into_any_element();
        }

        if let Some(error) = stats_error {
            let retry = Button::new("retry-db-stats")
                .xsmall()
                .label("Retry")
                .disabled(database_key.is_none())
                .on_click({
                    let state = state.clone();
                    let key = database_key.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(key) = key.clone() else {
                            return;
                        };
                        AppCommands::reload_database(state.clone(), key, cx);
                    }
                });
            return section
                .child(
                    ErrorCallout::new(
                        "db-stats-error",
                        ErrorReport::from_message("Couldn't load database stats", &error),
                    )
                    .action(retry)
                    .state(state.clone()),
                )
                .into_any_element();
        }

        let Some(stats) = stats else {
            row = row.child(
                div().text_sm().text_color(cx.theme().muted_foreground).child("No stats available"),
            );
            return section.child(row).into_any_element();
        };

        row = row
            .child(stat_cell("Collections", format_number(stats.collections), cx))
            .child(stat_cell("Objects", format_number(stats.objects), cx))
            .child(stat_cell("Avg size", format_bytes(stats.avg_obj_size), cx))
            .child(stat_cell("Data size", format_bytes(stats.data_size), cx))
            .child(stat_cell("Storage", format_bytes(stats.storage_size), cx))
            .child(stat_cell("Indexes", format_number(stats.indexes), cx))
            .child(stat_cell("Index size", format_bytes(stats.index_size), cx));

        section.child(row).into_any_element()
    }

    /// What this database's fields point at, and a way to find out.
    ///
    /// It lives here because inference is a database-wide read: it samples every collection and
    /// asks each one's neighbours, so the database is the scope that matches the work.
    #[allow(clippy::too_many_arguments)]
    fn render_relations_section(
        database_name: &str,
        pane: RelationsPane,
        focus: Option<String>,
        view: Entity<Self>,
        state: Entity<AppState>,
        cx: &App,
    ) -> AnyElement {
        let state_ref = state.read(cx);
        let known = state_ref.relation_count(database_name);
        // Another database's search still blocks this one, so say whose it is.
        let run = state_ref.inference_run().filter(|run| run.database == database_name).cloned();
        let last_run = state_ref
            .inference_summary()
            .filter(|summary| summary.database == database_name)
            .cloned();
        let last_line = last_run.as_ref().map(|summary| summary.line());
        // Worth copying only when there is something to read beyond the counts.
        // Only when something actually failed. Fields that matched nothing are explained by
        // the summary line itself, and a copy button for them was debugging scaffolding.
        let report = last_run
            .filter(|summary| !summary.failed_collections.is_empty())
            .map(|summary| summary.report());
        let busy_elsewhere = state_ref.inference_run().is_some() && run.is_none();

        let mut row = div()
            .flex()
            .items_center()
            .gap(spacing::lg())
            .px(spacing::lg())
            .py(spacing::sm())
            .bg(cx.theme().tab_bar)
            .border_1()
            .border_color(cx.theme().border)
            .rounded(borders::radius_sm());

        row = match &run {
            Some(run) => row
                .child(Spinner::new().small())
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(format!(
                            "Reading {} — {} of {} collections, {} relations found{}",
                            run.collection,
                            format_number(run.done as u64 + 1),
                            format_number(run.total as u64),
                            format_number(run.found as u64),
                            if run.failed > 0 {
                                format!(", {} could not be read", run.failed)
                            } else {
                                String::new()
                            },
                        )),
                )
                .child(Button::new("cancel-inference").ghost().xsmall().label("Stop").on_click({
                    let state = state.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        AppCommands::cancel_inference(&state, cx);
                    }
                })),
            None => row
                .child(stat_cell("Known relations", format_number(known as u64), cx))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        // What the last search could not do is worth more than what it did: a
                        // small number with no explanation is the thing that wastes time.
                        .child(if let Some(line) = last_line {
                            line
                        } else if known == 0 {
                            "Nothing is known yet. Inferring reads a sample of every collection \
                             and confirms each guess against the data."
                                .to_string()
                        } else {
                            "Cmd+click an ObjectId to follow it, or an _id to see what points \
                             at it."
                                .to_string()
                        }),
                )
                .child(pane_button(
                    "Diagram",
                    "show-diagram",
                    RelationsPane::Diagram,
                    pane,
                    known,
                    view.clone(),
                ))
                .children(report.map(|report| {
                    Button::new("copy-inference-report")
                        .ghost()
                        .xsmall()
                        .label("Copy details")
                        .tooltip("Copy the fields this search could not place")
                        .on_click(move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                            cx.write_to_clipboard(ClipboardItem::new_string(report.clone()));
                        })
                }))
                .child(
                    Button::new("infer-relations-db")
                        .xsmall()
                        .label(if known == 0 { "Infer relations" } else { "Infer again" })
                        .disabled(busy_elsewhere)
                        .on_click({
                            let state = state.clone();
                            let database = database_name.to_string();
                            move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                AppCommands::infer_relations_for_database(
                                    state.clone(),
                                    database.clone(),
                                    cx,
                                );
                            }
                        }),
                ),
        };

        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .px(spacing::lg())
            .pt(spacing::lg())
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Relations"))
            .child(row)
            .children(match pane {
                RelationsPane::Closed => None,
                RelationsPane::Diagram => {
                    // Centred on whatever is most referenced until something else is chosen.
                    let focus = focus.or_else(|| {
                        state
                            .read(cx)
                            .relations()
                            .by_target(database_name)
                            .first()
                            .map(|(target, _)| target.clone())
                    });
                    focus.map(|focus| {
                        render_relation_diagram(database_name, &focus, view, state, cx)
                    })
                }
            })
            .into_any_element()
    }
    fn render_collections_section(
        collections: Vec<CollectionOverview>,
        collections_loading: bool,
        collections_error: Option<String>,
        database_name: String,
        database_key: Option<crate::state::DatabaseKey>,
        state: Entity<AppState>,
        cx: &App,
    ) -> AnyElement {
        let mut section = div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .gap(spacing::sm())
            // No side padding here: the table below is the scroll owner and has to reach the
            // panel edges, or its scrollbar and the header's rule float inside the panel. The
            // caption and the state messages carry the inset themselves.
            .pt(spacing::lg())
            .pb(spacing::lg());

        section = section.child(
            div()
                .px(spacing::lg())
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Collections"),
        );

        if collections_loading {
            return section
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::sm())
                        .px(spacing::lg())
                        .child(Spinner::new().small())
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Loading collections…"),
                        ),
                )
                .into_any_element();
        }

        if let Some(error) = collections_error {
            let retry = Button::new("retry-db-collections").xsmall().label("Retry").on_click({
                let state = state.clone();
                let database_key = database_key.clone();
                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    let Some(key) = database_key.clone() else {
                        return;
                    };
                    AppCommands::reload_database(state.clone(), key, cx);
                }
            });
            return section
                .child(
                    div().px(spacing::lg()).child(
                        ErrorCallout::new(
                            "db-collections-error",
                            ErrorReport::from_message("Couldn't load collections", &error),
                        )
                        .action(retry)
                        .state(state.clone()),
                    ),
                )
                .into_any_element();
        }

        if collections.is_empty() {
            return section
                .child(
                    div()
                        .px(spacing::lg())
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No collections yet. Use the sidebar menu to create one."),
                )
                .into_any_element();
        }

        let header_row = div()
            .flex()
            .items_center()
            .px(spacing::lg())
            .py(spacing::xs())
            .bg(cx.theme().tab_bar)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Collection"),
            )
            .child(
                div().w(px(120.0)).text_xs().text_color(cx.theme().muted_foreground).child("Type"),
            )
            .child(
                div()
                    .w(px(100.0))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Capped"),
            )
            .child(
                div()
                    .w(px(120.0))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Read only"),
            );

        let theme_border_subtle = cx.theme().sidebar_border;
        let theme_list_hover = cx.theme().list_hover;
        let theme_text_primary = cx.theme().foreground;
        let theme_text_secondary = cx.theme().secondary_foreground;

        let connection_id = database_key.map(|key| key.connection_id);
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
                    .border_color(theme_border_subtle)
                    .hover(move |s| s.bg(theme_list_hover))
                    .cursor_pointer()
                    .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let Some(connection_id) = connection_id else {
                            return;
                        };
                        request_preview_collection(
                            state.clone(),
                            connection_id,
                            database.clone(),
                            collection_name.clone(),
                            window,
                            cx,
                        );
                    })
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .text_color(theme_text_primary)
                            .child(overview.name.clone()),
                    )
                    .child(
                        div()
                            .w(px(120.0))
                            .text_sm()
                            .text_color(theme_text_secondary)
                            .child(overview.collection_type.clone()),
                    )
                    .child(
                        div()
                            .w(px(100.0))
                            .text_sm()
                            .text_color(theme_text_secondary)
                            .child(if overview.capped { "Yes" } else { "No" }),
                    )
                    .child(
                        div()
                            .w(px(120.0))
                            .text_sm()
                            .text_color(theme_text_secondary)
                            .child(if overview.read_only { "Yes" } else { "No" }),
                    )
            })
            .collect::<Vec<_>>();

        section
            .child(header_row)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .children(rows),
            )
            .into_any_element()
    }
}

fn stat_cell(label: &str, value: String, cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label.to_string()))
        .child(div().text_sm().text_color(cx.theme().foreground).child(value))
        .into_any_element()
}
/// What the Relations section is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RelationsPane {
    #[default]
    Closed,
    /// One collection and its neighbours, drawn.
    Diagram,
}

/// One collection, what points at it, and what it points at.
///
/// Not the whole database: fifty-eight collections and a hundred edges drawn at once is a
/// hairball whichever way it is laid out, and the question people actually arrive with is about
/// one collection. Clicking a neighbour moves the focus, so the graph is walked rather than
/// surveyed.
///
/// Three columns and elbow connectors, built from ordinary elements. A path-drawn diagram would
/// need a layout engine and measured positions to say the same thing.
fn render_relation_diagram(
    database: &str,
    focus: &str,
    view: Entity<DatabaseView>,
    state: Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let graph = state.read(cx).relations();
    let incoming = graph.into_collection(database, focus);
    let outgoing = graph.from_collection(database, focus);

    if incoming.is_empty() && outgoing.is_empty() {
        return div()
            .flex()
            .items_center()
            .justify_center()
            .py(spacing::lg())
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(format!("Nothing is known to point at or from {focus}."))
            .into_any_element();
    }

    div()
        .id("relation-diagram")
        // Its own scroll, bounded: the tab below it clips rather than scrolls, so a diagram
        // that grew with the graph pushed the collections list off the page.
        .max_h(px(420.0))
        .overflow_y_scrollbar()
        .flex()
        .items_stretch()
        .gap(spacing::sm())
        .pt(spacing::sm())
        .child(
            // What points here, on the left, pointing right.
            div().flex().flex_col().gap(px(2.0)).flex_1().min_w(px(0.0)).items_end().children(
                incoming.iter().map(|relation| {
                    neighbour(
                        &relation.source.collection,
                        &format!("{}.{}", relation.source.collection, relation.source.path),
                        true,
                        view.clone(),
                        cx,
                    )
                }),
            ),
        )
        .child(
            // The collection in focus, between the two.
            div().flex().flex_col().justify_center().child(
                div()
                    .px(spacing::md())
                    .py(spacing::sm())
                    .rounded(borders::radius_md())
                    .border_1()
                    .border_color(cx.theme().primary)
                    .bg(cx.theme().secondary)
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::theme::fonts::mono())
                            .font_weight(FontWeight::MEDIUM)
                            .child(focus.to_string()),
                    )
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!(
                        "{} in · {} out",
                        incoming.len(),
                        outgoing.len()
                    ))),
            ),
        )
        .child(
            // What it points at, on the right.
            div().flex().flex_col().gap(px(2.0)).flex_1().min_w(px(0.0)).items_start().children(
                outgoing.iter().map(|relation| {
                    neighbour(
                        &relation.target.collection,
                        &format!("{}.{}", focus, relation.source.path),
                        false,
                        view.clone(),
                        cx,
                    )
                }),
            ),
        )
        .into_any_element()
}

/// One neighbour and the field that joins it, with an elbow pointing at the focus.
fn neighbour(
    collection: &str,
    field: &str,
    incoming: bool,
    view: Entity<DatabaseView>,
    cx: &App,
) -> AnyElement {
    let name = collection.to_string();
    let box_el = div()
        .id(SharedString::from(format!("node:{field}")))
        .px(spacing::sm())
        .py(px(3.0))
        .min_w(px(0.0))
        .rounded(borders::radius_sm())
        .border_1()
        .border_color(cx.theme().border)
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().list_hover))
        .on_click(move |_, _window, cx| {
            view.update(cx, |view, cx| {
                view.diagram_focus = Some(name.clone());
                cx.notify();
            });
        })
        .child(
            div()
                .text_xs()
                .font_family(crate::theme::fonts::mono())
                .truncate()
                .child(collection.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .truncate()
                .child(field.to_string()),
        );

    // The elbow: a rule out of the box and an arrow into the focus.
    let connector = div()
        .flex()
        .items_center()
        .flex_none()
        .child(div().w(px(18.0)).h(px(1.0)).bg(cx.theme().border))
        .child(Icon::new(IconName::ArrowRight).xsmall().text_color(cx.theme().muted_foreground));

    let row = div().flex().items_center().gap(px(2.0)).max_w(px(340.0));
    if incoming {
        row.child(box_el).child(connector).into_any_element()
    } else {
        row.child(connector).child(box_el).into_any_element()
    }
}

/// One of the Relations section's views. Pressing the open one closes it, so the section
/// collapses back to its summary.
fn pane_button(
    label: &'static str,
    id: &'static str,
    pane: RelationsPane,
    current: RelationsPane,
    known: usize,
    view: Entity<DatabaseView>,
) -> impl IntoElement {
    Button::new(id)
        .ghost()
        .xsmall()
        .label(label)
        .selected(current == pane)
        .disabled(known == 0)
        .on_click(move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
            view.update(cx, |view, cx| {
                view.relations_pane =
                    if view.relations_pane == pane { RelationsPane::Closed } else { pane };
                cx.notify();
            });
        })
}
