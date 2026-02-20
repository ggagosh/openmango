use gpui::prelude::{FluentBuilder as _, InteractiveElement as _};
use gpui::*;
use gpui_component::ActiveTheme as _;

use super::sidebar::Sidebar;
use crate::components::action_bar::ActionBar;
use crate::components::{ConnectionManager, ContentArea, StatusBar, open_confirm_dialog};
use crate::keyboard::{
    CloseTab, CopyConnectionUri, CopySelectionName, CreateCollection, CreateDatabase, CreateIndex,
    DeleteConnection, DeleteDatabase, DisconnectConnection, DownloadUpdate, EditConnection,
    InstallUpdate, NewConnection, NextTab, OpenActionBar, OpenForge, OpenSettings, PrevTab,
    QuitApp, RefreshView,
};
use crate::state::app_state::updater::UpdateStatus;
use crate::state::{AppCommands, AppState, CollectionSubview, View};
use crate::theme::{borders, spacing};

// =============================================================================
// App Component
// =============================================================================

pub struct AppRoot {
    pub(super) state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    content_area: Entity<ContentArea>,
    pub(super) action_bar: Entity<ActionBar>,
    pub(super) key_debug: bool,
    pub(super) last_keystroke: Option<String>,
    sidebar_dragging: bool,
    sidebar_drag_start_x: Pixels,
    sidebar_drag_start_width: Pixels,
    _subscriptions: Vec<Subscription>,
}

impl AppRoot {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Create the app state entity
        let state = cx.new(|_| AppState::new());

        // Create sidebar with state reference
        let sidebar = cx.new(|cx| Sidebar::new(state.clone(), window, cx));

        // Create content area with state reference
        let content_area = cx.new(|cx| ContentArea::new(state.clone(), cx));

        // Create action bar with execution callback
        let action_bar = cx.new(|_cx| {
            ActionBar::new(state.clone()).on_execute({
                let state = state.clone();
                move |execution, window, cx| {
                    Self::execute_action(&state, execution, window, cx);
                }
            })
        });

        cx.observe(&state, |_, _, cx| cx.notify()).detach();

        // Delayed update check + periodic re-check (default 4h, override with
        // OPENMANGO_UPDATE_INTERVAL_SECS=30 for testing)
        cx.spawn({
            let state = state.clone();
            let startup_delay = std::env::var("OPENMANGO_UPDATE_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map(|s| s.min(5)) // use short startup delay when testing
                .unwrap_or(10);
            let recheck_secs = std::env::var("OPENMANGO_UPDATE_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(4 * 60 * 60);
            async move |_this: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                gpui::Timer::after(std::time::Duration::from_secs(startup_delay)).await;
                let _ = cx.update(|cx| {
                    AppCommands::check_for_updates(state.clone(), cx);
                });
                // Periodic re-check
                loop {
                    gpui::Timer::after(std::time::Duration::from_secs(recheck_secs)).await;
                    let should_check = cx
                        .update(|cx| {
                            let s = state.read(cx);
                            s.settings.auto_update && matches!(s.update_status, UpdateStatus::Idle)
                        })
                        .unwrap_or(false);
                    if should_check {
                        let _ = cx.update(|cx| {
                            AppCommands::check_for_updates(state.clone(), cx);
                        });
                    }
                }
            }
        })
        .detach();

        // Show "What's New" dialog if build changed since last launch
        {
            let current_sha = env!("OPENMANGO_GIT_SHA");
            let last_seen = &state.read(cx).settings.last_seen_version;
            // Skip if SHA matches, or if last_seen is a legacy semver value
            // (pre-SHA migration) — treat those as "already seen"
            let is_legacy_version = last_seen.contains('.');
            let force_changelog = std::env::var("OPENMANGO_SHOW_CHANGELOG").is_ok();
            let should_show = force_changelog || (!is_legacy_version && last_seen != current_sha);
            if !force_changelog && is_legacy_version {
                // Migrate legacy semver value to current SHA silently
                state.update(cx, |state, _cx| {
                    state.settings.last_seen_version = current_sha.to_string();
                    state.save_settings();
                });
            } else if should_show {
                let state_clone = state.clone();
                window.defer(cx, move |_window, cx| {
                    crate::changelog::open_changelog_tab(state_clone, cx);
                });
            }
        }

        let key_debug = std::env::var("OPENMANGO_DEBUG_KEYS").is_ok();
        let mut subscriptions = Vec::new();

        let subscription = Self::install_global_shortcuts(cx);
        subscriptions.push(subscription);

        Self {
            state,
            sidebar,
            content_area,
            action_bar,
            key_debug,
            last_keystroke: None,
            sidebar_dragging: false,
            sidebar_drag_start_x: px(0.0),
            sidebar_drag_start_width: px(0.0),
            _subscriptions: subscriptions,
        }
    }
}

