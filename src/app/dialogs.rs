use gpui_kit::component::Size;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::input::InputState;
use gpui_kit::*;

use crate::components::ErrorCallout;
use crate::components::{
    Button, ConnectionIdentity, FormField, busy_label, cancel_button, connection_identity_badge,
    request_connection_write,
};
use crate::error::ErrorReport;
use crate::state::{AppCommands, AppState};
use crate::theme::spacing;

/// A dialog that runs a write: it stays open until the write succeeds and shows why it didn't.
#[derive(Default)]
struct DialogRun {
    busy: bool,
    error: Option<ErrorReport>,
}

fn begin(run: &Entity<DialogRun>, cx: &mut App) {
    run.update(cx, |run, cx| {
        run.busy = true;
        run.error = None;
        cx.notify();
    });
}

/// Holds the run weakly: if the dialog was closed meanwhile, its state is gone and nothing
/// else (such as a dialog opened later) gets closed.
fn finish(
    run: WeakEntity<DialogRun>,
    window: AnyWindowHandle,
) -> impl FnOnce(Result<(), ErrorReport>, &mut App) + 'static {
    move |result, cx| {
        let Some(run) = run.upgrade() else {
            return;
        };
        match result {
            Ok(()) => {
                let _ = window.update(cx, |_, window, cx| window.close_dialog(cx));
            }
            Err(report) => run.update(cx, |run, cx| {
                run.busy = false;
                run.error = Some(report);
                cx.notify();
            }),
        }
    }
}

fn run_error(
    id: &'static str,
    run: &Entity<DialogRun>,
    state: &Entity<AppState>,
    cx: &App,
) -> Option<ErrorCallout> {
    run.read(cx).error.clone().map(|report| ErrorCallout::new(id, report).state(state.clone()))
}

fn selected_connection_identity(state: &Entity<AppState>, cx: &App) -> AnyElement {
    state
        .read(cx)
        .selected_connection_id()
        .and_then(|id| state.read(cx).connection_by_id(id))
        .map(ConnectionIdentity::from)
        .map(|identity| connection_identity_badge(&identity, true, cx))
        .unwrap_or_else(|| div().into_any_element())
}

pub(crate) fn open_create_database_dialog(
    state: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) {
    let db_state =
        cx.new(|cx| InputState::new(window, cx).placeholder("database_name").default_value(""));
    let col_state = cx.new(|cx| {
        InputState::new(window, cx).placeholder("collection_name").default_value("default")
    });

    let db_state_save = db_state.clone();
    let col_state_save = col_state.clone();
    let run = cx.new(|_| DialogRun::default());
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, cx: &mut App| {
        let busy = run.read(cx).busy;
        dialog
            .title("Create database")
            .min_w(px(420.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .p(spacing::md())
                    .child(selected_connection_identity(&state, cx))
                    .child(FormField::new("Database name", &db_state).render(cx))
                    .child(FormField::new("Initial collection", &col_state).render(cx))
                    .children(run_error("create-db-error", &run, &state, cx)),
            )
            .footer({
                let state = state.clone();
                let db_state = db_state_save.clone();
                let col_state = col_state_save.clone();

                let state = state.clone();
                let db_state = db_state.clone();
                let col_state = col_state.clone();
                let run = run.clone();
                gpui_kit::component::dialog::DialogFooter::new().children(vec![
                    cancel_button("cancel-db"),
                    busy_label(Button::new("create-db").primary(), Size::Medium, "Create", busy)
                        .on_click(move |_, window, cx| {
                            if run.read(cx).busy {
                                return;
                            }
                            let db = db_state.read(cx).value().to_string();
                            let col = col_state.read(cx).value().to_string();
                            if db.trim().is_empty() || col.trim().is_empty() {
                                return;
                            }
                            let Some(connection_id) = state.read(cx).selected_connection_id()
                            else {
                                return;
                            };
                            let db = db.trim().to_string();
                            let col = col.trim().to_string();
                            let state_for_write = state.clone();
                            let run = run.clone();
                            request_connection_write(
                                state.clone(),
                                crate::components::WriteRequest::new(
                                    connection_id,
                                    format!("{db}.{col}"),
                                    "Create a database and its initial collection",
                                    None,
                                ),
                                window,
                                cx,
                                move |window, cx| {
                                    begin(&run, cx);
                                    let done = finish(run.downgrade(), window.window_handle());
                                    AppCommands::create_database(
                                        state_for_write,
                                        db,
                                        col,
                                        cx,
                                        done,
                                    );
                                },
                            );
                        })
                        .into_any_element(),
                ])
            })
    });
}

