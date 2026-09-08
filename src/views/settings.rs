//! Settings view for application configuration.

mod keybindings;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage, Settings};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Icon, IconName, Sizable as _, Size};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ai::bridge::AiBridge;
use crate::ai::model_registry::{self, ModelCache};
use crate::ai::provider::{AiGenerationRequest, generate_text};
use crate::components::{Button, open_confirm_dialog, request_app_quit};
use crate::state::settings::CollectionDoubleClickAction;
use crate::state::{
    AiProvider, AppCommands, AppSettings, AppState, AppTheme, DEFAULT_FILENAME_TEMPLATE,
    FILENAME_PLACEHOLDERS, InsertMode, McpClientKind, TransferFormat,
};
use crate::theme::{borders, islands, spacing};

use self::keybindings::KeybindingsView;

#[derive(Debug, Clone)]
enum AiTestResult {
    Success(String),
    Error(String),
}

pub struct SettingsView {
    state: Entity<AppState>,
    _subscriptions: Vec<Subscription>,
    keybindings_view: Entity<KeybindingsView>,
    // Input states (lazily initialized)
    template_input_state: Option<Entity<InputState>>,
    batch_size_input_state: Option<Entity<InputState>>,
    query_timeout_input_state: Option<Entity<InputState>>,
    ai_api_key_input_state: Option<Entity<InputState>>,
    ai_ollama_base_url_input_state: Option<Entity<InputState>>,
    ai_test_in_flight: bool,
    ai_test_result: Option<AiTestResult>,
    last_seen_provider: AiProvider,
}

impl SettingsView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let last_seen_provider = state.read(cx).settings.ai.provider;
        let keybindings_view = cx.new(|cx| KeybindingsView::new(state.clone(), cx));
        model_registry::spawn_model_fetch(&state, cx);
        let subscriptions = vec![cx.observe(&state, |this: &mut Self, _, cx| {
            let current = this.state.read(cx).settings.ai.provider;
            if current != this.last_seen_provider {
                this.last_seen_provider = current;
                model_registry::spawn_model_fetch(&this.state, cx);
            }
            cx.notify();
        })];

        Self {
            state,
            _subscriptions: subscriptions,
            keybindings_view,
            template_input_state: None,
            batch_size_input_state: None,
            query_timeout_input_state: None,
            ai_api_key_input_state: None,
            ai_ollama_base_url_input_state: None,
            ai_test_in_flight: false,
            ai_test_result: None,
            last_seen_provider,
        }
    }

    /// Initialize input states on first render (when window is available)
    fn ensure_input_states(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.template_input_state.is_some()
            && self.batch_size_input_state.is_some()
            && self.query_timeout_input_state.is_some()
            && self.ai_api_key_input_state.is_some()
            && self.ai_ollama_base_url_input_state.is_some()
        {
            return; // Already initialized
        }

        let template = self.state.read(cx).settings.transfer.export_filename_template.clone();
        let batch_size = self.state.read(cx).settings.transfer.default_batch_size;
        let query_timeout = self.state.read(cx).settings.interactive_query_timeout_ms;
        let ai_api_key = self.state.read(cx).settings.ai.api_key.clone();
        let ai_ollama_base_url = self.state.read(cx).settings.ai.ollama_base_url.clone();

        let template_input_state = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder("${database}_${collection}_${datetime}")
                .clean_on_escape();
            state.set_value(template, window, cx);
            state
        });

        // Subscribe to template input changes
        let state_for_template_sub = self.state.clone();
        let template_sub = cx.subscribe_in(
            &template_input_state,
            window,
            move |_view, state, event, _window, cx| {
                if let InputEvent::Change = event {
                    let new_text = state.read(cx).value().to_string();
                    state_for_template_sub.update(cx, |app_state, cx| {
                        app_state.settings.transfer.export_filename_template = new_text;
                        app_state.save_settings();
                        cx.notify();
                    });
                }
            },
        );
        self._subscriptions.push(template_sub);

        let batch_size_input_state = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("1000").clean_on_escape();
            state.set_value(batch_size.to_string(), window, cx);
            state
        });

        // Subscribe to batch size input changes
        let state_for_batch_sub = self.state.clone();
        let batch_sub = cx.subscribe_in(
            &batch_size_input_state,
            window,
            move |_view, state, event, _window, cx| {
                if let InputEvent::Change = event {
                    let new_text = state.read(cx).value().to_string();
                    if let Ok(value) = new_text.parse::<u32>() {
                        state_for_batch_sub.update(cx, |app_state, cx| {
                            app_state.settings.transfer.default_batch_size = value.clamp(1, 100000);
                            app_state.save_settings();
                            cx.notify();
                        });
                    }
                }
            },
        );
        self._subscriptions.push(batch_sub);

        let query_timeout_input_state = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("30000").clean_on_escape();
            state.set_value(query_timeout.to_string(), window, cx);
            state
        });
        let state_for_timeout_sub = self.state.clone();
        let timeout_sub = cx.subscribe_in(
            &query_timeout_input_state,
            window,
            move |_view, state, event, _window, cx| {
                if let InputEvent::Change = event {
                    let new_text = state.read(cx).value().to_string();
                    if let Ok(value) = new_text.parse::<u64>() {
                        state_for_timeout_sub.update(cx, |app_state, cx| {
                            app_state.settings.interactive_query_timeout_ms =
                                value.clamp(100, 3_600_000);
                            app_state.save_settings();
                            cx.notify();
                        });
                    }
                }
            },
        );
        self._subscriptions.push(timeout_sub);

        let ai_api_key_input_state = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder("API key (or use env var)").masked(true);
            state.set_value(ai_api_key, window, cx);
            state
        });
        let state_for_key_sub = self.state.clone();
        let ai_key_sub = cx.subscribe_in(
            &ai_api_key_input_state,
            window,
            move |_view, state, event, _window, cx| {
                if let InputEvent::Change = event {
                    let value = state.read(cx).value().to_string();
                    state_for_key_sub.update(cx, |app_state, cx| {
                        app_state.settings.ai.set_api_key(value, cx);
                        app_state.save_settings();
                        cx.notify();
                    });
                }
            },
        );
        self._subscriptions.push(ai_key_sub);

        let ai_ollama_base_url_input_state = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("http://localhost:11434");
            state.set_value(ai_ollama_base_url, window, cx);
            state
        });
        let state_for_ollama_sub = self.state.clone();
        let ai_ollama_sub = cx.subscribe_in(
            &ai_ollama_base_url_input_state,
            window,
            move |_view, state, event, _window, cx| {
                if let InputEvent::Change = event {
                    let value = state.read(cx).value().to_string();
                    state_for_ollama_sub.update(cx, |app_state, cx| {
                        app_state.settings.ai.set_ollama_base_url(value);
                        app_state.save_settings();
                        cx.notify();
                    });
                }
            },
        );
        self._subscriptions.push(ai_ollama_sub);

        self.template_input_state = Some(template_input_state);
        self.batch_size_input_state = Some(batch_size_input_state);
        self.query_timeout_input_state = Some(query_timeout_input_state);
        self.ai_api_key_input_state = Some(ai_api_key_input_state);
        self.ai_ollama_base_url_input_state = Some(ai_ollama_base_url_input_state);
    }

    fn sync_ai_inputs_from_settings(&self, window: &mut Window, cx: &mut App) {
        let ai_settings = self.state.read(cx).settings.ai.clone();

        if let Some(api_key_state) = self.ai_api_key_input_state.clone() {
            api_key_state.update(cx, |state, cx| {
                state.set_value(ai_settings.api_key.clone(), window, cx);
            });
        }
        if let Some(base_url_state) = self.ai_ollama_base_url_input_state.clone() {
            base_url_state.update(cx, |state, cx| {
                state.set_value(ai_settings.ollama_base_url.clone(), window, cx);
            });
        }
    }

    fn start_ai_test(&mut self, cx: &mut Context<Self>) {
        if self.ai_test_in_flight {
            return;
        }

        self.ai_test_in_flight = true;
        self.ai_test_result = None;

        let settings = self.state.read(cx).settings.ai.clone();
        let view = cx.entity();
        let task = cx.background_spawn(async move {
            AiBridge::block_on(async move {
                let request = AiGenerationRequest {
                    system_prompt: "You are a health-check assistant. Respond briefly.".to_string(),
                    history: Vec::new(),
                    user_prompt: "Return exactly: AI test passed.".to_string(),
                };
                generate_text(&settings, request).await
            })
        });

        cx.spawn(async move |_view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            cx.update(|cx| {
                view.update(cx, |this, cx| {
                    this.ai_test_in_flight = false;
                    this.ai_test_result = Some(match result {
                        Ok(message) => AiTestResult::Success(message),
                        Err(error) => AiTestResult::Error(error.user_message()),
                    });
                    cx.notify();
                });
            });
        })
        .detach();
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_input_states(window, cx);

        let view = cx.entity();
        let state = self.state.clone();
        let appearance = self.state.read(cx).settings.appearance.clone();

        let general_state = state.clone();
        let query_timeout_input = self.query_timeout_input_state.clone().unwrap();
        let general = SettingPage::new("General")
            .icon(Icon::new(IconName::Settings2))
            .description("Appearance, query behavior, updates, and support.")
            .resettable(false)
            .group(
                SettingGroup::new().item(
                    SettingItem::render(move |_, _, cx| {
                        let settings = general_state.read(cx).settings.clone();
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(spacing::lg())
                            .child(render_appearance_section(general_state.clone(), &settings, cx))
                            .child(render_query_section(
                                general_state.clone(),
                                &settings,
                                query_timeout_input.clone(),
                                cx,
                            ))
                            .child(render_updates_section(general_state.clone(), &settings, cx))
                            .child(render_support_section(general_state.clone(), cx))
                    })
                    .keywords([
                        "theme",
                        "appearance",
                        "vibrancy",
                        "status bar",
                        "query timeout",
                        "collection",
                        "double click",
                        "forge",
                        "updates",
                        "support",
                        "diagnostics",
                        "logs",
                    ]),
                ),
            );

        let transfer_state = state.clone();
        let template_input = self.template_input_state.clone().unwrap();
        let batch_size_input = self.batch_size_input_state.clone().unwrap();
        let transfer = SettingPage::new("Transfer")
            .icon(Icon::new(crate::assets::AppIcon::Download))
            .description("Defaults used when importing and exporting data.")
            .resettable(false)
            .group(
                SettingGroup::new().item(
                    SettingItem::render(move |_, _, cx| {
                        let settings = transfer_state.read(cx).settings.clone();
                        div().w_full().child(render_transfer_section(
                            transfer_state.clone(),
                            &settings,
                            template_input.clone(),
                            batch_size_input.clone(),
                            cx,
                        ))
                    })
                    .keywords([
                        "export",
                        "import",
                        "format",
                        "folder",
                        "filename",
                        "template",
                        "batch size",
                        "json",
                        "csv",
                        "bson",
                    ]),
                ),
            );

        let ai_state = state.clone();
        let ai_view = view.clone();
        let ai_ui = AiSectionUiState {
            api_key_input_state: self.ai_api_key_input_state.clone().unwrap(),
            ollama_base_url_input_state: self.ai_ollama_base_url_input_state.clone().unwrap(),
            ai_test_in_flight: self.ai_test_in_flight,
            ai_test_result: self.ai_test_result.clone(),
        };
        let ai = SettingPage::new("AI Assistant")
            .icon(Icon::new(IconName::Bot))
            .description("Provider, privacy, credentials, and diagnostics.")
            .resettable(false)
            .group(
                SettingGroup::new().item(
                    SettingItem::render(move |_, _, cx| {
                        let settings = ai_state.read(cx).settings.clone();
                        div().w_full().child(render_ai_section(
                            ai_view.clone(),
                            ai_state.clone(),
                            &settings,
                            ai_ui.clone(),
                            cx,
                        ))
                    })
                    .keywords([
                        "ai",
                        "assistant",
                        "provider",
                        "model",
                        "privacy",
                        "documents",
                        "api key",
                        "ollama",
                        "openai",
                        "anthropic",
                        "gemini",
                        "test provider",
                    ]),
                ),
            );

        let agents_state = state.clone();
        let agents = SettingPage::new("Agents & MCP")
            .icon(Icon::new(IconName::Eye))
            .description("Trusted local clients and connection-level permissions.")
            .resettable(false)
            .group(
                SettingGroup::new().item(
                    SettingItem::render(move |_, _, cx| {
                        let settings = agents_state.read(cx).settings.clone();
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(spacing::lg())
                            .child(render_mcp_section(agents_state.clone(), &settings, cx))
                            .child(render_mcp_grants_section(agents_state.clone(), &settings, cx))
                            .child(render_agent_connections_section(
                                agents_state.clone(),
                                &settings,
                                cx,
                            ))
                    })
                    .keywords([
                        "agents",
                        "mcp",
                        "clients",
                        "tokens",
                        "connections",
                        "permissions",
                        "writes",
                        "history",
                        "keychain",
                        "codex",
                        "claude",
                        "cursor",
                        "vscode",
                    ]),
                ),
            );

        let keybindings = self.keybindings_view.clone();
        let keybindings_page = SettingPage::new("Keybindings")
            .icon(Icon::new(IconName::SquareTerminal))
            .description("Search commands and customize keyboard shortcuts.")
            .resettable(false)
            .group(SettingGroup::new().item(
                SettingItem::render(move |_, _, _| keybindings.clone()).keywords([
                    "keyboard",
                    "keybindings",
                    "shortcuts",
                    "commands",
                    "hotkeys",
                ]),
            ));

        div()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(islands::content_bg(&appearance, cx))
            .child(
                Settings::new("openmango-settings")
                    .sidebar_width(px(220.0))
                    .with_group_variant(GroupBoxVariant::Normal)
                    .pages([general, transfer, ai, agents, keybindings_page]),
            )
    }
}

