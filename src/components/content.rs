use gpui::*;

use crate::components::Button;
use crate::state::{ActiveTab, AppEvent, AppState, StatusLevel, TabKey, View};
use crate::theme::{borders, colors, spacing};
use crate::views::{CollectionView, DatabaseView};
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputState};
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{Icon, IconName, Sizable as _, WindowExt as _};

/// Content area component that shows collection view or welcome screen
pub struct ContentArea {
    state: Entity<AppState>,
    collection_view: Option<Entity<CollectionView>>,
    database_view: Option<Entity<DatabaseView>>,
    _subscriptions: Vec<Subscription>,
}

impl ContentArea {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![];

        subscriptions.push(cx.observe(&state, |_, _, cx| cx.notify()));

        // Subscribe to view-change events to lazily create collection view
        subscriptions.push(cx.subscribe(&state, |this, state, event, cx| match event {
            AppEvent::ViewChanged | AppEvent::Connected(_) => {
                let (should_create_collection, should_create_database) = {
                    let state_ref = state.read(cx);
                    (
                        state_ref.conn.selected_collection.is_some(),
                        matches!(state_ref.current_view, View::Database)
                            && state_ref.conn.selected_database.is_some(),
                    )
                };

                if should_create_collection && this.collection_view.is_none() {
                    this.collection_view =
                        Some(cx.new(|cx| CollectionView::new(state.clone(), cx)));
                }
                if should_create_database && this.database_view.is_none() {
                    this.database_view = Some(cx.new(|cx| DatabaseView::new(state.clone(), cx)));
                }

                cx.notify();
            }
            _ => {}
        }));

        // Check if we should create collection view initially
        let collection_view = if state.read(cx).conn.selected_collection.is_some() {
            Some(cx.new(|cx| CollectionView::new(state.clone(), cx)))
        } else {
            None
        };
        let database_view = if matches!(state.read(cx).current_view, View::Database)
            && state.read(cx).conn.selected_database.is_some()
        {
            Some(cx.new(|cx| DatabaseView::new(state.clone(), cx)))
        } else {
            None
        };

        Self { state, collection_view, database_view, _subscriptions: subscriptions }
    }
}

impl Render for ContentArea {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_ref = self.state.read(cx);
        let has_collection = state_ref.conn.selected_collection.is_some();
        let has_connection = state_ref.conn.active.is_some();
        let selected_db = state_ref.conn.selected_database.clone();
        let tabs: Vec<TabKey> = state_ref.tabs.open.clone();
        let active_tab = state_ref.tabs.active;
        let preview_tab = state_ref.tabs.preview.clone();
        let dirty_tabs = state_ref.tabs.dirty.clone();
        let current_view = state_ref.current_view;
        let error_text = state_ref.status_message.as_ref().and_then(|message| {
            if matches!(message.level, StatusLevel::Error) {
                Some(message.text.clone())
            } else {
                None
            }
        });

        if !tabs.is_empty() || preview_tab.is_some() {
            if matches!(current_view, View::Documents) && self.collection_view.is_none() {
                self.collection_view =
                    Some(cx.new(|cx| CollectionView::new(self.state.clone(), cx)));
            }
            if matches!(current_view, View::Database) && self.database_view.is_none() {
                self.database_view = Some(cx.new(|cx| DatabaseView::new(self.state.clone(), cx)));
            }
        }