pub(crate) fn open_create_collection_dialog(
    state: Entity<AppState>,
    database: String,
    window: &mut Window,
    cx: &mut App,
) {
    let col_state =
        cx.new(|cx| InputState::new(window, cx).placeholder("collection_name").default_value(""));
    let col_state_save = col_state.clone();
    let run = cx.new(|_| DialogRun::default());
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, cx: &mut App| {
        let busy = run.read(cx).busy;
        dialog
            .title(format!("Create Collection in {database}"))
            .min_w(px(420.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .p(spacing::md())
                    .child(selected_connection_identity(&state, cx))
                    .child(FormField::new("Collection name", &col_state).render(cx))
                    .children(run_error("create-collection-error", &run, &state, cx)),
            )
            .footer({
                let state = state.clone();
                let database = database.clone();
                let col_state = col_state_save.clone();

                let state = state.clone();
                let database = database.clone();
                let col_state = col_state.clone();
                let run = run.clone();
                gpui_kit::component::dialog::DialogFooter::new().children(vec![
                    cancel_button("cancel-collection"),
                    busy_label(
                        Button::new("create-collection").primary(),
                        Size::Medium,
                        "Create",
                        busy,
                    )
                    .on_click(move |_, window, cx| {
                        if run.read(cx).busy {
                            return;
                        }
                        let col = col_state.read(cx).value().to_string();
                        if col.trim().is_empty() {
                            return;
                        }
                        let Some(connection_id) = state.read(cx).selected_connection_id() else {
                            return;
                        };
                        let collection = col.trim().to_string();
                        let state_for_write = state.clone();
                        let database_for_write = database.clone();
                        let run = run.clone();
                        let target = format!("{database}.{collection}");
                        request_connection_write(
                            state.clone(),
                            crate::components::WriteRequest::new(
                                connection_id,
                                target,
                                "Create a collection",
                                None,
                            ),
                            window,
                            cx,
                            move |window, cx| {
                                begin(&run, cx);
                                let done = finish(run.downgrade(), window.window_handle());
                                AppCommands::create_collection(
                                    state_for_write,
                                    database_for_write,
                                    collection,
                                    cx,
                                    done,
                                );
                            },
                        );
                    })
                    .into_any_element(),
                ])
            })
    });
}

pub(crate) fn open_rename_collection_dialog(
    state: Entity<AppState>,
    database: String,
    collection: String,
    window: &mut Window,
    cx: &mut App,
) {
    let name_state = cx.new(|cx| {
        InputState::new(window, cx).placeholder("collection_name").default_value(collection.clone())
    });
    let name_state_save = name_state.clone();
    let run = cx.new(|_| DialogRun::default());
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, cx: &mut App| {
        let busy = run.read(cx).busy;
        dialog
            .title(format!("Rename Collection {database}.{collection}"))
            .min_w(px(420.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .p(spacing::md())
                    .child(selected_connection_identity(&state, cx))
                    .child(FormField::new("New collection name", &name_state).render(cx))
                    .children(run_error("rename-collection-error", &run, &state, cx)),
            )
            .footer({
                let state = state.clone();
                let database = database.clone();
                let collection = collection.clone();
                let name_state = name_state_save.clone();

                let state = state.clone();
                let database = database.clone();
                let collection = collection.clone();
                let name_state = name_state.clone();
                let run = run.clone();
                gpui_kit::component::dialog::DialogFooter::new().children(vec![
                    cancel_button("cancel-rename-collection"),
                    busy_label(
                        Button::new("rename-collection").primary(),
                        Size::Medium,
                        "Rename",
                        busy,
                    )
                    .on_click(move |_, window, cx| {
                        if run.read(cx).busy {
                            return;
                        }
                        let new_name = name_state.read(cx).value().to_string();
                        let new_name = new_name.trim();
                        if new_name.is_empty() || new_name == collection.as_str() {
                            return;
                        }
                        let Some(connection_id) = state.read(cx).selected_connection_id() else {
                            return;
                        };
                        let new_name = new_name.to_string();
                        let state_for_write = state.clone();
                        let database_for_write = database.clone();
                        let collection_for_write = collection.clone();
                        let run = run.clone();
                        let target = format!("{database}.{collection} → {database}.{new_name}");
                        request_connection_write(
                            state.clone(),
                            crate::components::WriteRequest::new(
                                connection_id,
                                target,
                                "Rename a collection",
                                None,
                            ),
                            window,
                            cx,
                            move |window, cx| {
                                begin(&run, cx);
                                let done = finish(run.downgrade(), window.window_handle());
                                AppCommands::rename_collection(
                                    state_for_write,
                                    database_for_write,
                                    collection_for_write,
                                    new_name,
                                    cx,
                                    done,
                                );
                            },
                        );
                    })
                    .into_any_element(),
                ])
            })
    });
}