fn render_appearance_section(
    state: Entity<AppState>,
    settings: &AppSettings,
    cx: &App,
) -> impl IntoElement {
    let current_theme = settings.appearance.theme;
    let show_status_bar = settings.appearance.show_status_bar;

    // Theme dropdown
    let theme_dropdown = {
        let state = state.clone();
        gpui_kit::component::button::Button::new("theme-dropdown")
            .xsmall()
            .label(current_theme.label())
            .dropdown_caret(true)
            .rounded(borders::radius_sm())
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu: PopupMenu, _window, _cx| {
                let mut m = menu;
                // Dark themes section
                m = m.label("Dark");
                for theme in AppTheme::dark_themes() {
                    let s = state.clone();
                    let t = *theme;
                    m = m.item(PopupMenuItem::new(theme.label()).on_click(move |_, window, cx| {
                        s.update(cx, |state, cx| {
                            state.settings.appearance.theme = t;
                            state.save_settings();
                            cx.notify();
                        });
                        let (user_vibrancy, startup_vibrancy) = {
                            let state_ref = s.read(cx);
                            (state_ref.settings.appearance.vibrancy, state_ref.startup_vibrancy)
                        };
                        let target_vibrancy = crate::theme::effective_vibrancy(t, user_vibrancy);
                        crate::theme::apply_theme(t, target_vibrancy, window, cx);
                        if crate::theme::requires_vibrancy_restart(
                            startup_vibrancy,
                            t,
                            user_vibrancy,
                        ) {
                            crate::components::open_confirm_dialog(
                                window,
                                cx,
                                "Restart required",
                                "Switching this theme changes window vibrancy mode. Restart now to fully apply it.",
                                "Restart now",
                                false,
                                {
                                    let state = s.clone();
                                    move |window, cx| {
                                        request_app_quit(state.clone(), window, cx);
                                    }
                                },
                            );
                        }
                    }));
                }
                // Light themes section (when available)
                let light = AppTheme::light_themes();
                if !light.is_empty() {
                    m = m.separator().label("Light");
                    for theme in light {
                        let s = state.clone();
                        let t = *theme;
                        m = m.item(PopupMenuItem::new(theme.label()).on_click(
                            move |_, window, cx| {
                                s.update(cx, |state, cx| {
                                    state.settings.appearance.theme = t;
                                    state.save_settings();
                                    cx.notify();
                                });
                                let (user_vibrancy, startup_vibrancy) = {
                                    let state_ref = s.read(cx);
                                    (state_ref.settings.appearance.vibrancy, state_ref.startup_vibrancy)
                                };
                                let target_vibrancy =
                                    crate::theme::effective_vibrancy(t, user_vibrancy);
                                crate::theme::apply_theme(t, target_vibrancy, window, cx);
                                if crate::theme::requires_vibrancy_restart(
                                    startup_vibrancy,
                                    t,
                                    user_vibrancy,
                                ) {
                                    crate::components::open_confirm_dialog(
                                        window,
                                        cx,
                                        "Restart required",
                                        "Switching this theme changes window vibrancy mode. Restart now to fully apply it.",
                                        "Restart now",
                                        false,
                                        {
                                            let state = s.clone();
                                            move |window, cx| {
                                                request_app_quit(state.clone(), window, cx);
                                            }
                                        },
                                    );
                                }
                            },
                        ));
                    }
                }
                m
            })
    };

    // Status bar toggle
    let status_bar_checkbox = {
        let state = state.clone();
        let checked = show_status_bar;
        gpui_kit::component::checkbox::Checkbox::new("show-status-bar").checked(checked).on_click(
            move |_, _, cx| {
                state.update(cx, |state, cx| {
                    state.settings.appearance.show_status_bar = !checked;
                    state.save_settings();
                    cx.notify();
                });
            },
        )
    };

    // Vibrancy toggle
    let vibrancy_checkbox = {
        let state = state.clone();
        let checked = settings.appearance.vibrancy;
        gpui_kit::component::checkbox::Checkbox::new("vibrancy").checked(checked).on_click(
            move |_, window, cx| {
                state.update(cx, |state, cx| {
                    state.settings.appearance.vibrancy = !checked;
                    state.save_settings();
                    cx.notify();
                });
                crate::components::open_confirm_dialog(
                    window,
                    cx,
                    "Restart required",
                    "Vibrancy changes require a restart to take effect.",
                    "Restart now",
                    false,
                    {
                        let state = state.clone();
                        move |window, cx| request_app_quit(state.clone(), window, cx)
                    },
                );
            },
        )
    };

    section(
        "Appearance",
        div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .child(setting_row("Theme", theme_dropdown, cx))
            .child(setting_row_with_description(
                "Show status bar",
                "Display the status bar at the bottom of the window",
                status_bar_checkbox,
                cx,
            ))
            .child(setting_row_with_description(
                "Vibrancy",
                "Blurred transparent window background (restart required)",
                vibrancy_checkbox,
                cx,
            )),
        cx,
    )
}