        if !tabs.is_empty() || preview_tab.is_some() {
            let selected_index = match active_tab {
                ActiveTab::Preview => tabs.len(),
                ActiveTab::Index(index) => index.min(tabs.len().saturating_sub(1)),
                ActiveTab::None => 0,
            };
            let state = self.state.clone();
            let tab_bar = TabBar::new("collection-tabs")
                .underline()
                .small()
                .selected_index(selected_index)
                .menu(true)
                .on_click(move |index, _window, cx| {
                    let index = *index;
                    state.update(cx, |state, cx| {
                        if index < state.tabs.open.len() {
                            state.select_tab(index, cx);
                        } else {
                            state.select_preview_tab(cx);
                        }
                    });
                })
                .children(
                    tabs.iter()
                        .enumerate()
                        .map(|(index, tab)| {
                            let (label, is_dirty) = match tab {
                                TabKey::Collection(tab) => (
                                    format!("{}/{}", tab.database, tab.collection),
                                    dirty_tabs.contains(tab),
                                ),
                                TabKey::Database(tab) => (tab.database.clone(), false),
                            };
                            let state = self.state.clone();
                            let close_button = div()
                                .id(("tab-close", index))
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(16.0))
                                .h(px(16.0))
                                .rounded(borders::radius_sm())
                                .cursor_pointer()
                                .hover(|s| s.bg(colors::bg_hover()))
                                .child(
                                    Icon::new(IconName::Close)
                                        .xsmall()
                                        .text_color(colors::text_muted()),
                                )
                                .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                    cx.stop_propagation();
                                    state.update(cx, |state, cx| {
                                        state.close_tab(index, cx);
                                    });
                                });

                            let dirty_dot =
                                div().w(px(6.0)).h(px(6.0)).rounded_full().bg(colors::accent());

                            let mut tab_view = Tab::new().label(label);
                            if is_dirty {
                                tab_view = tab_view.prefix(dirty_dot);
                            }

                            tab_view.suffix(close_button)
                        })
                        .chain(preview_tab.clone().map(|tab| {
                            let label = format!("{}/{}", tab.database, tab.collection);
                            let is_dirty = dirty_tabs.contains(&tab);
                            let state = self.state.clone();
                            let close_button = div()
                                .id("tab-close-preview")
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(16.0))
                                .h(px(16.0))
                                .rounded(borders::radius_sm())
                                .cursor_pointer()
                                .hover(|s| s.bg(colors::bg_hover()))
                                .child(
                                    Icon::new(IconName::Close)
                                        .xsmall()
                                        .text_color(colors::text_muted()),
                                )
                                .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                    cx.stop_propagation();
                                    state.update(cx, |state, cx| {
                                        state.close_preview_tab(cx);
                                    });
                                });

                            let dirty_dot =
                                div().w(px(6.0)).h(px(6.0)).rounded_full().bg(colors::accent());

                            let mut tab_view = Tab::new().child(
                                div().italic().text_color(colors::text_muted()).child(label),
                            );
                            if is_dirty {
                                tab_view = tab_view.prefix(dirty_dot);
                            }

                            tab_view.suffix(close_button)
                        })),
                );

            let mut root = div().flex().flex_col().flex_1().h_full();

            if let Some(text) = error_text.clone() {
                root = root.child(Self::render_error_banner(text, self.state.clone()));
            }

            return root
                .child(div().pl(spacing::sm()).child(tab_bar))
                .child(match current_view {
                    View::Database => self
                        .database_view
                        .as_ref()
                        .map(|view| view.clone().into_any_element())
                        .unwrap_or_else(|| div().into_any_element()),
                    _ => {
                        if has_collection {
                            self.collection_view
                                .as_ref()
                                .map(|view| view.clone().into_any_element())
                                .unwrap_or_else(|| div().into_any_element())
                        } else {
                            div()
                                .flex()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .text_sm()
                                .text_color(colors::text_muted())
                                .child("Select a tab or open a collection")
                                .into_any_element()
                        }
                    }
                })
                .into_any_element();
        }

        if matches!(current_view, View::Database)
            && let Some(view) = &self.database_view
        {
            let mut root = div().flex().flex_col().flex_1().h_full();

            if let Some(text) = error_text.clone() {
                root = root.child(Self::render_error_banner(text, self.state.clone()));
            }

            return root.child(view.clone()).into_any_element();
        }

        if has_collection && let Some(view) = &self.collection_view {
            let mut root = div().flex().flex_col().flex_1().h_full();

            if let Some(text) = error_text.clone() {
                root = root.child(Self::render_error_banner(text, self.state.clone()));
            }

            return root.child(view.clone()).into_any_element();
        }

        let hint = if !has_connection {
            "Add a connection to get started".to_string()
        } else if selected_db.is_none() {
            "Select a database in the sidebar".to_string()
        } else {
            "Select a collection to view documents".to_string()
        };

        let mut root = div().flex().flex_col().flex_1().h_full().bg(colors::bg_app());

        if let Some(text) = error_text.clone() {
            root = root.child(Self::render_error_banner(text, self.state.clone()));
        }

        root.child(
            div().flex().flex_1().items_center().justify_center().child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::lg())
                    .items_center()
                    .child(img("logo/openmango-logo.svg").w(px(120.0)).h(px(120.0)))
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(colors::accent())
                            .font_family(crate::theme::fonts::heading())
                            .child("OpenMango"),
                    )
                    .child(
                        div()
                            .text_base()
                            .text_color(colors::text_secondary())
                            .child("MongoDB GUI Client"),
                    )
                    .child(
                        div()
                            .mt(spacing::lg())
                            .text_sm()
                            .text_color(colors::text_muted())
                            .child(hint),
                    ),
            ),
        )
        .into_any_element()
    }
}