/// What a new view is made from, as far as the dialog needs to know.
pub(crate) enum NewView {
    /// The aggregation screen's pipeline over `view_on`. Offers a collation.
    Pipeline { view_on: String, pipeline: Vec<mongodb::bson::Document> },
    /// A copy of an existing view, which brings its own collation along.
    CopyOf(String),
}

/// Asks for the name of a new view, and for a collation when the view is new rather than a
/// copy. A collation can only be given at creation, so this is the one place to ask.
pub(crate) fn open_new_view_dialog(
    state: Entity<AppState>,
    connection_id: uuid::Uuid,
    database: String,
    new_view: NewView,
    window: &mut Window,
    cx: &mut App,
) {
    // Only an offer: it says what the pipeline does, in the source's naming style, and the
    // person types over it if they had something else in mind.
    let taken = state
        .read(cx)
        .active_connection_by_id(connection_id)
        .and_then(|conn| conn.collections.get(&database).cloned())
        .unwrap_or_default();
    let (title, summary, suggested) = match &new_view {
        NewView::Pipeline { view_on, pipeline } => (
            format!("Save as View in {database}"),
            format!(
                "Reads {view_on} through {}. A view is read-only and stores no data of its own.",
                match pipeline.len() {
                    0 => "no stages".to_string(),
                    1 => "1 stage".to_string(),
                    count => format!("{count} stages"),
                }
            ),
            crate::helpers::view_name::suggest_view_name(view_on, pipeline, &taken),
        ),
        NewView::CopyOf(view) => (
            format!("Duplicate View {database}.{view}"),
            "The copy gets the same source, pipeline and collation.".to_string(),
            crate::helpers::view_name::suggest_copy_name(view, &taken),
        ),
    };
    let offers_collation = matches!(new_view, NewView::Pipeline { .. });
    let new_view = std::rc::Rc::new(new_view);
    let name_state =
        cx.new(|cx| InputState::new(window, cx).placeholder("view_name").default_value(suggested));
    let collation_state = cx.new(|cx| {
        InputState::new(window, cx).placeholder("{ locale: \"en\", strength: 2 }").default_value("")
    });
    let run = cx.new(|_| DialogRun::default());
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, cx: &mut App| {
        let busy = run.read(cx).busy;
        let mut fields = div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .p(spacing::md())
            .child(selected_connection_identity(&state, cx))
            .child(FormField::new("View name", &name_state).render(cx));
        if offers_collation {
            fields = fields
                .child(FormField::new("Collation (optional)", &collation_state).render(cx));
        }
        dialog
            .title(title.clone())
            .min_w(px(460.0))
            .child(
                fields
                    .child(
                        div()
                            .text_sm()
                            .text_color(
                                gpui_kit::component::ActiveTheme::theme(cx).muted_foreground,
                            )
                            .child(summary.clone()),
                    )
                    .children(run_error("new-view-error", &run, &state, cx)),
            )
            .footer({
                let state = state.clone();
                let database = database.clone();
                let name_state = name_state.clone();
                let collation_state = collation_state.clone();
                let new_view = new_view.clone();
                let run = run.clone();
                gpui_kit::component::dialog::DialogFooter::new().children(vec![
                    cancel_button("cancel-new-view"),
                    busy_label(Button::new("create-view").primary(), Size::Medium, "Create view", busy)
                        .on_click(move |_, window, cx| {
                            if run.read(cx).busy {
                                return;
                            }
                            let name = name_state.read(cx).value().trim().to_string();
                            if name.is_empty() {
                                return;
                            }
                            let collation_text = collation_state.read(cx).value().trim().to_string();
                            let collation = if collation_text.is_empty() {
                                None
                            } else {
                                match crate::bson::parse_bson_from_relaxed_json(&collation_text) {
                                    Ok(mongodb::bson::Bson::Document(collation)) => Some(collation),
                                    _ => {
                                        run.update(cx, |run, cx| {
                                            run.error = Some(ErrorReport::new(
                                                "The collation isn't a document",
                                                "Write it like { locale: \"en\", strength: 2 }, or leave it empty.",
                                            ));
                                            cx.notify();
                                        });
                                        return;
                                    }
                                }
                            };
                            let source = match new_view.as_ref() {
                                NewView::Pipeline { view_on, pipeline } => {
                                    crate::state::ViewSource::Pipeline {
                                        view_on: view_on.clone(),
                                        pipeline: pipeline.clone(),
                                        collation,
                                    }
                                }
                                NewView::CopyOf(view) => {
                                    crate::state::ViewSource::CopyOf(view.clone())
                                }
                            };
                            let state_for_write = state.clone();
                            let database_for_write = database.clone();
                            let run = run.clone();
                            let target = format!("{database}.{name}");
                            request_connection_write(
                                state.clone(),
                                crate::components::WriteRequest::new(
                                    connection_id,
                                    target,
                                    "Create a view",
                                    None,
                                ),
                                window,
                                cx,
                                move |window, cx| {
                                    begin(&run, cx);
                                    let done = finish(run.downgrade(), window.window_handle());
                                    AppCommands::save_view(
                                        state_for_write,
                                        crate::state::ViewSave {
                                            connection_id,
                                            database: database_for_write,
                                            name,
                                            source,
                                            replace: false,
                                        },
                                        cx,
                                        done,
                                    );
                                },
                            );
                        })
                        .into_any_element(),
                ])
            })
    });
}