fn render_query_section(
    state: Entity<AppState>,
    settings: &AppSettings,
    query_timeout_input_state: Entity<InputState>,
    cx: &App,
) -> impl IntoElement {
    let timeout_input = NumberInput::new(&query_timeout_input_state).small().w(px(120.0));
    let double_click_action =
        gpui_kit::component::button::Button::new("collection-double-click-action")
            .xsmall()
            .label(settings.collection_double_click_action.label())
            .dropdown_caret(true)
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu: PopupMenu, _, _| {
                for action in
                    [CollectionDoubleClickAction::Data, CollectionDoubleClickAction::Forge]
                {
                    let state = state.clone();
                    menu =
                        menu.item(PopupMenuItem::new(action.label()).on_click(move |_, _, cx| {
                            state.update(cx, |state, cx| {
                                state.settings.collection_double_click_action = action;
                                state.save_settings();
                                cx.notify();
                            });
                        }));
                }
                menu
            });
    section(
        "Queries",
        div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .child(setting_row_with_description(
                "Interactive query timeout (ms)",
                "Server maxTimeMS for document count and find commands (100–3,600,000)",
                timeout_input,
                cx,
            ))
            .child(setting_row_with_description(
                "Collection double-click action",
                "Forge opens a find-all query for the collection, ready to run.",
                double_click_action,
                cx,
            )),
        cx,
    )
}

fn render_updates_section(
    state: Entity<AppState>,
    settings: &AppSettings,
    cx: &App,
) -> impl IntoElement {
    let auto_update = settings.auto_update;

    let auto_update_checkbox = {
        let state = state.clone();
        gpui_kit::component::checkbox::Checkbox::new("auto-update").checked(auto_update).on_click(
            move |_, _, cx| {
                state.update(cx, |state, cx| {
                    state.settings.auto_update = !auto_update;
                    state.save_settings();
                    cx.notify();
                });
            },
        )
    };

    section(
        "Updates",
        div().flex().flex_col().gap(spacing::md()).child(setting_row_with_description(
            "Automatic updates",
            "Automatically check for and download updates; restart to install",
            auto_update_checkbox,
            cx,
        )),
        cx,
    )
}

fn render_support_section(state: Entity<AppState>, cx: &App) -> impl IntoElement {
    let log_path = crate::helpers::support::app_log_path();
    let export_button = Button::new("export-support-bundle")
        .xsmall()
        .label("Export Support Bundle...")
        .on_click(move |_, _, cx| {
            let state = state.clone();
            cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
                let path = crate::components::file_picker::open_file_dialog_async(
                    crate::components::file_picker::FilePickerMode::Save,
                    vec![crate::components::file_picker::FileFilter::new(
                        "Support bundle",
                        vec!["txt"],
                    )],
                    Some("OpenMango-support.txt".to_string()),
                )
                .await;
                let Some(path) = path else {
                    return;
                };
                let result = cx.update(|cx| {
                    crate::helpers::support::export_support_bundle(state.read(cx), &path)
                });
                cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        match result {
                            Ok(()) => {
                                state.set_status_message(Some(crate::state::StatusMessage::info(
                                    format!("Support bundle exported to {}", path.display()),
                                )))
                            }
                            Err(error) => {
                                state.set_status_message(Some(crate::state::StatusMessage::error(
                                    format!("Support bundle export failed: {error}"),
                                )))
                            }
                        }
                        cx.notify();
                    });
                });
            })
            .detach();
        });

    section(
        "Support",
        div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .child(setting_row_with_description(
                "Log file",
                &log_path.display().to_string(),
                div().text_xs().text_color(cx.theme().muted_foreground).child("On disk"),
                cx,
            ))
            .child(setting_row_with_description(
                "Diagnostics",
                "Exports redacted configuration, runtime details, and the recent log.",
                export_button,
                cx,
            )),
        cx,
    )
}

fn render_mcp_section(
    state: Entity<AppState>,
    settings: &AppSettings,
    cx: &App,
) -> impl IntoElement {
    let enabled = settings.mcp.enabled;
    let endpoint = if settings.mcp.port == 0 {
        "Assigned when enabled".to_string()
    } else {
        format!("http://127.0.0.1:{}/mcp", settings.mcp.port)
    };
    let toggle =
        Switch::new("mcp-enabled").checked(enabled).small().on_click({
            let state = state.clone();
            move |checked, _window, cx| {
                state.update(cx, |state, cx| {
                    state.settings.mcp.enabled = *checked;
                    state.save_settings();
                    state.set_status_message(Some(crate::state::StatusMessage::info(
                        if *checked { "Agent access enabled" } else { "Agent access disabled" },
                    )));
                    cx.notify();
                });
            }
        });
    let copy_endpoint = Button::new("copy-mcp-endpoint")
        .xsmall()
        .icon(Icon::new(IconName::Copy).xsmall())
        .label("Copy")
        .disabled(settings.mcp.port == 0)
        .on_click({
            let endpoint = endpoint.clone();
            move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(endpoint.clone()))
        });
    let appearance = &settings.appearance;

    group(
        "Local MCP server",
        div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .child(setting_row_with_description(
                "Allow agent clients",
                "Accept authenticated MCP connections while OpenMango is open.",
                toggle,
                cx,
            ))
            .child(
                div()
                    .border_t_1()
                    .border_color(islands::panel_border(appearance, cx))
                    .pt(spacing::md())
                    .child(setting_row_with_description(
                        "Listen address",
                        &endpoint,
                        copy_endpoint,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().size(px(7.0)).rounded_full().bg(if enabled {
                        cx.theme().success
                    } else {
                        cx.theme().muted_foreground
                    }))
                    .child(if enabled {
                        "Running locally · tokens stored in macOS Keychain · MCP 2026-07-28"
                    } else {
                        "Off · existing grants remain in macOS Keychain"
                    }),
            ),
        appearance,
        cx,
    )
}

