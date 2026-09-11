//! Native page navigation shared by Tree, Table, and JSON.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button as MenuButton, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::pagination::Pagination;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::CollectionView;
use crate::state::{AppState, SessionKey};
use crate::theme::spacing;

const PER_PAGE_OPTIONS: &[i64] = &[10, 25, 50, 100];

impl CollectionView {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_pagination(
        page: u64,
        total_pages: u64,
        per_page: i64,
        range_start: u64,
        range_end: u64,
        total: u64,
        is_loading: bool,
        session_key: Option<SessionKey>,
        state: Entity<AppState>,
        view: Entity<CollectionView>,
        cx: &App,
    ) -> impl IntoElement {
        let disabled = is_loading || session_key.is_none();
        let page_state = state.clone();
        let page_view = view.clone();
        let page_key = session_key.clone();
        let edge_state = state.clone();
        let edge_view = view.clone();
        let edge_key = session_key.clone();
        let page_edges = MenuButton::new("document-page-edges")
            .ghost()
            .xsmall()
            .label(format!("Page {} of {}", page + 1, total_pages.max(1)))
            .disabled(disabled)
            .dropdown_caret(true)
            .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu: PopupMenu, _, _| {
                for (label, target) in
                    [("First page", 0), ("Last page", total_pages.saturating_sub(1))]
                {
                    let state = edge_state.clone();
                    let view = edge_view.clone();
                    let key = edge_key.clone();
                    menu = menu.item(PopupMenuItem::new(label).disabled(target == page).on_click(
                        move |_, window, cx| {
                            if let Some(key) = key.clone() {
                                CollectionView::reload_document_page(
                                    view.clone(),
                                    state.clone(),
                                    key,
                                    window,
                                    cx,
                                    move |state, key| state.set_document_page(key, target),
                                );
                            }
                        },
                    ));
                }
                menu
            });
        let page_size = MenuButton::new("per-page-selector")
            .ghost()
            .xsmall()
            .label(format!("{per_page} / page"))
            .dropdown_caret(true)
            .disabled(disabled)
            .dropdown_menu_with_anchor(Anchor::TopLeft, move |mut menu: PopupMenu, _, _| {
                for &size in PER_PAGE_OPTIONS {
                    let state = state.clone();
                    let view = view.clone();
                    let key = session_key.clone();
                    menu = menu.item(
                        PopupMenuItem::new(size.to_string()).checked(size == per_page).on_click(
                            move |_, window, cx| {
                                let Some(key) = key.clone() else { return };
                                CollectionView::reload_document_page(
                                    view.clone(),
                                    state.clone(),
                                    key,
                                    window,
                                    cx,
                                    move |state, key| state.set_per_page(key, size),
                                );
                            },
                        ),
                    );
                }
                menu
            });
        let pagination = Pagination::new("document-pages")
            .xsmall()
            .visible_pages(5)
            // ponytail: Kit 0.6 builds every hidden page in its ellipsis menu; use compact controls for large result sets until that menu is virtualized.
            .when(total_pages > 100, |pagination| pagination.compact())
            .current_page(page.saturating_add(1) as usize)
            .total_pages(total_pages.max(1) as usize)
            .disabled(disabled)
            .on_click(move |page, window, cx| {
                let Some(key) = page_key.clone() else { return };
                let page = page.saturating_sub(1) as u64;
                CollectionView::reload_document_page(
                    page_view.clone(),
                    page_state.clone(),
                    key,
                    window,
                    cx,
                    move |state, key| state.set_document_page(key, page),
                );
            });
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(spacing::sm())
            .px(spacing::lg())
            .py(px(5.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{range_start}–{range_end} of {total} documents")),
                    )
                    .child(page_size),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .when(total_pages > 100, |row| row.child(page_edges))
                    .child(pagination),
            )
    }
}
