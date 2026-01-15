//! Selection management for AppState.

use gpui::Context;

use super::AppState;
use super::tabs::TabOpenMode;

impl AppState {
    /// Select a database (clears collection + documents)
    pub fn select_database(&mut self, database: String, cx: &mut Context<Self>) {
        self.open_database_tab(database, cx);
    }

    /// Select a collection (resets pagination + documents)
    pub fn select_collection(
        &mut self,
        database: String,
        collection: String,
        cx: &mut Context<Self>,
    ) {
        self.open_collection_with_mode(database, collection, TabOpenMode::Permanent, cx);
    }

    pub fn preview_collection(
        &mut self,
        database: String,
        collection: String,
        cx: &mut Context<Self>,
    ) {
        self.close_database_tabs(cx);
        self.open_collection_with_mode(database, collection, TabOpenMode::Preview, cx);
    }
}