/// Saves the tab as a new task. The tab is then linked to it, so its next save updates it.
pub(crate) fn open_save_task_dialog(
    state: Entity<AppState>,
    tab: crate::state::TabKey,
    window: &mut Window,
    cx: &mut App,
) {
    let spec = match &tab {
        crate::state::TabKey::Transfer(key) => state.read(cx).transfer_task_spec(key.id),
        crate::state::TabKey::Compare(key) => state.read(cx).compare_task_spec(key.id),
        _ => None,
    };
    let Some(spec) = spec else {
        return;
    };
    let kind = spec.kind().label();
    let name_state = cx.new(|cx| {
        InputState::new(window, cx).placeholder("Task name").default_value(spec.default_name())
    });
    let run = cx.new(|_| DialogRun::default());
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, cx: &mut App| {
        let save = {
            let (state, tab, name_state, run) =
                (state.clone(), tab.clone(), name_state.clone(), run.clone());
            move |window: &mut Window, cx: &mut App| {
                let name = name_state.read(cx).value().trim().to_string();
                if name.is_empty() {
                    return;
                }
                // Read the tab again: it may have changed while the dialog was open.
                let spec = match &tab {
                    crate::state::TabKey::Transfer(key) => {
                        state.read(cx).transfer_task_spec(key.id)
                    }
                    crate::state::TabKey::Compare(key) => state.read(cx).compare_task_spec(key.id),
                    _ => None,
                };
                let Some(spec) = spec else {
                    return;
                };
                let task = crate::tasks::model::Task::new(name, spec);
                let id = task.id;
                match AppCommands::save_task(&state, task, cx) {
                    Ok(()) => {
                        state.update(cx, |app, cx| {
                            app.link_tab_to_task(&tab, id);
                            cx.notify();
                        });
                        window.close_dialog(cx);
                    }
                    Err(error) => run.update(cx, |run, cx| {
                        run.error = Some(ErrorReport::new("Couldn't save the task", error));
                        cx.notify();
                    }),
                }
            }
        };
        let save_on_enter = save.clone();
        dialog
            .title(format!("Save {} as a task", kind.to_lowercase()))
            .min_w(px(420.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .p(spacing::md())
                    .child(
                        div()
                            .on_action(move |_: &gpui_kit::component::input::Enter, window, cx| {
                                save_on_enter(window, cx)
                            })
                            .child(FormField::new("Name", &name_state).render(cx)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(
                                gpui_kit::component::ActiveTheme::theme(cx).muted_foreground,
                            )
                            .child("Run it again from Tasks, with the settings this tab has now."),
                    )
                    .children(run_error("save-task-error", &run, &state, cx)),
            )
            .footer(gpui_kit::component::dialog::DialogFooter::new().children(vec![
                cancel_button("cancel-save-task"),
                Button::new("save-task")
                    .primary()
                    .label("Save task")
                    .on_click(move |_, window, cx| save(window, cx))
                    .into_any_element(),
            ]))
    });
}
