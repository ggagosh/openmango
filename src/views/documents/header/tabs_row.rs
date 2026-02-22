//! Subview tabs rendering for collection header.

use gpui::*;
use gpui_component::Sizable as _;
use gpui_component::tab::{Tab, TabBar};

use crate::state::{AppCommands, AppState, CollectionSubview, SessionKey};

/// Render the subview tabs (Documents/Indexes/Stats/Aggregation).
pub fn render_subview_tabs(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    active_subview: CollectionSubview,
) -> TabBar {
    TabBar::new("collection-subview-tabs")
        .underline()
        .xsmall()
        .selected_index(active_subview.to_index())
        .on_click({
            let session_key = session_key.clone();
            let state_for_subview = state.clone();
            move |index, _window, cx| {
                let Some(session_key) = session_key.clone() else {
                    return;
                };
                let next = CollectionSubview::from_index(*index);
                let should_load = state_for_subview.update(cx, |state, cx| {
                    let should_load = state.set_collection_subview(&session_key, next);
                    cx.notify();
                    should_load
                });
                if next == CollectionSubview::Indexes {
                    AppCommands::load_collection_indexes(
                        state_for_subview.clone(),
                        session_key,
                        false,
                        cx,
                    );
                } else if should_load && next == CollectionSubview::Stats {
                    AppCommands::load_collection_stats(state_for_subview.clone(), session_key, cx);
                } else if should_load && next == CollectionSubview::Schema {
                    AppCommands::analyze_collection_schema(
                        state_for_subview.clone(),
                        session_key,
                        cx,
                    );
                }
            }
        })
        .children(vec![
            Tab::new().label("Documents"),
            Tab::new().label("Indexes"),
            Tab::new().label("Stats"),
            Tab::new().label("Aggregation"),
            Tab::new().label("Schema"),
        ])
}
