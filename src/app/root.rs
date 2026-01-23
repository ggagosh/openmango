use gpui::prelude::InteractiveElement as _;
use gpui::*;

use super::sidebar::Sidebar;
use crate::components::action_bar::ActionBar;
use crate::components::{ConnectionManager, ContentArea, StatusBar, open_confirm_dialog};
use crate::keyboard::{
    CloseTab, CopyConnectionUri, CopySelectionName, CreateCollection, CreateDatabase, CreateIndex,
    DeleteConnection, DeleteDatabase, DisconnectConnection, EditConnection, NewConnection, NextTab,
    OpenActionBar, PrevTab, QuitApp, RefreshView,
};
use crate::state::{AppCommands, AppState, CollectionSubview, View};
use crate::theme::{borders, colors, spacing};

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
                    _ => {}
                }
            }
            View::Database => key_context.push_str(" Database"),
            View::Databases => key_context.push_str(" Databases"),
            View::Collections => key_context.push_str(" Collections"),
            View::Welcome => key_context.push_str(" Welcome"),
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
            .bg(colors::bg_app())
            .text_color(colors::text_primary())
            .font_family(crate::theme::fonts::ui())
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
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .overflow_hidden()
                    .child(self.sidebar.clone())
                    .child(div().flex().flex_1().min_w(px(0.0)).child(self.content_area.clone())),
            )
            .child(StatusBar::new(is_connected, connection_name, status_message, read_only))
            .children(dialog_layer)
            .child(self.action_bar.clone());

        if self.key_debug {
            root =
                root.child(render_key_debug_overlay(&key_context, self.last_keystroke.as_deref()));
        }

        root
    }
}

fn render_key_debug_overlay(key_context: &str, last_keystroke: Option<&str>) -> AnyElement {
    let last_keystroke = last_keystroke.unwrap_or("-");
    div()
        .absolute()
        .bottom(px(12.0))
        .right(px(12.0))
        .w(px(320.0))
        .p(spacing::sm())
        .rounded(borders::radius_sm())
        .bg(colors::bg_header())
        .border_1()
        .border_color(colors::border())
        .text_xs()
        .text_color(colors::text_primary())
        .font_family(crate::theme::fonts::mono())
        .child(div().text_sm().child("Keymap debug"))
        .child(div().text_color(colors::text_muted()).child("Key context:"))
        .child(div().child(key_context.to_string()))
        .child(div().text_color(colors::text_muted()).child("Last keystroke:"))
        .child(div().child(last_keystroke.to_string()))
        .into_any_element()
}