fn render_mcp_grants_section(
    state: Entity<AppState>,
    settings: &AppSettings,
    cx: &App,
) -> impl IntoElement {
    let port = settings.mcp.port;
    let active_count = settings.mcp.grants.iter().filter(|grant| grant.active()).count();
    let create_button = {
        let state = state.clone();
        gpui_kit::component::button::Button::new("create-mcp-grant")
            .xsmall()
            .primary()
            .icon(Icon::new(IconName::Plus).xsmall())
            .label("Add client")
            .dropdown_caret(true)
            .rounded(borders::radius_sm())
            .with_size(Size::Small)
            .disabled(port == 0)
            .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu: PopupMenu, _window, _cx| {
                McpClientKind::ALL.into_iter().fold(menu, |menu, client| {
                    let state = state.clone();
                    menu.item(PopupMenuItem::new(client.label()).on_click(move |_, _, cx| {
                        create_mcp_client_grant(state.clone(), client, cx);
                    }))
                })
            })
    };

    let grant_ids = settings
        .mcp
        .grants
        .iter()
        .filter(|grant| grant.active())
        .map(|grant| grant.id)
        .collect::<Vec<_>>();
    let reset_button = Button::new("reset-mcp-access")
        .xsmall()
        .danger()
        .label("Revoke all clients")
        .disabled(!settings.mcp.legacy_access && grant_ids.is_empty())
        .on_click({
            let state = state.clone();
            move |_, window, cx| {
                let state = state.clone();
                let grant_ids = grant_ids.clone();
                open_confirm_dialog(
                    window,
                    cx,
                    "Revoke all agent clients",
                    "Disable agent access and revoke every client token? Connected clients will lose access immediately.",
                    "Revoke all clients",
                    true,
                    move |_window, cx| {
                        state.update(cx, |state, cx| {
                            state.settings.mcp.enabled = false;
                            state.settings.mcp.legacy_access = false;
                            let now = chrono::Utc::now();
                            for grant in &mut state.settings.mcp.grants {
                                if grant.active() {
                                    grant.revoked_at = Some(now);
                                }
                            }
                            state.save_settings();
                            cx.notify();
                        });
                        crate::helpers::keystore::KeyStore::delete_mcp_token(cx).detach();
                        for id in &grant_ids {
                            crate::helpers::keystore::KeyStore::delete_mcp_grant(cx, *id).detach();
                        }
                    },
                );
            }
        });

    let mut grants = div().flex().flex_col();
    if settings.mcp.legacy_access {
        let state_for_revoke = state.clone();
        grants = grants.child(
            mcp_grant_row(
                "Legacy local token",
                "Compatibility access used by the initial OpenMango MCP setup",
                div().flex().items_center().gap(spacing::xs()).child(
                    Button::new("revoke-legacy-mcp-grant")
                        .xsmall()
                        .danger()
                        .label("Revoke")
                        .on_click(move |_, window, cx| {
                            let state = state_for_revoke.clone();
                            open_confirm_dialog(
                                window,
                                cx,
                                "Revoke legacy MCP access",
                                "Clients using the original OpenMango token will disconnect. Create and copy a Pi grant first.",
                                "Revoke access",
                                true,
                                move |_window, cx| {
                                    state.update(cx, |state, cx| {
                                        state.settings.mcp.legacy_access = false;
                                        if !state.settings.mcp.grants.iter().any(|grant| grant.active()) {
                                            state.settings.mcp.enabled = false;
                                        }
                                        state.save_settings();
                                        cx.notify();
                                    });
                                    crate::helpers::keystore::KeyStore::delete_mcp_token(cx)
                                        .detach();
                                },
                            );
                        }),
                ),
                cx,
            ),
        );
    }
    for grant in &settings.mcp.grants {
        let id = grant.id;
        let client = grant.client;
        let active = grant.active();
        let state_for_token = state.clone();
        let state_for_revoke = state.clone();
        let state_for_remove = state.clone();
        let actions = if active {
            div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .child(
                    Button::new(("copy-mcp-grant", id.as_u128() as u64))
                        .xsmall()
                        .label("Copy config")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(mcp_client_config(
                                client, port, id,
                            )));
                        }),
                )
                .when(
                    matches!(
                        client,
                        McpClientKind::Codex | McpClientKind::Cursor | McpClientKind::VsCode
                    ),
                    |actions| {
                        actions.child(
                            Button::new(("copy-mcp-grant-token", id.as_u128() as u64))
                                .xsmall()
                                .label("Copy token")
                                .on_click(move |_, _, cx| {
                                    copy_mcp_grant_token(state_for_token.clone(), id, cx);
                                }),
                        )
                    },
                )
                .child(
                    Button::new(("revoke-mcp-grant", id.as_u128() as u64))
                        .xsmall()
                        .danger()
                        .label("Revoke")
                        .on_click(move |_, window, cx| {
                            let state = state_for_revoke.clone();
                            open_confirm_dialog(
                                window,
                                cx,
                                "Revoke MCP client grant",
                                "This client will lose access immediately.",
                                "Revoke grant",
                                true,
                                move |_window, cx| {
                                    state.update(cx, |state, cx| {
                                        if let Some(grant) = state
                                            .settings
                                            .mcp
                                            .grants
                                            .iter_mut()
                                            .find(|grant| grant.id == id)
                                        {
                                            grant.revoked_at = Some(chrono::Utc::now());
                                        }
                                        if !state.settings.mcp.legacy_access
                                            && !state
                                                .settings
                                                .mcp
                                                .grants
                                                .iter()
                                                .any(|grant| grant.active())
                                        {
                                            state.settings.mcp.enabled = false;
                                        }
                                        state.save_settings();
                                        cx.notify();
                                    });
                                    crate::helpers::keystore::KeyStore::delete_mcp_grant(cx, id)
                                        .detach();
                                },
                            );
                        }),
                )
                .into_any_element()
        } else {
            Button::new(("remove-mcp-grant", id.as_u128() as u64))
                .xsmall()
                .danger()
                .label("Remove")
                .on_click(move |_, _, cx| {
                    state_for_remove.update(cx, |state, cx| {
                        state.settings.mcp.grants.retain(|grant| grant.id != id);
                        state.save_settings();
                        cx.notify();
                    });
                    crate::helpers::keystore::KeyStore::delete_mcp_grant(cx, id).detach();
                })
                .into_any_element()
        };
        grants = grants.child(mcp_grant_row(
            &grant.label,
            &format!(
                "Created {} · {} · {} · {} · {}",
                grant.created_at.format("%Y-%m-%d"),
                if active { "Active" } else { "Revoked" },
                mcp_client_config_target(grant.client),
                mcp_client_setup_note(grant.client),
                grant
                    .last_used_at
                    .map(|last_used| format!("Last used {}", last_used.format("%Y-%m-%d %H:%M")))
                    .unwrap_or_else(|| "Never used".to_string()),
            ),
            actions,
            cx,
        ));
    }

    if !settings.mcp.legacy_access && settings.mcp.grants.is_empty() {
        grants = grants.child(
            div()
                .border_t_1()
                .border_color(islands::panel_border(&settings.appearance, cx))
                .py(spacing::lg())
                .flex()
                .items_center()
                .gap(spacing::sm())
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::Bot).small())
                .child(
                    "No clients yet. Add an agent to copy its client-specific MCP configuration.",
                ),
        );
    }

    group(
        "Agent clients",
        div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .child(setting_row_with_description(
                "Authenticated clients",
                &format!(
                    "{active_count} active. Each client has its own revocable Keychain token."
                ),
                create_button,
                cx,
            ))
            .child(grants)
            .when(active_count > 0 || settings.mcp.legacy_access, |content| {
                content.child(
                    div()
                        .border_t_1()
                        .border_color(islands::panel_border(&settings.appearance, cx))
                        .pt(spacing::md())
                        .child(setting_row_with_description(
                            "Revoke all access",
                            "Disconnect every agent client and disable the local MCP server.",
                            reset_button,
                            cx,
                        )),
                )
            }),
        &settings.appearance,
        cx,
    )
}

fn mcp_grant_row(label: &str, description: &str, actions: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::md())
        .py(spacing::md())
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .min_w(px(0.0))
                .flex_1()
                .child(
                    div()
                        .size(px(28.0))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .bg(cx.theme().secondary.opacity(0.5))
                        .text_color(cx.theme().secondary_foreground)
                        .child(Icon::new(IconName::Bot).xsmall()),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .min_w(px(0.0))
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(cx.theme().foreground)
                                .child(label.to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(description.to_string()),
                        ),
                ),
        )
        .child(div().flex_shrink_0().child(actions))
}

fn create_mcp_client_grant(state: Entity<AppState>, client: McpClientKind, cx: &mut App) {
    let id = uuid::Uuid::new_v4();
    let bytes: [u8; 32] = rand::random();
    let token = bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let write = crate::helpers::keystore::KeyStore::write_mcp_grant(cx, id, &token);
    cx.spawn(async move |cx: &mut AsyncApp| match write.await {
        Ok(()) => {
            cx.update(|cx| {
                let (label, port) = {
                    let settings = &state.read(cx).settings.mcp;
                    let count =
                        settings.grants.iter().filter(|grant| grant.client == client).count();
                    let label = if count == 0 {
                        client.label().to_string()
                    } else {
                        format!("{} {}", client.label(), count + 1)
                    };
                    (label, settings.port)
                };
                state.update(cx, |state, cx| {
                    state.settings.mcp.grants.push(crate::state::McpClientGrant {
                        id,
                        label,
                        client,
                        created_at: chrono::Utc::now(),
                        last_used_at: None,
                        revoked_at: None,
                    });
                    state.save_settings();
                    state.set_status_message(Some(crate::state::StatusMessage::info(format!(
                        "{} client grant created; configuration copied",
                        client.label()
                    ))));
                    cx.notify();
                });
                cx.write_to_clipboard(ClipboardItem::new_string(mcp_client_config(
                    client, port, id,
                )));
            });
        }
        Err(error) => {
            cx.update(|cx| {
                state.update(cx, |state, cx| {
                    state.set_status_message(Some(crate::state::StatusMessage::error(format!(
                        "Could not create MCP client grant: {error}"
                    ))));
                    cx.notify();
                });
            });
        }
    })
    .detach();
}

fn copy_mcp_grant_token(state: Entity<AppState>, id: uuid::Uuid, cx: &mut App) {
    let read = crate::helpers::keystore::KeyStore::read_mcp_grant(cx, id);
    cx.spawn(async move |cx: &mut AsyncApp| match read.await {
        Ok(Some(token)) => {
            cx.update(|cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(token));
                state.update(cx, |state, cx| {
                    state.set_status_message(Some(crate::state::StatusMessage::info(
                        "Client token copied; treat it like a password",
                    )));
                    cx.notify();
                });
            });
        }
        Ok(None) => {
            cx.update(|cx| {
                state.update(cx, |state, cx| {
                    state.set_status_message(Some(crate::state::StatusMessage::error(
                        "Client token is missing from macOS Keychain",
                    )));
                    cx.notify();
                });
            });
        }
        Err(error) => {
            cx.update(|cx| {
                state.update(cx, |state, cx| {
                    state.set_status_message(Some(crate::state::StatusMessage::error(format!(
                        "Could not copy client token: {error}"
                    ))));
                    cx.notify();
                });
            });
        }
    })
    .detach();
}

