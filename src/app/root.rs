use gpui::prelude::{FluentBuilder as _, InteractiveElement as _};
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::tooltip::Tooltip;
use uuid::Uuid;

use super::sidebar::Sidebar;
use crate::components::action_bar::ActionBar;
use crate::components::{
    ConnectionManager, ContentArea, StatusBar, open_confirm_dialog, request_app_quit,
    request_disconnect_connection, request_remove_connection,
};
use crate::helpers::keystore::KeyStore;
use crate::helpers::validate::UriSecrets;
use crate::keyboard::{
    CloseTab, CopyConnectionUri, CopySelectionName, CreateCollection, CreateDatabase, CreateIndex,
    DeleteConnection, DeleteDatabase, DisconnectConnection, DownloadUpdate, EditConnection,
    FocusContent, FocusSidebar, InstallUpdate, NewConnection, NextTab, OpenActionBar, OpenForge,
    OpenSettings, PrevTab, QuitApp, RefreshView, ToggleAiPanel,
};
use crate::state::app_state::updater::UpdateStatus;
use crate::state::app_state::{
    ConnectionSecrets, LEGACY_CONNECTION_SECRET_KEYS, connection_secret_bundle_key,
};
use crate::state::{AppCommands, AppState, CollectionSubview, View};
use crate::theme::{borders, islands, spacing};
use crate::views::AiView;

const AI_ISLAND_DEFAULT_WIDTH: f32 = 380.0;
const AI_ISLAND_MIN_WIDTH: f32 = 320.0;
const AI_ISLAND_MAX_WIDTH: f32 = 900.0;

// =============================================================================
// App Component
// =============================================================================

pub struct AppRoot {
    pub(super) state: Entity<AppState>,
    focus_handle: FocusHandle,
    pub(super) sidebar: Entity<Sidebar>,
    content_area: Entity<ContentArea>,
    ai_view: Entity<AiView>,
    pub(super) action_bar: Entity<ActionBar>,
    pub(super) key_debug: bool,
    pub(super) last_keystroke: Option<String>,
    sidebar_dragging: bool,
    sidebar_drag_start_x: Pixels,
    sidebar_drag_start_width: Pixels,
    ai_dragging: bool,
    ai_drag_start_x: Pixels,
    ai_drag_start_width: f32,
    ai_drag_current_width: Option<f32>,
    _subscriptions: Vec<Subscription>,
}

