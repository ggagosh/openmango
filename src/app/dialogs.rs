use gpui::*;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::InputState;

use crate::components::{FormField, cancel_button, primary_button};
use crate::state::{AppCommands, AppState};
use crate::theme::spacing;

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
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
        dialog
            .title("Create Database")
            .min_w(px(420.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .p(spacing::md())
                    .child(FormField::new("Database name", &db_state))
                    .child(FormField::new("Initial collection", &col_state)),
            )
            .footer({
                let state = state.clone();
                let db_state = db_state_save.clone();
                let col_state = col_state_save.clone();
                move |_ok_fn, _cancel_fn, _window: &mut Window, _cx: &mut App| {
                    let state = state.clone();
                    let db_state = db_state.clone();
                    let col_state = col_state.clone();
                    vec![
                        cancel_button("cancel-db"),
                        primary_button("create-db", "Create", move |window, cx| {
                            let db = db_state.read(cx).value().to_string();
                            let col = col_state.read(cx).value().to_string();
                            if db.trim().is_empty() || col.trim().is_empty() {
                                return;
                            }
                            AppCommands::create_database(
                                state.clone(),
                                db.trim().to_string(),
                                col.trim().to_string(),
                                cx,
                            );
                            window.close_dialog(cx);
                        }),
                    ]
                }
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
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
        dialog
            .title(format!("Create Collection in {database}"))
            .min_w(px(420.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .p(spacing::md())
                    .child(FormField::new("Collection name", &col_state)),
            )
            .footer({
                let state = state.clone();
                let database = database.clone();
                let col_state = col_state_save.clone();
                move |_ok_fn, _cancel_fn, _window: &mut Window, _cx: &mut App| {
                    let state = state.clone();
                    let database = database.clone();
                    let col_state = col_state.clone();
                    vec![
                        cancel_button("cancel-collection"),
                        primary_button("create-collection", "Create", move |window, cx| {
                            let col = col_state.read(cx).value().to_string();
                            if col.trim().is_empty() {
                                return;
                            }
                            AppCommands::create_collection(
                                state.clone(),
                                database.clone(),
                                col.trim().to_string(),
                                cx,
                            );
                            window.close_dialog(cx);
                        }),
                    ]
                }
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
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
        dialog
            .title(format!("Rename Collection {database}.{collection}"))
            .min_w(px(420.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .p(spacing::md())
                    .child(FormField::new("New collection name", &name_state)),
            )
            .footer({
                let state = state.clone();
                let database = database.clone();
                let collection = collection.clone();
                let name_state = name_state_save.clone();
                move |_ok_fn, _cancel_fn, _window: &mut Window, _cx: &mut App| {
                    let state = state.clone();
                    let database = database.clone();
                    let collection = collection.clone();
                    let name_state = name_state.clone();
                    vec![
                        cancel_button("cancel-rename-collection"),
                        primary_button("rename-collection", "Rename", move |window, cx| {
                            let new_name = name_state.read(cx).value().to_string();
                            let new_name = new_name.trim();
                            if new_name.is_empty() || new_name == collection.as_str() {
                                return;
                            }
                            AppCommands::rename_collection(
                                state.clone(),
                                database.clone(),
                                collection.clone(),
                                new_name.to_string(),
                                cx,
                            );
                            window.close_dialog(cx);
                        }),
                    ]
                }
            })
    });
}