fn mcp_client_config(client: McpClientKind, port: u16, id: uuid::Uuid) -> String {
    let url = format!("http://127.0.0.1:{port}/mcp");
    match client {
        McpClientKind::Pi => pretty_json(serde_json::json!({
            "mcpServers": {
                "openmango": {
                    "url": url,
                    "auth": "bearer",
                    "bearerToken": format!("!{}", mcp_keychain_command(id)),
                    "protocolVersion": "2026-07-28"
                }
            }
        })),
        McpClientKind::ClaudeCode => pretty_json(serde_json::json!({
            "mcpServers": {
                "openmango": {
                    "type": "http",
                    "url": url,
                    "headersHelper": format!(
                        "token=$({}) && printf '{{\"Authorization\":\"Bearer %s\"}}' \"$token\"",
                        mcp_keychain_command(id)
                    )
                }
            }
        })),
        McpClientKind::Codex => format!(
            "[mcp_servers.openmango]\nurl = \"{url}\"\nbearer_token_env_var = \"OPENMANGO_MCP_TOKEN\"\ntool_timeout_sec = 35\n"
        ),
        McpClientKind::Cursor => pretty_json(serde_json::json!({
            "mcpServers": {
                "openmango": {
                    "url": url,
                    "headers": {
                        "Authorization": "Bearer ${env:OPENMANGO_MCP_TOKEN}"
                    }
                }
            }
        })),
        McpClientKind::VsCode => {
            let input_id = format!("openmango-mcp-token-{}", &id.to_string()[..8]);
            pretty_json(serde_json::json!({
                "inputs": [{
                    "type": "promptString",
                    "id": input_id,
                    "description": "OpenMango MCP client token",
                    "password": true
                }],
                "servers": {
                    "openmango": {
                        "type": "http",
                        "url": url,
                        "headers": {
                            "Authorization": format!("Bearer ${{input:{input_id}}}")
                        }
                    }
                }
            }))
        }
    }
}

fn mcp_keychain_command(id: uuid::Uuid) -> String {
    format!("security find-internet-password -s com.openmango.mcp.grant.{id} -a {id} -w")
}

fn pretty_json(value: serde_json::Value) -> String {
    serde_json::to_string_pretty(&value).expect("JSON values are serializable")
}

fn mcp_client_config_target(client: McpClientKind) -> &'static str {
    match client {
        McpClientKind::Pi | McpClientKind::ClaudeCode => ".mcp.json",
        McpClientKind::Codex => "~/.codex/config.toml",
        McpClientKind::Cursor => "~/.cursor/mcp.json",
        McpClientKind::VsCode => ".vscode/mcp.json",
    }
}

fn mcp_client_setup_note(client: McpClientKind) -> &'static str {
    match client {
        McpClientKind::Pi | McpClientKind::ClaudeCode => "Uses macOS Keychain automatically",
        McpClientKind::Codex | McpClientKind::Cursor => "Uses OPENMANGO_MCP_TOKEN",
        McpClientKind::VsCode => "Prompts once for the copied token",
    }
}

fn history_coverage_summary(report: &crate::history::EligibilityReport) -> Option<String> {
    let needs_setup = report
        .collections
        .iter()
        .filter(|coverage| {
            coverage.regular
                && coverage.reason.as_deref() == Some("changeStreamPreAndPostImages is not enabled")
        })
        .count();
    let unavailable = report
        .collections
        .iter()
        .filter(|coverage| {
            coverage.regular
                && coverage.reason.is_some()
                && coverage.reason.as_deref() != Some("changeStreamPreAndPostImages is not enabled")
        })
        .count();
    let excluded = report.collections.iter().filter(|coverage| !coverage.regular).count();
    let mut summaries = Vec::new();
    if needs_setup > 0 {
        summaries.push(format!(
            "Pre/post images are disabled on {needs_setup} collection{}.",
            if needs_setup == 1 { "" } else { "s" }
        ));
    }
    if unavailable > 0 {
        summaries.push(format!(
            "{unavailable} regular collection{} cannot be covered.",
            if unavailable == 1 { "" } else { "s" }
        ));
    }
    if excluded > 0 {
        summaries.push(format!(
            "{excluded} unsupported view or time-series collection{} excluded.",
            if excluded == 1 { " is" } else { "s are" }
        ));
    }
    (!summaries.is_empty()).then(|| summaries.join(" "))
}

