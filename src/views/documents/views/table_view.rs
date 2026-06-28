use gpui::*;
use gpui_component::table::Table;

use crate::bson::DocumentKey;
use crate::state::SessionKey;

use super::super::CollectionView;

impl CollectionView {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::views::documents) fn render_table_subview(
        &mut self,
        total: u64,
        display_page: u64,
        total_pages: u64,
        per_page: i64,
        range_start: u64,
        range_end: u64,
        is_loading: bool,
        session_key: Option<SessionKey>,
        _selected_docs: std::collections::HashSet<DocumentKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let table_state = self.view_model.ensure_table_state(&self.state, &view, window, cx);
        self.view_model.rebuild_table(&self.state, &view, window, cx);

        let table_element = Table::new(&table_state).stripe(true).bordered(false);

        let documents_view = div().flex().flex_1().min_w(px(0.0)).overflow_hidden().child(
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(table_element),
        );

        let main_panel =
            div().flex().flex_col().flex_1().min_w(px(0.0)).child(documents_view).child(
                Self::render_pagination(
                    display_page,
                    total_pages,
                    per_page,
                    range_start,
                    range_end,
                    total,
                    is_loading,
                    session_key.clone(),
                    self.state.clone(),
                    view,
                    cx,
                ),
            );

        main_panel.into_any_element()
    }
}
