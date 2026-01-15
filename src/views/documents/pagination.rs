//! Pagination controls for collection view.

use gpui::*;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::components::Button;
use crate::state::{AppCommands, AppState, SessionKey};
use crate::theme::{colors, spacing};

use super::CollectionView;

impl CollectionView {
    /// Render pagination controls.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_pagination(
        page: u64,
        total_pages: u64,
        range_start: u64,
        range_end: u64,
        total: u64,
        is_loading: bool,
        session_key: Option<SessionKey>,
        state_for_prev: Entity<AppState>,
        state_for_next: Entity<AppState>,
    ) -> impl IntoElement {
        let session_key_prev = session_key.clone();
        let session_key_next = session_key.clone();

        div()
            .flex()
            .items_center()
            .justify_between()
            .px(spacing::lg())
            .py(spacing::sm())
            .border_t_1()
            .border_color(colors::border())
            .child(
                div()
                    .text_sm()
                    .text_color(colors::text_muted())
                    .child(format!("Showing {}-{} of {}", range_start, range_end, total)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(
                        Button::new("prev")
                            .ghost()
                            .disabled(page == 0 || is_loading || session_key.is_none())
                            .icon(Icon::new(IconName::ChevronLeft).xsmall())
                            .on_click(move |_, _, cx| {
                                let Some(session_key) = session_key_prev.clone() else {
                                    return;
                                };
                                state_for_prev.update(cx, |state, cx| {
                                    state.prev_page(&session_key);
                                    cx.notify();
                                });
                                AppCommands::load_documents_for_session(
                                    state_for_prev.clone(),
                                    session_key,
                                    cx,
                                );
                            }),
                    )
                    .child(div().text_sm().text_color(colors::text_primary()).child(format!(
                        "Page {} of {}",
                        page + 1,
                        total_pages
                    )))
                    .child(
                        Button::new("next")
                            .ghost()
                            .disabled(
                                page + 1 >= total_pages || is_loading || session_key.is_none(),
                            )
                            .icon(Icon::new(IconName::ChevronRight).xsmall())
                            .on_click(move |_, _, cx| {
                                let Some(session_key) = session_key_next.clone() else {
                                    return;
                                };
                                state_for_next.update(cx, |state, cx| {
                                    state.next_page(&session_key, total_pages);
                                    cx.notify();
                                });
                                AppCommands::load_documents_for_session(
                                    state_for_next.clone(),
                                    session_key,
                                    cx,
                                );
                            }),
                    ),
            )
    }
}