fn render_agent_connections_section(
    state: Entity<AppState>,
    settings: &AppSettings,
    cx: &App,
) -> impl IntoElement {
    let connections = state.read(cx).connections_snapshot();
    let content = if connections.is_empty() {
        div()
            .border_t_1()
            .border_color(islands::panel_border(&settings.appearance, cx))
            .py(spacing::lg())
            .flex()
            .items_center()
            .gap(spacing::sm())
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(IconName::Info).small())
            .child("Add a MongoDB connection before granting agent access.")
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .children(connections.into_iter().map(|connection| {
                let connection_id = connection.id;
                let shared = connection.agent_shared;
                let agent_writable = connection.agent_writable;
                let connected = state.read(cx).is_connected(connection.id);
                let history_enabled = connection.history_enabled;
                let history_max_age_days = connection.history_max_age_days;
                let history_max_bytes = connection.history_max_bytes;
                let history_report = state.read(cx).history_eligibility(connection_id).cloned();
                let history_inspecting = state.read(cx).history_inspecting(connection_id);
                let history_eligible = history_report.as_ref().is_some_and(|report| {
                    report.status == crate::history::EligibilityStatus::Eligible
                });
                let history_needs_setup = history_report.as_ref().is_some_and(|report| {
                    report.status == crate::history::EligibilityStatus::NeedsSetup
                });
                let history_reason = if history_needs_setup {
                    "History needs setup before it can be enabled.".to_string()
                } else {
                    history_report
                        .as_ref()
                        .and_then(|report| report.exact_reason())
                        .unwrap_or(if connected {
                            "Inspect eligibility before enabling History."
                        } else {
                            "Connect before inspecting History eligibility."
                        })
                        .to_string()
                };
                let history_usage = state
                    .read(cx)
                    .history_service()
                    .and_then(|service| service.cached_usage(connection_id))
                    .or_else(|| state.read(cx).history_usage(connection_id));
                let history_gap_count = if history_enabled {
                    state
                        .read(cx)
                        .history_service()
                        .and_then(|service| service.list_gaps(connection_id, None, None).ok())
                        .map(|gaps| gaps.into_iter().filter(|gap| !gap.resolved).count())
                        .unwrap_or(0)
                } else {
                    0
                };
                let history_coverage =
                    history_report.as_ref().and_then(history_coverage_summary);
                let protected = connection.protected
                    || connection.environment
                        == Some(crate::models::ConnectionEnvironment::Production);
                let state_for_share = state.clone();
                let state_for_writes = state.clone();
                let state_for_history = state.clone();
                let state_for_inspection = state.clone();
                let state_for_setup = state.clone();
                let state_for_age = state.clone();
                let state_for_bytes = state.clone();
                let state_for_clear = state.clone();
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .items_start()
                    .gap(spacing::md())
                    .py(spacing::md())
                    .border_t_1()
                    .border_color(islands::panel_border(&settings.appearance, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .min_w(px(0.0))
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(cx.theme().foreground)
                                    .child(connection.name),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} · {} · {}{}",
                                        connection
                                            .environment
                                            .map(|environment| environment.label())
                                            .unwrap_or("Environment not set"),
                                        if connected { "Connected" } else { "Disconnected" },
                                        if connection.read_only { "Read-only" } else { "Writable" },
                                        if protected { " · Protected" } else { "" },
                                    )),
                            )
                            .when(shared && protected, |content| {
                                content.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().warning)
                                        .child(if agent_writable {
                                            "Agents have direct write access to this protected connection."
                                        } else {
                                            "Visible to agents; direct writes are disabled."
                                        }),
                                )
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if history_eligible {
                                        cx.theme().success
                                    } else {
                                        cx.theme().muted_foreground
                                    })
                                    .child(if history_eligible {
                                        "History eligible: all covered regular collections have pre/post images."
                                            .to_string()
                                    } else {
                                        history_reason.clone()
                                    }),
                            )
                            .child(
                                div()
                                    .max_w(px(620.0))
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("History records supported changes from every client observed by OpenMango on this device, can contain gaps, and is not a backup or audit log."),
                            )
                            .when_some(history_coverage, |content, coverage| {
                                content.child(
                                    div()
                                        .max_w(px(620.0))
                                        .text_xs()
                                        .text_color(cx.theme().warning)
                                        .child(coverage),
                                )
                            })
                            .when(history_gap_count > 0, |content| {
                                content.child(
                                    div()
                                        .max_w(px(620.0))
                                        .text_xs()
                                        .text_color(cx.theme().warning)
                                        .child(format!(
                                            "History coverage: {history_gap_count} interruption{}. Some changes may not be restorable.",
                                            if history_gap_count == 1 { "" } else { "s" }
                                        )),
                                )
                            })
                            .when_some(history_usage, |content, usage| {
                                content.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "History storage: {} MiB · {} batches · {} items",
                                            usage.encrypted_bytes / (1024 * 1024),
                                            usage.batches,
                                            usage.items,
                                        )),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .items_end()
                            .gap(spacing::sm())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(spacing::sm())
                                    .child(div().text_xs().child("Share with agents"))
                                    .child(
                                        Switch::new(("agent-share-connection", connection_id.as_u128() as u64))
                                            .checked(shared)
                                            .small()
                                            .on_click(move |checked, window, cx| {
                                                let apply = {
                                                    let state = state_for_share.clone();
                                                    let checked = *checked;
                                                    move |_window: &mut Window, cx: &mut App| {
                                                        state.update(cx, |state, cx| {
                                                            state.set_connection_agent_shared(
                                                                connection_id,
                                                                checked,
                                                                cx,
                                                            );
                                                        });
                                                    }
                                                };
                                                if *checked && protected {
                                                    open_confirm_dialog(
                                                        window,
                                                        cx,
                                                        "Share protected connection",
                                                        "Agents will be able to see this connection and read database metadata. Credentials remain hidden. Direct writes remain disabled until separately enabled.",
                                                        "Share connection",
                                                        false,
                                                        apply,
                                                    );
                                                } else {
                                                    apply(window, cx);
                                                }
                                            }),
                                    ),
                            )
                            .when(shared, |controls| {
                                controls.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(spacing::sm())
                                        .child(div().text_xs().child("Allow agent writes"))
                                        .child(
                                            Switch::new(("agent-write-connection", connection_id.as_u128() as u64))
                                                .checked(agent_writable)
                                                .small()
                                                .disabled(connection.read_only)
                                                .on_click(move |checked, window, cx| {
                                                    let apply = {
                                                        let state = state_for_writes.clone();
                                                        let checked = *checked;
                                                        move |_window: &mut Window, cx: &mut App| {
                                                            state.update(cx, |state, cx| {
                                                                state.set_connection_agent_writable(
                                                                    connection_id,
                                                                    checked,
                                                                    cx,
                                                                );
                                                            });
                                                        }
                                                    };
                                                    if *checked && protected {
                                                        open_confirm_dialog(
                                                            window,
                                                            cx,
                                                            "Allow direct agent writes",
                                                            "Authenticated MCP clients will be able to insert, update, replace, delete, and restore History batches directly on this protected or Production connection without per-operation approval. History is not a backup, and restores remain conflict-safe.",
                                                            "Allow agent writes",
                                                            true,
                                                            apply,
                                                        );
                                                    } else {
                                                        apply(window, cx);
                                                    }
                                                }),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(spacing::sm())
                                    .child(div().text_xs().child("History"))
                                    .child(
                                        Switch::new(("history-connection", connection_id.as_u128() as u64))
                                            .checked(history_enabled)
                                            .small()
                                            .disabled(
                                                history_inspecting
                                                    || (!history_enabled && !history_eligible),
                                            )
                                            .on_click(move |checked, _window, cx| {
                                                state_for_history.update(cx, |state, cx| {
                                                    state.set_connection_history_enabled(
                                                        connection_id,
                                                        *checked,
                                                        cx,
                                                    );
                                                });
                                            }),
                                    ),
                            )
                            .when(!history_eligible, |controls| {
                                controls.child(
                                    Button::new(("inspect-history", connection_id.as_u128() as u64))
                                        .ghost()
                                        .xsmall()
                                        .label(if history_inspecting {
                                            "Inspecting…"
                                        } else if history_needs_setup {
                                            "Enable pre/post images"
                                        } else {
                                            "Inspect eligibility"
                                        })
                                        .disabled(!connected || history_inspecting)
                                        .on_click(move |_, _, cx| {
                                            AppCommands::inspect_history_eligibility(
                                                if history_needs_setup {
                                                    state_for_setup.clone()
                                                } else {
                                                    state_for_inspection.clone()
                                                },
                                                connection_id,
                                                history_needs_setup,
                                                history_needs_setup,
                                                cx,
                                            );
                                        }),
                                )
                            })
                            .when(history_enabled, |controls| {
                                let next_age = if history_max_age_days <= 7 {
                                    30
                                } else if history_max_age_days <= 30 {
                                    90
                                } else {
                                    7
                                };
                                let mib = 1024 * 1024;
                                let next_bytes = if history_max_bytes <= 256 * mib {
                                    1024 * mib
                                } else if history_max_bytes <= 1024 * mib {
                                    5 * 1024 * mib
                                } else {
                                    256 * mib
                                };
                                controls.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(spacing::xs())
                                        .child(
                                            Button::new(("history-age", connection_id.as_u128() as u64))
                                                .ghost()
                                                .xsmall()
                                                .label(format!("{} days", history_max_age_days))
                                                .on_click(move |_, _, cx| {
                                                    state_for_age.update(cx, |state, cx| {
                                                        state.set_connection_history_retention(
                                                            connection_id,
                                                            next_age,
                                                            history_max_bytes,
                                                            cx,
                                                        );
                                                    });
                                                }),
                                        )
                                        .child(
                                            Button::new(("history-size", connection_id.as_u128() as u64))
                                                .ghost()
                                                .xsmall()
                                                .label(format!(
                                                    "{} MiB",
                                                    history_max_bytes / mib
                                                ))
                                                .on_click(move |_, _, cx| {
                                                    state_for_bytes.update(cx, |state, cx| {
                                                        state.set_connection_history_retention(
                                                            connection_id,
                                                            history_max_age_days,
                                                            next_bytes,
                                                            cx,
                                                        );
                                                    });
                                                }),
                                        )
                                        .child(
                                            Button::new(("clear-connection-history", connection_id.as_u128() as u64))
                                                .ghost()
                                                .xsmall()
                                                .label("Clear connection")
                                                .on_click(move |_, window, cx| {
                                                    let state = state_for_clear.clone();
                                                    open_confirm_dialog(
                                                        window,
                                                        cx,
                                                        "Clear connection History",
                                                        "Delete all non-active encrypted History batches, gaps, and resume state for this connection. Recording restarts from the current point and this cannot be undone.",
                                                        "Clear History",
                                                        true,
                                                        move |_, cx| {
                                                            AppCommands::clear_connection_history(
                                                                state.clone(),
                                                                connection_id,
                                                                cx,
                                                            );
                                                        },
                                                    );
                                                }),
                                        ),
                                )
                            }),
                    )
            }))
            .into_any_element()
    };

    group(
        "Connection access",
        div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .child(
                div()
                    .max_w(px(620.0))
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Agents see only enabled connections. Credentials and connection strings are never exposed.",
                    ),
            )
            .child(content)
            .child(
                Button::new("clear-all-history")
                    .ghost()
                    .xsmall()
                    .label("Clear all History")
                    .on_click({
                        let state = state.clone();
                        move |_, window, cx| {
                            let state_for_clear = state.clone();
                            open_confirm_dialog(
                                window,
                                cx,
                                "Clear all History",
                                "Delete every non-active local encrypted History batch, gap, and resume cursor. Recording restarts from the current point. This cannot be undone.",
                                "Clear all History",
                                true,
                                move |_, cx| {
                                    AppCommands::clear_all_history(state_for_clear.clone(), cx);
                                },
                            );
                        }
                    }),
            ),
        &settings.appearance,
        cx,
    )
}