impl Render for AppRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Read state for StatusBar props
        let state = self.state.read(cx);
        let active_conn = state.active_connection();
        let is_connected = active_conn.is_some();
        let connection_name = active_conn.map(|c| c.config.name.clone());
        let status_message = state.status_message();
        let read_only = active_conn.map(|c| c.config.read_only).unwrap_or(false);
        let show_status_bar = state.settings.appearance.show_status_bar;
        let vibrancy = state.startup_vibrancy;
        let update_status = state.update_status.clone();

        let documents_subview = if matches!(state.current_view, View::Documents) {
            state.current_session_key().and_then(|key| state.session_subview(&key))
        } else {
            None
        };

        let mut key_context = String::from("Workspace");
        match state.current_view {
            View::Documents => {
                key_context.push_str(" Documents");
                match documents_subview {
                    Some(CollectionSubview::Indexes) => key_context.push_str(" Indexes"),
                    Some(CollectionSubview::Stats) => key_context.push_str(" Stats"),
                    Some(CollectionSubview::Aggregation) => key_context.push_str(" Aggregation"),
                    _ => {}
                }
            }
            View::Database => key_context.push_str(" Database"),
            View::Databases => key_context.push_str(" Databases"),
            View::Collections => key_context.push_str(" Collections"),
            View::Transfer => key_context.push_str(" Transfer"),
            View::Forge => key_context.push_str(" Forge"),
            View::Welcome => key_context.push_str(" Welcome"),
            View::Settings => key_context.push_str(" Settings"),
            View::Changelog => key_context.push_str(" Changelog"),
        }

        // Render dialog layer (Context derefs to App)
        use gpui_component::Root;
        let dialog_layer = Root::render_dialog_layer(window, cx);

        let mut root = div()
            .key_context(key_context.as_str())
            .flex()
            .flex_col()
            .size_full()
            .relative()
            .when(vibrancy, |s| s.pt(px(28.0)))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(crate::theme::fonts::ui())
            .line_height(crate::theme::fonts::ui_line_height())
            .on_action(cx.listener(|this, _: &CloseTab, _window, cx| {
                this.handle_close_tab(cx);
            }))
            .on_action(cx.listener(|this, _: &NextTab, _window, cx| {
                this.state.update(cx, |state, cx| {
                    state.select_next_tab(cx);
                });
            }))
            .on_action(cx.listener(|this, _: &PrevTab, _window, cx| {
                this.state.update(cx, |state, cx| {
                    state.select_prev_tab(cx);
                });
            }))
            .on_action(cx.listener(|this, _: &NewConnection, window, cx| {
                this.handle_new_connection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CreateDatabase, window, cx| {
                this.handle_create_database(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CreateCollection, window, cx| {
                this.handle_create_collection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CreateIndex, window, cx| {
                this.handle_create_index(window, cx);
            }))
            .on_action(cx.listener(|_this, _: &QuitApp, _window, cx| {
                cx.quit();
            }))
            .on_action(cx.listener(|this, _: &DeleteDatabase, window, cx| {
                let Some(database_key) = this.state.read(cx).current_database_key() else {
                    return;
                };
                let message =
                    format!("Drop database \"{}\"? This cannot be undone.", database_key.database);
                open_confirm_dialog(window, cx, "Drop database", message, "Drop", true, {
                    let state = this.state.clone();
                    let database = database_key.database.clone();
                    let connection_id = database_key.connection_id;
                    move |_window, cx| {
                        state.update(cx, |state, cx| {
                            state.select_connection(Some(connection_id), cx);
                        });
                        AppCommands::drop_database(state.clone(), database.clone(), cx);
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &DeleteConnection, window, cx| {
                if let Some(connection_id) = this.state.read(cx).selected_connection_id() {
                    let name = this
                        .state
                        .read(cx)
                        .connection_name(connection_id)
                        .unwrap_or_else(|| "connection".to_string());
                    let message = format!("Remove connection \"{name}\"?");
                    open_confirm_dialog(
                        window,
                        cx,
                        "Remove connection",
                        message,
                        "Remove",
                        true,
                        {
                            let state = this.state.clone();
                            move |_window, cx| {
                                state.update(cx, |state, cx| {
                                    state.remove_connection(connection_id, cx);
                                });
                            }
                        },
                    );
                }
            }))
            .on_action(cx.listener(|this, _: &DisconnectConnection, _window, cx| {
                if let Some(connection_id) = this.state.read(cx).selected_connection_id() {
                    AppCommands::disconnect(this.state.clone(), connection_id, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &EditConnection, window, cx| {
                if let Some(connection_id) = this.state.read(cx).selected_connection_id() {
                    ConnectionManager::open_selected(this.state.clone(), connection_id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CopyConnectionUri, _window, cx| {
                if let Some(connection_id) = this.state.read(cx).selected_connection_id()
                    && let Some(uri) = this.state.read(cx).connection_uri(connection_id)
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(uri));
                }
            }))
            .on_action(cx.listener(|this, _: &CopySelectionName, _window, cx| {
                let state_ref = this.state.read(cx);
                let selection_name = if let Some(collection) = state_ref.selected_collection_name()
                {
                    Some(collection)
                } else if let Some(database) = state_ref.selected_database_name() {
                    Some(database)
                } else if let Some(connection_id) = state_ref.selected_connection_id() {
                    state_ref.connection_name(connection_id)
                } else {
                    None
                };
                if let Some(name) = selection_name {
                    cx.write_to_clipboard(ClipboardItem::new_string(name));
                }
            }))
            .on_action(cx.listener(|this, _: &RefreshView, _window, cx| {
                this.handle_refresh(cx);
            }))
            .on_action(cx.listener(|this, _: &OpenActionBar, window, cx| {
                this.action_bar.update(cx, |bar, cx| {
                    bar.toggle(window, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, _window, cx| {
                this.state.update(cx, |state, cx| {
                    state.open_settings_tab(cx);
                });
            }))
            .on_action(cx.listener(|this, _: &OpenForge, _window, cx| {
                this.state.update(cx, |state, cx| {
                    let Some(key) = state.current_database_key() else {
                        return;
                    };
                    state.open_forge_tab(key.connection_id, key.database, None, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &DownloadUpdate, _window, cx| {
                AppCommands::download_update(this.state.clone(), cx);
            }))
            .on_action(cx.listener(|this, _: &InstallUpdate, _window, cx| {
                AppCommands::install_update(this.state.clone(), cx);
            }))
            .child({
                let is_dragging = self.sidebar_dragging;

                let resize_handle = div()
                    .id("sidebar-resize-handle")
                    .flex_shrink_0()
                    .w(px(4.0))
                    .h_full()
                    .cursor_col_resize()
                    .bg(crate::theme::colors::transparent())
                    .hover(|s| s.bg(cx.theme().ring))
                    .when(is_dragging, |s: Stateful<Div>| s.bg(cx.theme().ring))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                            this.sidebar_dragging = true;
                            this.sidebar_drag_start_x = event.position.x;
                            this.sidebar_drag_start_width = this.sidebar.read(cx).width();
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(|this, event: &ClickEvent, _window, cx| {
                        if event.click_count() >= 2 {
                            this.sidebar.update(cx, |sidebar, cx| {
                                sidebar.toggle_collapsed();
                                cx.notify();
                            });
                            cx.notify();
                        }
                    }));

                let mut row = div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(self.sidebar.clone())
                    .child(resize_handle)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .child(self.content_area.clone()),
                    );

                if is_dragging {
                    row = row
                        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                            let delta = event.position.x - this.sidebar_drag_start_x;
                            let new_width = this.sidebar_drag_start_width + delta;
                            this.sidebar.update(cx, |sidebar, cx| {
                                sidebar.set_width(new_width);
                                cx.notify();
                            });
                            cx.notify();
                        }))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                                this.sidebar_dragging = false;
                                cx.notify();
                            }),
                        );
                }

                row
            })
            .children(show_status_bar.then(|| {
                let sidebar_collapsed = self.sidebar.read(cx).width() == px(0.0);
                let sidebar = self.sidebar.clone();
                StatusBar::new(
                    is_connected,
                    connection_name,
                    status_message,
                    read_only,
                    update_status,
                    self.state.clone(),
                )
                .sidebar_collapsed(sidebar_collapsed)
                .on_toggle_sidebar(move |_window: &mut Window, cx: &mut App| {
                    sidebar.update(cx, |sidebar, cx| {
                        sidebar.toggle_collapsed();
                        cx.notify();
                    });
                })
            }))
            .children(dialog_layer)
            .child(self.action_bar.clone());

        if self.key_debug {
            root = root.child(render_key_debug_overlay(
                &key_context,
                self.last_keystroke.as_deref(),
                cx,
            ));
        }

        root
    }
}

fn render_key_debug_overlay(
    key_context: &str,
    last_keystroke: Option<&str>,
    cx: &App,
) -> AnyElement {
    let last_keystroke = last_keystroke.unwrap_or("-");
    div()
        .absolute()
        .bottom(px(12.0))
        .right(px(12.0))
        .w(px(320.0))
        .p(spacing::sm())
        .rounded(borders::radius_sm())
        .bg(cx.theme().tab_bar)
        .border_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().foreground)
        .font_family(crate::theme::fonts::mono())
        .child(div().text_sm().child("Keymap debug"))
        .child(div().text_color(cx.theme().muted_foreground).child("Key context:"))
        .child(div().child(key_context.to_string()))
        .child(div().text_color(cx.theme().muted_foreground).child("Last keystroke:"))
        .child(div().child(last_keystroke.to_string()))
        .into_any_element()
}
