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
            .title("Create Database")
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