fn render_transfer_section(
    state: Entity<AppState>,
    settings: &AppSettings,
    template_input_state: Entity<InputState>,
    batch_size_input_state: Entity<InputState>,
    cx: &App,
) -> impl IntoElement {
    let current_format = settings.transfer.default_export_format;
    let current_import_mode = settings.transfer.default_import_mode;
    let current_folder = settings.transfer.default_export_folder.clone();

    // Format dropdown
    let format_dropdown = {
        let state = state.clone();
        gpui_kit::component::button::Button::new("format-dropdown")
            .xsmall()
            .label(current_format.label())
            .dropdown_caret(true)
            .rounded(borders::radius_sm())
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu: PopupMenu, _window, _cx| {
                let formats = [
                    TransferFormat::JsonLines,
                    TransferFormat::JsonArray,
                    TransferFormat::Csv,
                    TransferFormat::Bson,
                ];
                let mut m = menu;
                for format in formats {
                    let s = state.clone();
                    m = m.item(PopupMenuItem::new(format.label()).on_click(move |_, _, cx| {
                        s.update(cx, |state, cx| {
                            state.settings.transfer.default_export_format = format;
                            state.save_settings();
                            cx.notify();
                        });
                    }));
                }
                m
            })
    };

    // Batch size input using NumberInput
    let batch_size_input = NumberInput::new(&batch_size_input_state).small().w(px(100.0));

    // Import mode dropdown
    let import_mode_dropdown = {
        let state = state.clone();
        gpui_kit::component::button::Button::new("import-mode-dropdown")
            .xsmall()
            .label(current_import_mode.label())
            .dropdown_caret(true)
            .rounded(borders::radius_sm())
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _window, _cx| {
                let modes = [InsertMode::Insert, InsertMode::Upsert, InsertMode::Replace];
                let mut m = menu;
                for mode in modes {
                    let s = state.clone();
                    m = m.item(PopupMenuItem::new(mode.label()).on_click(move |_, _, cx| {
                        s.update(cx, |state, cx| {
                            state.settings.transfer.default_import_mode = mode;
                            state.save_settings();
                            cx.notify();
                        });
                    }));
                }
                m
            })
    };

    // Folder picker
    let folder_control = {
        let state = state.clone();
        let folder_display = if current_folder.is_empty() {
            "Default (Downloads)".to_string()
        } else {
            // Show just the last component
            std::path::Path::new(&current_folder)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or(current_folder.clone())
        };
        let is_empty = current_folder.is_empty();

        let state_for_browse = state.clone();
        let browse_button = crate::components::Button::new("browse-folder")
            .xsmall()
            .label("Browse...")
            .on_click(move |_, _, cx| {
                let state = state_for_browse.clone();
                cx.spawn(async move |cx| {
                    if let Some(path) =
                        crate::components::file_picker::open_folder_dialog_async().await
                    {
                        cx.update(|cx| {
                            state.update(cx, |state, cx| {
                                state.settings.transfer.default_export_folder =
                                    path.display().to_string();
                                state.save_settings();
                                cx.notify();
                            });
                        });
                    }
                })
                .detach();
            });

        let clear_button = if !is_empty {
            let state = state.clone();
            Some(
                crate::components::Button::new("clear-folder")
                    .ghost()
                    .xsmall()
                    .label("Clear")
                    .on_click(move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.settings.transfer.default_export_folder.clear();
                            state.save_settings();
                            cx.notify();
                        });
                    }),
            )
        } else {
            None
        };

        div()
            .flex()
            .items_center()
            .gap(spacing::sm())
            .child(
                div()
                    .px(spacing::sm())
                    .py(px(6.0))
                    .bg(cx.theme().sidebar)
                    .border_1()
                    .border_color(cx.theme().sidebar_border)
                    .rounded(borders::radius_sm())
                    .text_sm()
                    .text_color(if is_empty {
                        cx.theme().muted_foreground
                    } else {
                        cx.theme().foreground
                    })
                    .min_w(px(150.0))
                    .child(folder_display),
            )
            .child(browse_button)
            .children(clear_button)
    };

    // Filename template with placeholder dropdown
    let template_control = {
        let state_for_reset = state.clone();
        let template_state_for_dropdown = template_input_state.clone();
        let template_state_for_reset = template_input_state.clone();

        let placeholder_button = gpui_kit::component::button::Button::new("placeholder-dropdown")
            .xsmall()
            .label("${}")
            .rounded(borders::radius_sm())
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _window, _cx| {
                for (placeholder, description) in FILENAME_PLACEHOLDERS {
                    let p = (*placeholder).to_string();
                    let template_state = template_state_for_dropdown.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{} - {}", placeholder, description)).on_click(
                            move |_, window, cx| {
                                template_state.update(cx, |input_state, cx| {
                                    let current = input_state.value().to_string();
                                    input_state.set_value(format!("{}{}", current, p), window, cx);
                                });
                            },
                        ),
                    );
                }
                // Add reset option
                let state = state_for_reset.clone();
                let template_state = template_state_for_reset.clone();
                menu = menu.separator().item(PopupMenuItem::new("Reset to default").on_click(
                    move |_, window, cx| {
                        state.update(cx, |state, cx| {
                            state.settings.transfer.export_filename_template =
                                DEFAULT_FILENAME_TEMPLATE.to_string();
                            state.save_settings();
                            cx.notify();
                        });
                        template_state.update(cx, |input_state, cx| {
                            input_state.set_value(
                                DEFAULT_FILENAME_TEMPLATE.to_string(),
                                window,
                                cx,
                            );
                        });
                    },
                ));
                menu
            });

        div()
            .flex()
            .items_center()
            .gap(spacing::sm())
            .child(Input::new(&template_input_state).small().w(px(250.0)))
            .child(placeholder_button)
    };

    div()
        .flex()
        .flex_col()
        .gap(spacing::md())
        .child(group(
            "Export",
            div()
                .flex()
                .flex_col()
                .gap(spacing::md())
                .child(setting_row("Default format", format_dropdown, cx))
                .child(setting_row_with_description(
                    "Target folder",
                    "Default folder for exported files",
                    folder_control,
                    cx,
                ))
                .child(setting_row_with_description(
                    "Filename template",
                    "Template for generated filenames",
                    template_control,
                    cx,
                )),
            &settings.appearance,
            cx,
        ))
        .child(group(
            "Import",
            div()
                .flex()
                .flex_col()
                .gap(spacing::md())
                .child(setting_row("Default import mode", import_mode_dropdown, cx))
                .child(setting_row("Batch size", batch_size_input, cx)),
            &settings.appearance,
            cx,
        ))
}

#[derive(Clone)]
struct AiSectionUiState {
    api_key_input_state: Entity<InputState>,
    ollama_base_url_input_state: Entity<InputState>,
    ai_test_in_flight: bool,
    ai_test_result: Option<AiTestResult>,
}

