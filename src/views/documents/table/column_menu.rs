use std::collections::HashSet;

use gpui::Entity;
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use gpui_component::{Icon, IconName};

use crate::state::{AppCommands, AppState, SessionKey};

use super::column_schema::TableColumnDef;

pub enum ColumnMenuKind {
    Document,
    Aggregation,
}

#[allow(clippy::too_many_arguments)]
pub fn build_table_column_menu(
    mut menu: PopupMenu,
    selected_col: Option<usize>,
    columns: &[TableColumnDef],
    pinned_columns: &HashSet<String>,
    kind: ColumnMenuKind,
    state: &Entity<AppState>,
    session_key: &SessionKey,
    _cx: &gpui::App,
) -> PopupMenu {
    let col_key = selected_col.and_then(|c| columns.get(c).map(|col| col.key.clone()));

    let Some(col_key) = col_key else {
        return menu;
    };

    let is_pinned = pinned_columns.contains(&col_key);
    let pin_label = if is_pinned { "Unpin Column" } else { "Pin Column" };
    let pin_icon = if is_pinned { IconName::PinOff } else { IconName::Pin };

    menu = menu.separator();

    match kind {
        ColumnMenuKind::Document => {
            menu = menu.item(
                PopupMenuItem::new("Sort Ascending").icon(Icon::new(IconName::ArrowUp)).on_click({
                    let state = state.clone();
                    let sk = session_key.clone();
                    let key = col_key.clone();
                    move |_, _window, cx| {
                        let raw = format!("{{\"{}\": 1}}", key);
                        let doc = mongodb::bson::doc! { &key: 1 };
                        let state = state.clone();
                        let sk = sk.clone();
                        state.update(cx, |s, cx| {
                            let proj_raw = s
                                .session_data(&sk)
                                .map(|d| d.projection_raw.clone())
                                .unwrap_or_default();
                            let proj = s.session_data(&sk).and_then(|d| d.projection.clone());
                            s.set_sort_projection(&sk, raw, Some(doc), proj_raw, proj);
                            cx.notify();
                        });
                        AppCommands::load_documents_for_session(state, sk, cx);
                    }
                }),
            );
            menu = menu.item(
                PopupMenuItem::new("Sort Descending")
                    .icon(Icon::new(IconName::ArrowDown))
                    .on_click({
                        let state = state.clone();
                        let sk = session_key.clone();
                        let key = col_key.clone();
                        move |_, _window, cx| {
                            let raw = format!("{{\"{}\": -1}}", key);
                            let doc = mongodb::bson::doc! { &key: -1 };
                            let state = state.clone();
                            let sk = sk.clone();
                            state.update(cx, |s, cx| {
                                let proj_raw = s
                                    .session_data(&sk)
                                    .map(|d| d.projection_raw.clone())
                                    .unwrap_or_default();
                                let proj = s.session_data(&sk).and_then(|d| d.projection.clone());
                                s.set_sort_projection(&sk, raw, Some(doc), proj_raw, proj);
                                cx.notify();
                            });
                            AppCommands::load_documents_for_session(state, sk, cx);
                        }
                    }),
            );
            menu = menu.separator();
        }
        ColumnMenuKind::Aggregation => {
            // Aggregation sort is handled via the table header (perform_sort).
            // No server-side sort from context menu.
        }
    }

    menu = menu.item(PopupMenuItem::new(pin_label).icon(Icon::new(pin_icon)).on_click({
        let state = state.clone();
        let sk = session_key.clone();
        let key = col_key.clone();
        let kind_is_agg = matches!(kind, ColumnMenuKind::Aggregation);
        move |_, _window, cx| {
            state.update(cx, |s, cx| {
                if kind_is_agg {
                    s.toggle_agg_table_pinned_column(&sk, key.clone());
                } else {
                    s.toggle_table_pinned_column(&sk, key.clone());
                }
                cx.notify();
            });
        }
    }));
    menu =
        menu.item(PopupMenuItem::new("Hide Column").icon(Icon::new(IconName::EyeOff)).on_click({
            let state = state.clone();
            let sk = session_key.clone();
            let key = col_key.clone();
            let kind_is_agg = matches!(kind, ColumnMenuKind::Aggregation);
            move |_, _window, cx| {
                state.update(cx, |s, cx| {
                    if kind_is_agg {
                        s.toggle_agg_table_hidden_column(&sk, key.clone());
                    } else {
                        s.toggle_table_hidden_column(&sk, key.clone());
                    }
                    cx.notify();
                });
            }
        }));

    menu
}
