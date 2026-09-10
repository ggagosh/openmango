use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::input::InputState;
use gpui_kit::*;

use crate::components::{
    ConnectionIdentity, FormField, cancel_button, connection_identity_badge, primary_button,
    request_connection_write,
};
use crate::state::{AppCommands, AppState};
use crate::theme::spacing;

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
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, cx: &mut App| {
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
                    .child(FormField::new("Initial collection", &col_state).render(cx)),
            )
            .footer({
                let state = state.clone();
                let db_state = db_state_save.clone();
                let col_state = col_state_save.clone();

                let state = state.clone();
                let db_state = db_state.clone();
                let col_state = col_state.clone();
                gpui_kit::component::dialog::DialogFooter::new().children(vec![
                    cancel_button("cancel-db"),
                    primary_button("create-db", "Create", move |window, cx| {
                        let db = db_state.read(cx).value().to_string();
                        let col = col_state.read(cx).value().to_string();
                        if db.trim().is_empty() || col.trim().is_empty() {
                            return;
                        }
                        let Some(connection_id) = state.read(cx).selected_connection_id() else {
                            return;
                        };
                        let db = db.trim().to_string();
                        let col = col.trim().to_string();
                        let state_for_write = state.clone();
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
                                window.close_dialog(cx);
                                AppCommands::create_database(state_for_write, db, col, cx);
                            },
                        );
                    }),
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
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, cx: &mut App| {
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
                    .child(FormField::new("Collection name", &col_state).render(cx)),
            )
            .footer({
                let state = state.clone();
                let database = database.clone();
                let col_state = col_state_save.clone();

                let state = state.clone();
                let database = database.clone();
                let col_state = col_state.clone();
                gpui_kit::component::dialog::DialogFooter::new().children(vec![
                    cancel_button("cancel-collection"),
                    primary_button("create-collection", "Create", move |window, cx| {
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
                                window.close_dialog(cx);
                                AppCommands::create_collection(
                                    state_for_write,
                                    database_for_write,
                                    collection,
                                    cx,
                                );
                            },
                        );
                    }),
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
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, cx: &mut App| {
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
                    .child(FormField::new("New collection name", &name_state).render(cx)),
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
                gpui_kit::component::dialog::DialogFooter::new().children(vec![
                    cancel_button("cancel-rename-collection"),
                    primary_button("rename-collection", "Rename", move |window, cx| {
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
                                window.close_dialog(cx);
                                AppCommands::rename_collection(
                                    state_for_write,
                                    database_for_write,
                                    collection_for_write,
                                    new_name,
                                    cx,
                                );
                            },
                        );
                    }),
                ])
            })
    });
}