fn render_ai_section(
    view: Entity<SettingsView>,
    state: Entity<AppState>,
    settings: &AppSettings,
    ai_ui: AiSectionUiState,
    cx: &App,
) -> impl IntoElement {
    let AiSectionUiState {
        api_key_input_state,
        ollama_base_url_input_state,
        ai_test_in_flight,
        ai_test_result,
    } = ai_ui;

    let ai_enabled = settings.ai.enabled;
    let current_provider = settings.ai.provider;
    let cached = &state.read(cx).ai_chat.cached_models;

    let enabled_checkbox = {
        let state = state.clone();
        gpui_kit::component::checkbox::Checkbox::new("ai-enabled").checked(ai_enabled).on_click(
            move |_, window, cx| {
                if ai_enabled {
                    state.update(cx, |state, cx| {
                        state.settings.ai.enabled = false;
                        state.save_settings();
                        cx.notify();
                    });
                    return;
                }
                let provider = state.read(cx).settings.ai.provider.label();
                let destination = if provider == "Ollama" {
                    "your configured Ollama endpoint"
                } else {
                    provider
                };
                open_confirm_dialog(
                    window,
                    cx,
                    "Enable AI Assistant",
                    format!(
                        "AI requests are sent to {destination}. They include your chat messages and may include connection/database/collection names, active query text, schema summaries (including schema sample values), indexes, statistics, aggregation stages, and tool results. Credentials are not included. Selected document contents and automatic document samples remain excluded unless you enable those separate sharing options."
                    ),
                    "Enable AI",
                    false,
                    {
                        let state = state.clone();
                        move |_window, cx| {
                            state.update(cx, |state, cx| {
                                state.settings.ai.enabled = true;
                                state.save_settings();
                                cx.notify();
                            });
                        }
                    },
                );
            },
        )
    };

    let selected_documents_checkbox = {
        let state = state.clone();
        let checked = settings.ai.share_selected_documents;
        gpui_kit::component::checkbox::Checkbox::new("ai-share-selected-documents")
            .checked(checked)
            .on_click(move |_, _, cx| {
                state.update(cx, |state, cx| {
                    state.settings.ai.share_selected_documents = !checked;
                    state.save_settings();
                    cx.notify();
                });
            })
    };

    let sample_documents_checkbox = {
        let state = state.clone();
        let checked = settings.ai.share_sample_documents;
        gpui_kit::component::checkbox::Checkbox::new("ai-share-sample-documents")
            .checked(checked)
            .on_click(move |_, _, cx| {
                state.update(cx, |state, cx| {
                    state.settings.ai.share_sample_documents = !checked;
                    state.save_settings();
                    cx.notify();
                });
            })
    };

    let provider_dropdown = {
        let state = state.clone();
        let view = view.clone();
        gpui_kit::component::button::Button::new("ai-provider-dropdown")
            .ghost()
            .xsmall()
            .label(current_provider.label())
            .dropdown_caret(true)
            .rounded(islands::radius_sm(&settings.appearance))
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _window, _cx| {
                let providers = [
                    AiProvider::Gemini,
                    AiProvider::OpenAi,
                    AiProvider::Anthropic,
                    AiProvider::Ollama,
                ];
                let mut menu = menu;
                for provider in providers {
                    let state = state.clone();
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(provider.label()).on_click(
                        move |_, window, cx| {
                            state.update(cx, |state, cx| {
                                state.settings.ai.set_provider(provider);
                                state.save_settings();
                                cx.notify();
                            });
                            view.update(cx, |this, cx| {
                                this.ai_test_result = None;
                                this.sync_ai_inputs_from_settings(window, cx);
                                cx.notify();
                            });
                        },
                    ));
                }
                menu
            })
    };

    let model_dropdown = {
        let state = state.clone();
        let current_model = settings.ai.model.clone();

        let models: Vec<String> = match current_provider {
            AiProvider::Ollama => match cached {
                ModelCache::Loaded(list) => {
                    let mut m = list.clone();
                    if !current_model.trim().is_empty() && !m.contains(&current_model) {
                        m.push(current_model.clone());
                        m.sort();
                    }
                    m
                }
                _ => {
                    if !current_model.trim().is_empty() {
                        vec![current_model.clone()]
                    } else {
                        vec![]
                    }
                }
            },
            _ => current_provider.model_options(&current_model),
        };

        let cached_hint: Option<String> = if current_provider == AiProvider::Ollama {
            match cached {
                ModelCache::Loading => Some("Loading models...".to_string()),
                ModelCache::Error(msg) => {
                    let hint =
                        if msg.len() > 60 { format!("{}...", &msg[..57]) } else { msg.clone() };
                    Some(hint)
                }
                ModelCache::NotFetched => Some("Fetching models...".to_string()),
                _ => None,
            }
        } else if matches!(cached, ModelCache::NoKey) {
            Some("Add API key in Settings".to_string())
        } else {
            None
        };

        gpui_kit::component::button::Button::new("ai-model-dropdown")
            .ghost()
            .xsmall()
            .label(current_model)
            .dropdown_caret(true)
            .rounded(islands::radius_sm(&settings.appearance))
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _window, _cx| {
                let mut menu = menu;
                if let Some(hint) = &cached_hint {
                    menu = menu.item(PopupMenuItem::new(hint.clone()).disabled(true));
                }
                for model in &models {
                    let state = state.clone();
                    let m = model.clone();
                    let note = AiProvider::model_note(model);
                    let item = if let Some(note) = note {
                        let model_label = model.clone();
                        let note = note.to_string();
                        PopupMenuItem::element(move |_window, cx| {
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().foreground)
                                        .child(model_label.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(note.clone()),
                                )
                        })
                    } else {
                        PopupMenuItem::new(model.clone())
                    };
                    menu = menu.item(item.on_click(move |_, _, cx| {
                        state.update(cx, |app_state, cx| {
                            app_state.settings.ai.set_model(m.clone());
                            app_state.save_settings();
                            cx.notify();
                        });
                    }));
                }
                menu
            })
    };

    let model_status_badge = {
        let (label, accent) = match (current_provider, cached) {
            (AiProvider::Ollama, ModelCache::Loaded(list)) => {
                (format!("{} models", list.len()), cx.theme().primary)
            }
            (AiProvider::Ollama, ModelCache::Loading) => {
                ("Loading models".to_string(), cx.theme().warning)
            }
            (AiProvider::Ollama, ModelCache::NotFetched) => {
                ("Fetching models".to_string(), cx.theme().warning)
            }
            (AiProvider::Ollama, ModelCache::Error(_)) => {
                ("Model fetch error".to_string(), cx.theme().danger)
            }
            (_, ModelCache::NoKey) => ("API key missing".to_string(), cx.theme().warning),
            _ => ("Ready".to_string(), cx.theme().muted_foreground),
        };
        div()
            .px(spacing::xs())
            .py(px(2.0))
            .rounded(islands::radius_sm(&settings.appearance))
            .bg(accent.opacity(0.1))
            .border_1()
            .border_color(accent.opacity(0.28))
            .text_xs()
            .text_color(accent)
            .child(label)
    };

    let test_button = {
        let view = view.clone();
        Button::new("ai-test-provider")
            .xsmall()
            .label(if ai_test_in_flight { "Testing..." } else { "Test provider" })
            .disabled(ai_test_in_flight || !ai_enabled)
            .on_click(move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                view.update(cx, |this, cx| {
                    this.start_ai_test(cx);
                });
            })
    };

    let test_status = ai_test_result.as_ref().map(|result| match result {
        AiTestResult::Success(message) => {
            (format!("Provider test succeeded: {}", message.trim()), cx.theme().primary)
        }
        AiTestResult::Error(message) => {
            (format!("Provider test failed: {}", message.trim()), cx.theme().danger_foreground)
        }
    });

    div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .child(group(
                "Provider",
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .child(setting_row_with_description(
                        "Enable AI",
                        "Turn on AI chat and AI-assisted actions in collection views.",
                        enabled_checkbox,
                        cx,
                    ))
                    .child(setting_row("Provider", provider_dropdown, cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(spacing::sm())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(spacing::sm())
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().secondary_foreground)
                                            .child("Model"),
                                    )
                                    .child(model_dropdown),
                            )
                            .child(model_status_badge),
                    ),
                &settings.appearance,
                cx,
            ))
            .child(group(
                "Privacy",
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "Every AI request includes chat messages and relevant workspace metadata such as names, active queries, schema summaries, indexes, statistics, aggregation stages, and tool results. Credentials are never included.",
                            ),
                    )
                    .child(setting_row_with_description(
                        "Share selected documents",
                        "Include up to three selected document bodies in automatic AI context.",
                        selected_documents_checkbox,
                        cx,
                    ))
                    .child(setting_row_with_description(
                        "Share document samples",
                        "Include up to five documents from the current result set in automatic AI context.",
                        sample_documents_checkbox,
                        cx,
                    )),
                &settings.appearance,
                cx,
            ))
            .child(group(
                "Credentials",
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .children((current_provider != AiProvider::Ollama).then(|| {
                        setting_row(
                            "API key",
                            Input::new(&api_key_input_state).small().w(px(260.0)),
                            cx,
                        )
                    }))
                    .children((current_provider == AiProvider::Ollama).then(|| {
                        setting_row_with_description(
                            "Ollama base URL",
                            "Used only for Ollama provider.",
                            Input::new(&ollama_base_url_input_state).small().w(px(260.0)),
                            cx,
                        )
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if current_provider == AiProvider::Ollama {
                                "Ollama runs locally and does not need an API key. Keep the base URL pointed at your local server."
                            } else {
                                "If API key is empty, provider environment variables are used when available."
                            }),
                    ),
                &settings.appearance,
                cx,
            ))
            .child(group(
                "Diagnostics",
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .child(setting_row_with_description(
                        "Provider test",
                        "Sends a short request using current provider and model.",
                        test_button,
                        cx,
                    ))
                    .children(test_status.map(|(status, color)| {
                        div()
                            .px(spacing::sm())
                            .py(spacing::xs())
                            .rounded(islands::radius_sm(&settings.appearance))
                            .bg(color.opacity(0.1))
                            .border_1()
                            .border_color(color.opacity(0.3))
                            .child(div().text_xs().text_color(color).child(status))
                    })),
                &settings.appearance,
                cx,
            ))
            .children((!ai_enabled).then(|| {
                div()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child("AI is currently disabled in this workspace.")
            }))
}

// Helper functions for building UI

fn section(title: &str, content: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap(spacing::md())
        .child(
            div()
                .text_base()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .child(title.to_string()),
        )
        .child(content)
}

fn group(
    title: &str,
    content: impl IntoElement,
    appearance: &crate::state::AppearanceSettings,
    cx: &App,
) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap(spacing::sm())
        .p(spacing::md())
        .bg(islands::card_bg(appearance, cx))
        .border_1()
        .border_color(islands::panel_border(appearance, cx))
        .rounded(islands::radius_sm(appearance))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().secondary_foreground)
                .child(title.to_string()),
        )
        .child(content)
}

fn setting_row(label: &str, control: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::md())
        .child(div().text_sm().text_color(cx.theme().secondary_foreground).child(label.to_string()))
        .child(control)
}

fn setting_row_with_description(
    label: &str,
    description: &str,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap(spacing::md())
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().secondary_foreground)
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(description.to_string()),
                ),
        )
        .child(control)
}

#[cfg(test)]
mod mcp_tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn history_coverage_is_summarized_instead_of_listing_every_namespace() {
        let report = crate::history::EligibilityReport {
            status: crate::history::EligibilityStatus::NeedsSetup,
            version: Some("7.0.0".into()),
            topology: Some("replica_set".into()),
            storage_engine: Some("wiredTiger".into()),
            failures: Vec::new(),
            collections: vec![
                crate::history::CollectionCoverage {
                    database: "app".into(),
                    collection: "users".into(),
                    regular: true,
                    pre_post_images: false,
                    reason: Some("changeStreamPreAndPostImages is not enabled".into()),
                },
                crate::history::CollectionCoverage {
                    database: "app".into(),
                    collection: "orders".into(),
                    regular: true,
                    pre_post_images: false,
                    reason: Some("changeStreamPreAndPostImages is not enabled".into()),
                },
            ],
        };

        assert_eq!(
            history_coverage_summary(&report).as_deref(),
            Some("Pre/post images are disabled on 2 collections.")
        );
    }

    #[::core::prelude::v1::test]
    fn client_configs_use_native_secret_mechanisms() {
        let id = uuid::Uuid::new_v4();
        let pi = mcp_client_config(McpClientKind::Pi, 39123, id);
        let claude = mcp_client_config(McpClientKind::ClaudeCode, 39123, id);
        let codex = mcp_client_config(McpClientKind::Codex, 39123, id);
        let cursor = mcp_client_config(McpClientKind::Cursor, 39123, id);
        let vscode = mcp_client_config(McpClientKind::VsCode, 39123, id);

        for config in [&pi, &claude, &codex, &cursor, &vscode] {
            assert!(config.contains("http://127.0.0.1:39123/mcp"));
            assert!(!config.contains("mcp-server-token"));
        }
        assert!(pi.contains(&format!("com.openmango.mcp.grant.{id}")));
        assert!(pi.contains("2026-07-28"));
        assert!(claude.contains("headersHelper"));
        assert!(claude.contains(&format!("com.openmango.mcp.grant.{id}")));
        assert!(codex.contains("bearer_token_env_var = \"OPENMANGO_MCP_TOKEN\""));
        assert!(cursor.contains("${env:OPENMANGO_MCP_TOKEN}"));
        assert!(vscode.contains("${input:openmango-mcp-token-"));
        assert!(vscode.contains("\"password\": true"));
    }
}
