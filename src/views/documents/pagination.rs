//! Pagination controls for collection view.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button as MenuButton, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Icon, IconName, Sizable as _};
use gpui_kit::*;

use crate::components::Button;
use crate::state::{AppCommands, AppState, SessionKey};
use crate::theme::spacing;

use super::CollectionView;

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
        let state_for_prev = state.clone();
        let state_for_next = state.clone();
        let session_key_prev = session_key.clone();
        let session_key_next = session_key.clone();

        let per_page_selector = {
            let label = format!("{} / page", per_page);
            let btn = MenuButton::new("per-page-selector")
                .ghost()
                .xsmall()
                .label(label)
                .dropdown_caret(true)
                .with_size(gpui_kit::component::Size::XSmall)
                .disabled(is_loading || session_key.is_none());

            let sk = session_key.clone();
            btn.dropdown_menu_with_anchor(Anchor::TopLeft, move |mut menu: PopupMenu, _, _| {
                for &opt in PER_PAGE_OPTIONS {
                    let label = format!("{}", opt);
                    let state = state.clone();
                    let view = view.clone();
                    let sk = sk.clone();
                    let is_current = opt == per_page;
                    menu = menu.item(PopupMenuItem::new(label).checked(is_current).on_click(
                        move |_, _, cx| {
                            let Some(sk) = sk.clone() else {
                                return;
                            };
                            state.update(cx, |state, cx| {
                                state.set_per_page(&sk, opt);
                                cx.notify();
                            });
                            view.update(cx, |this, cx| {
                                this.view_model.invalidate_table();
                                cx.notify();
                            });
                            AppCommands::load_documents_for_session(state.clone(), sk, cx);
                        },
                    ));
                }
                menu
            })
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .px(spacing::lg())
            .py(px(5.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Showing {}-{} of {}", range_start, range_end, total)),
                    )
                    .child(per_page_selector),
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
                    .child(div().text_sm().text_color(cx.theme().foreground).child(format!(
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