impl ContentArea {
    fn format_error_banner_preview(message: &str) -> String {
        const MAX_PREVIEW_CHARS: usize = 100;

        let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut out = String::new();
        for (idx, ch) in normalized.chars().enumerate() {
            if idx >= MAX_PREVIEW_CHARS {
                out.push('…');
                break;
            }
            out.push(ch);
        }
        out
    }

    fn render_error_banner(message: String, state: Entity<AppState>) -> AnyElement {
        let preview = Self::format_error_banner_preview(&message);
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::md())
            .w_full()
            .px(spacing::md())
            .py(spacing::sm())
            .bg(colors::bg_error())
            .border_b_1()
            .border_color(colors::border_error())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .xsmall()
                            .text_color(colors::status_error()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .text_color(colors::text_error())
                            .truncate()
                            .child(preview),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .flex_shrink_0()
                    .child(Button::new("show-error").ghost().compact().label("Show more").on_click(
                        {
                            let message = message.clone();
                            move |_, window, cx| {
                                let message = message.clone();
                                let text_state = cx.new(|cx| {
                                    InputState::new(window, cx).code_editor("text").soft_wrap(true)
                                });
                                text_state.update(cx, |state, cx| {
                                    state.set_value(message.clone(), window, cx);
                                });
                                window.open_dialog(cx, move |dialog: Dialog, _window, _cx| {
                                    dialog
                                        .title("Error details")
                                        .min_w(px(720.0))
                                        .child(
                                            div().p(spacing::md()).child(
                                                Input::new(&text_state)
                                                    .font_family(crate::theme::fonts::mono())
                                                    .h(px(320.0))
                                                    .w_full()
                                                    .disabled(true),
                                            ),
                                        )
                                        .footer({
                                            let message = message.clone();
                                            move |_ok_fn, _cancel_fn, _window, _cx| {
                                                vec![
                                                    Button::new("copy-error")
                                                        .label("Copy")
                                                        .on_click({
                                                            let message = message.clone();
                                                            move |_, _window, cx| {
                                                                cx.write_to_clipboard(
                                                                    ClipboardItem::new_string(
                                                                        message.clone(),
                                                                    ),
                                                                );
                                                            }
                                                        })
                                                        .into_any_element(),
                                                    Button::new("close-error")
                                                        .label("Close")
                                                        .on_click(|_, window, cx| {
                                                            window.close_dialog(cx);
                                                        })
                                                        .into_any_element(),
                                                ]
                                            }
                                        })
                                });
                            }
                        },
                    ))
                    .child(
                        Button::new("dismiss-error")
                            .ghost()
                            .icon(Icon::new(IconName::Close).xsmall())
                            .on_click({
                                let state = state.clone();
                                move |_, _window, cx| {
                                    state.update(cx, |state, cx| {
                                        state.status_message = None;
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            )
            .into_any_element()
    }
}