enum ConnectionSecretRead {
    Bundle(Task<anyhow::Result<Option<String>>>),
    Legacy(Vec<(&'static str, Task<anyhow::Result<Option<String>>>)>),
}

fn merge_legacy_secret(
    secrets: &mut ConnectionSecrets,
    kind: &str,
    value: String,
) -> anyhow::Result<()> {
    match kind {
        "uri" => secrets.uri.password = Some(value),
        "uri-query" => {
            let query: UriSecrets = serde_json::from_str(&value)?;
            secrets.uri.tls_certificate_key_file_password = query.tls_certificate_key_file_password;
            secrets.uri.proxy_password = query.proxy_password;
            secrets.uri.aws_session_token = query.aws_session_token;
        }
        "ssh" => secrets.ssh_password = Some(value),
        "ssh-passphrase" => secrets.ssh_identity_passphrase = Some(value),
        "proxy" => secrets.proxy_password = Some(value),
        _ => {}
    }
    Ok(())
}

impl AppRoot {
    fn hydrate_connection_secrets(state: Entity<AppState>, cx: &mut App) {
        if !state.read(cx).connection_secrets_ready() {
            return;
        }
        let legacy_dev = match KeyStore::read_legacy_dev_credentials() {
            Ok(credentials) => credentials.unwrap_or_default(),
            Err(error) => {
                state.update(cx, |state, cx| {
                    state.finish_connection_secret_hydration(Err(error), cx);
                });
                return;
            }
        };
        let provider = state.read(cx).settings.ai.provider.keystore_id().to_string();
        let api_read = KeyStore::read(cx, &provider);
        let legacy_api_reads: Vec<_> = legacy_dev
            .iter()
            .filter(|(key, _)| !key.starts_with("conn."))
            .map(|(key, value)| (key.clone(), value.clone(), KeyStore::read(cx, key)))
            .collect();
        let mut reads = Vec::new();
        for connection in state.read(cx).connections.iter().cloned() {
            let source = if let Some(secret_id) = connection.secret_id {
                ConnectionSecretRead::Bundle(KeyStore::read_conn(
                    cx,
                    connection.id,
                    &connection_secret_bundle_key(secret_id),
                ))
            } else {
                ConnectionSecretRead::Legacy(
                    LEGACY_CONNECTION_SECRET_KEYS
                        .iter()
                        .map(|key| (*key, KeyStore::read_conn(cx, connection.id, key)))
                        .collect(),
                )
            };
            reads.push((connection, source));
        }
        state.update(cx, |state, _| state.begin_connection_secret_migration());

        cx.spawn(async move |cx: &mut AsyncApp| {
            let result: anyhow::Result<_> = async {
                let mut hydrated = Vec::with_capacity(reads.len());
                let mut migrated_ids = Vec::new();
                for (mut connection, source) in reads {
                    let secrets = match source {
                        ConnectionSecretRead::Bundle(task) => {
                            let payload = task.await?.ok_or_else(|| {
                                anyhow::anyhow!(
                                    "Credential bundle is missing for connection {}",
                                    connection.name
                                )
                            })?;
                            serde_json::from_str::<ConnectionSecrets>(&payload)?
                        }
                        ConnectionSecretRead::Legacy(tasks) => {
                            let mut secrets = ConnectionSecrets::from_connection(&connection);
                            for (kind, task) in tasks {
                                match task.await? {
                                    Some(value) => {
                                        merge_legacy_secret(&mut secrets, kind, value)?;
                                    }
                                    None => {
                                        if let Some(value) = legacy_dev
                                            .get(&format!("conn.{}.{kind}", connection.id))
                                        {
                                            merge_legacy_secret(&mut secrets, kind, value.clone())?;
                                        }
                                    }
                                }
                            }
                            connection.secret_id = Some(Uuid::new_v4());
                            migrated_ids.push(connection.id);
                            secrets
                        }
                    };
                    secrets.apply_to(&mut connection);
                    hydrated.push(connection);
                }

                let stored_api_key = api_read.await?;
                let api_key = stored_api_key.clone().or_else(|| legacy_dev.get(&provider).cloned());
                let mut legacy_api_writes = Vec::new();
                for (legacy_provider, value, read) in legacy_api_reads {
                    if read.await?.is_none() {
                        legacy_api_writes.push((legacy_provider, value));
                    }
                }
                let mut bundle_payloads = Vec::new();
                for connection in &hydrated {
                    if migrated_ids.contains(&connection.id) {
                        let secret_id = connection.secret_id.expect("migration assigned secret id");
                        bundle_payloads.push((
                            connection.id,
                            connection_secret_bundle_key(secret_id),
                            serde_json::to_string(&ConnectionSecrets::from_connection(connection))?,
                        ));
                    }
                }
                let writes = cx.update(|cx| {
                    let mut writes: Vec<Task<anyhow::Result<()>>> = bundle_payloads
                        .iter()
                        .map(|(id, key, payload)| KeyStore::write_conn(cx, *id, key, payload))
                        .collect();
                    for (legacy_provider, api_key) in &legacy_api_writes {
                        writes.push(KeyStore::write(cx, legacy_provider, api_key));
                    }
                    writes
                })?;
                let mut write_failure = None;
                for write in writes {
                    if let Err(error) = write.await
                        && write_failure.is_none()
                    {
                        write_failure = Some(error);
                    }
                }
                if let Some(error) = write_failure {
                    let cleanups = cx.update(|cx| {
                        bundle_payloads
                            .iter()
                            .map(|(id, key, _)| KeyStore::delete_conn(cx, *id, key))
                            .collect::<Vec<_>>()
                    })?;
                    for cleanup in cleanups {
                        let _ = cleanup.await;
                    }
                    return Err(error);
                }
                let candidate_bundles = bundle_payloads
                    .iter()
                    .map(|(id, key, _)| (*id, key.clone()))
                    .collect::<Vec<_>>();
                Ok((hydrated, migrated_ids, candidate_bundles, api_key, !legacy_dev.is_empty()))
            }
            .await;

            let _ = cx.update(|cx| match result {
                Ok((hydrated, migrated_ids, candidate_bundles, api_key, had_legacy_dev)) => {
                    let completed = state.update(cx, |state, cx| {
                        state.complete_connection_secret_startup(hydrated, cx)
                    });
                    if !completed {
                        let cleanups: Vec<_> = candidate_bundles
                            .into_iter()
                            .map(|(id, key)| KeyStore::delete_conn(cx, id, &key))
                            .collect();
                        let state_for_cleanup = state.clone();
                        cx.spawn(async move |cx: &mut AsyncApp| {
                            let mut first_error = None;
                            for cleanup in cleanups {
                                if let Err(error) = cleanup.await
                                    && first_error.is_none()
                                {
                                    first_error = Some(error);
                                }
                            }
                            if let Some(error) = first_error {
                                let _ = cx.update(|cx| {
                                    state_for_cleanup.update(cx, |state, cx| {
                                        state.report_connection_secret_error(error, cx);
                                    });
                                });
                            }
                        })
                        .detach();
                        return;
                    }
                    if let Some(api_key) = api_key {
                        state.update(cx, |state, _| {
                            if state.settings.ai.api_key.is_empty() {
                                state.settings.ai.api_key = api_key;
                            }
                        });
                    }
                    let mut cleanups = Vec::new();
                    for id in migrated_ids {
                        for key in LEGACY_CONNECTION_SECRET_KEYS {
                            cleanups.push(KeyStore::delete_conn(cx, id, key));
                        }
                    }
                    let state_for_cleanup = state.clone();
                    cx.spawn(async move |cx: &mut AsyncApp| {
                        let mut cleanup_error = None;
                        for cleanup in cleanups {
                            if let Err(error) = cleanup.await
                                && cleanup_error.is_none()
                            {
                                cleanup_error = Some(error);
                            }
                        }
                        if had_legacy_dev
                            && let Err(error) = KeyStore::delete_legacy_dev_credentials()
                            && cleanup_error.is_none()
                        {
                            cleanup_error = Some(error);
                        }
                        if let Some(error) = cleanup_error {
                            let _ = cx.update(|cx| {
                                state_for_cleanup.update(cx, |state, cx| {
                                    state.report_connection_secret_error(error, cx);
                                });
                            });
                        }
                    })
                    .detach();
                }
                Err(error) => {
                    state.update(cx, |state, cx| {
                        state.finish_connection_secret_hydration(Err(error), cx);
                    });
                }
            });
        })
        .detach();
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Create the app state entity
        let state = cx.new(|_| AppState::new());

        Self::hydrate_connection_secrets(state.clone(), cx);

        // Create sidebar with state reference
        let sidebar = cx.new(|cx| Sidebar::new(state.clone(), window, cx));

        // Create content area with state reference
        let content_area = cx.new(|cx| ContentArea::new(state.clone(), cx));
        let ai_view = cx.new(|cx| AiView::new(state.clone(), cx));

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
                let should_check =
                    cx.update(|cx| state.read(cx).settings.auto_update).unwrap_or(false);
                if should_check {
                    let _ = cx.update(|cx| {
                        AppCommands::check_for_updates(state.clone(), cx);
                    });
                }
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

        // Global keystroke observer: fallback for CloseTab when dispatch tree
        // loses track of focus after a tab close (stale FocusId → root_node_id(0)
        // → no "Workspace" context → keybinding doesn't match).
        let keystroke_sub = cx.observe_keystrokes(|this, event, window, cx| {
            let ks = &event.keystroke;
            let is_close = ks.key == "w"
                && (ks.modifiers.platform || ks.modifiers.control)
                && !ks.modifiers.alt
                && !ks.modifiers.shift;
            if is_close && event.action.is_none() {
                this.handle_close_tab(window, cx);
                window.focus(&this.focus_handle);
            }
        });
        subscriptions.push(keystroke_sub);

        let focus_handle = cx.focus_handle();

        Self {
            state,
            focus_handle,
            sidebar,
            content_area,
            ai_view,
            action_bar,
            key_debug,
            last_keystroke: None,
            sidebar_dragging: false,
            sidebar_drag_start_x: px(0.0),
            sidebar_drag_start_width: px(0.0),
            ai_dragging: false,
            ai_drag_start_x: px(0.0),
            ai_drag_start_width: AI_ISLAND_DEFAULT_WIDTH,
            ai_drag_current_width: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn flush_workspace_on_shutdown(&mut self, cx: &mut App) {
        self.state.update(cx, |state, _cx| {
            state.update_workspace_from_state();
            state.flush_workspace_now();
        });
    }

    pub fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        request_app_quit(self.state.clone(), window, cx);
    }

    pub fn close_all_editor_windows(&self, cx: &mut App) {
        let sessions = self.state.read(cx).editor_sessions();
        for handle in sessions.all_window_handles() {
            handle.update(cx, |_, window, _cx| window.remove_window()).ok();
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
        let appearance = state.settings.appearance.clone();
        let show_ai_island = state.ai_chat.panel_open && state.ai_assistant_available();
        let persisted_ai_panel_width = state
            .workspace
            .ai_panel_width
            .unwrap_or(AI_ISLAND_DEFAULT_WIDTH)
            .clamp(AI_ISLAND_MIN_WIDTH, AI_ISLAND_MAX_WIDTH);
        let ai_panel_width = self
            .ai_drag_current_width
            .unwrap_or(persisted_ai_panel_width)
            .clamp(AI_ISLAND_MIN_WIDTH, AI_ISLAND_MAX_WIDTH);
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
                    Some(CollectionSubview::Schema) => key_context.push_str(" Schema"),
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
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .relative()
            .when(vibrancy, |s| s.pt(px(28.0)))
            .bg(islands::canvas_bg(&appearance, cx))
            .border_1()
            .border_color(islands::panel_border(&appearance, cx))
            .rounded(islands::radius_md(&appearance))
            .text_color(cx.theme().foreground)
            .font_family(crate::theme::fonts::ui())
            .line_height(crate::theme::fonts::ui_line_height())
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.handle_close_tab(window, cx);
                window.focus(&this.focus_handle);
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
            .on_action(cx.listener(|this, _: &QuitApp, window, cx| {
                this.request_quit(window, cx);
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
                        AppCommands::drop_database(
                            state.clone(),
                            connection_id,
                            database.clone(),
                            cx,
                        );
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
                            move |window, cx| {
                                request_remove_connection(state.clone(), connection_id, window, cx);
                            }
                        },
                    );
                }
            }))
            .on_action(cx.listener(|this, _: &DisconnectConnection, window, cx| {
                if let Some(connection_id) = this.state.read(cx).selected_connection_id() {
                    request_disconnect_connection(this.state.clone(), connection_id, window, cx);
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
            .on_action(cx.listener(|this, _: &RefreshView, window, cx| {
                this.handle_refresh(window, cx);
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
            .on_action(cx.listener(|this, _: &ToggleAiPanel, window, cx| {
                let opened = this.state.update(cx, |state, cx| state.toggle_ai_panel(cx));
                if opened {
                    this.ai_view.update(cx, |view, cx| view.focus_input(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &OpenForge, _window, cx| {
                this.state.update(cx, |state, cx| {
                    let Some(key) = state.current_database_key() else {
                        return;
                    };
                    let collection = state.selected_collection_name();
                    state.open_forge_tab(key.connection_id, key.database, collection, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &FocusSidebar, window, cx| {
                this.sidebar.update(cx, |sidebar, cx| {
                    if sidebar.is_collapsed() {
                        sidebar.toggle_collapsed();
                        cx.notify();
                    }
                    window.focus(&sidebar.focus_handle);
                });
            }))
            .on_action(cx.listener(|this, _: &FocusContent, window, cx| {
                let focused_content = this
                    .content_area
                    .update(cx, |content, cx| content.focus_current_view(window, cx));
                if !focused_content {
                    window.focus(&this.focus_handle);
                }
            }))
            .on_action(cx.listener(|this, _: &DownloadUpdate, _window, cx| {
                AppCommands::download_update(this.state.clone(), cx);
            }))
            .on_action(cx.listener(|this, _: &InstallUpdate, _window, cx| {
                AppCommands::install_update(this.state.clone(), cx);
            }))
            .child({
                let is_dragging = self.sidebar_dragging;
                let is_ai_dragging = self.ai_dragging;
                let sidebar_collapsed = self.sidebar.read(cx).width() == px(0.0);

                let resize_handle = div()
                    .id("sidebar-resize-handle")
                    .flex_shrink_0()
                    .w(px(6.0))
                    .h_full()
                    .cursor_col_resize()
                    .bg(crate::theme::colors::transparent())
                    .my(px(10.0))
                    .rounded(px(999.0))
                    .hover(|s| s.bg(islands::panel_border(&appearance, cx).opacity(0.7)))
                    .tooltip(|window, cx| {
                        Tooltip::new("Drag to resize sidebar. Double-click to hide or show.")
                            .build(window, cx)
                    })
                    .when(is_dragging, |s: Stateful<Div>| {
                        s.bg(islands::panel_border(&appearance, cx).opacity(0.9))
                    })
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

                let sidebar_panel = div()
                    .h_full()
                    .overflow_hidden()
                    .rounded(islands::radius_md(&appearance))
                    .border_1()
                    .border_color(islands::panel_border(&appearance, cx))
                    .bg(islands::tool_bg(&appearance, cx))
                    .when(sidebar_collapsed, |s| s.w(px(0.0)).border_0())
                    .child(self.sidebar.clone())
                    .into_any_element();

                let content_panel = div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .rounded(islands::radius_md(&appearance))
                    .border_1()
                    .border_color(islands::panel_border(&appearance, cx))
                    .bg(islands::content_bg(&appearance, cx))
                    .child(self.content_area.clone())
                    .into_any_element();

                let ai_resize_handle = div()
                    .id("ai-resize-handle")
                    .flex_shrink_0()
                    .w(px(6.0))
                    .h_full()
                    .cursor_col_resize()
                    .bg(crate::theme::colors::transparent())
                    .my(px(10.0))
                    .rounded(px(999.0))
                    .hover(|s| s.bg(islands::panel_border(&appearance, cx).opacity(0.7)))
                    .when(is_ai_dragging, |s: Stateful<Div>| {
                        s.bg(islands::panel_border(&appearance, cx).opacity(0.9))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                            this.ai_dragging = true;
                            this.ai_drag_start_x = event.position.x;
                            this.ai_drag_start_width = ai_panel_width;
                            this.ai_drag_current_width = Some(ai_panel_width);
                            cx.notify();
                        }),
                    );

                let ai_panel = div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .w(px(ai_panel_width))
                    .min_w(px(ai_panel_width))
                    .max_w(px(ai_panel_width))
                    .h_full()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .rounded(islands::radius_md(&appearance))
                    .border_1()
                    .border_color(islands::panel_border(&appearance, cx))
                    .bg(islands::ai_shell_bg(&appearance, cx))
                    .child(self.ai_view.clone())
                    .into_any_element();

                let mut row = div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .px(spacing::xs())
                    .py(spacing::xs())
                    .child(sidebar_panel)
                    .child(resize_handle)
                    .child(content_panel)
                    .children(show_ai_island.then(|| ai_resize_handle.into_any_element()))
                    .children(show_ai_island.then_some(ai_panel));

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

                if is_ai_dragging {
                    row = row
                        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                            let delta = event.position.x - this.ai_drag_start_x;
                            let new_width = (this.ai_drag_start_width - f32::from(delta))
                                .clamp(AI_ISLAND_MIN_WIDTH, AI_ISLAND_MAX_WIDTH);
                            this.ai_drag_current_width = Some(new_width);
                            cx.notify();
                        }))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                                let final_width = this.ai_drag_current_width.take();
                                this.ai_dragging = false;
                                if let Some(width) = final_width {
                                    this.state.update(cx, |state, _cx| {
                                        state.set_workspace_ai_panel_width(width);
                                    });
                                }
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
                .ai_state(
                    self.state.read(cx).ai_assistant_available(),
                    self.state.read(cx).ai_chat.panel_open,
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
