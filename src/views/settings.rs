//! Settings view for application configuration.

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::button::ButtonVariants as _;
use gpui_component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{Sizable as _, Size};

use crate::ai::bridge::AiBridge;
use crate::ai::model_registry::{self, ModelCache};
use crate::ai::provider::{AiGenerationRequest, generate_text};
use crate::components::Button;
use crate::state::{
    AiProvider, AppSettings, AppState, AppTheme, DEFAULT_FILENAME_TEMPLATE, FILENAME_PLACEHOLDERS,
    InsertMode, TransferFormat,
};
use crate::theme::{borders, islands, sizing, spacing};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SettingsSubtab {
    #[default]
    General,
    Transfer,
    Ai,
}

impl SettingsSubtab {
    fn to_index(self) -> usize {
        match self {
            Self::General => 0,
            Self::Transfer => 1,
            Self::Ai => 2,
        }
    }

    fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Transfer,
            2 => Self::Ai,
            _ => Self::General,
        }
    }
}

#[derive(Debug, Clone)]
enum AiTestResult {
    Success(String),
    Error(String),
}

pub struct SettingsView {
    state: Entity<AppState>,
    _subscriptions: Vec<Subscription>,
    active_subtab: SettingsSubtab,
    // Input states (lazily initialized)
    template_input_state: Option<Entity<InputState>>,
    batch_size_input_state: Option<Entity<InputState>>,
    ai_api_key_input_state: Option<Entity<InputState>>,
    ai_ollama_base_url_input_state: Option<Entity<InputState>>,
    ai_test_in_flight: bool,
    ai_test_result: Option<AiTestResult>,
    last_seen_provider: AiProvider,
}

impl SettingsView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let last_seen_provider = state.read(cx).settings.ai.provider;
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
            active_subtab: SettingsSubtab::default(),
            template_input_state: None,
            batch_size_input_state: None,
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
            && self.ai_api_key_input_state.is_some()
            && self.ai_ollama_base_url_input_state.is_some()
        {
            return; // Already initialized
        }

        let template = self.state.read(cx).settings.transfer.export_filename_template.clone();
        let batch_size = self.state.read(cx).settings.transfer.default_batch_size;
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
            let _ = cx.update(|cx| {
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
        let settings = self.state.read(cx).settings.clone();
        let appearance = settings.appearance.clone();

        // Header
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .h(sizing::header_height())
            .px(spacing::lg())
            .bg(islands::tool_bg(&appearance, cx))
            .border_b_1()
            .border_color(islands::panel_border(&appearance, cx))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(cx.theme().foreground)
                    .child("Settings"),
            );

        let subtab_bar = islands::tab_bar(TabBar::new("settings-subtabs"), &appearance)
            .xsmall()
            .selected_index(self.active_subtab.to_index())
            .on_click({
                let view = view.clone();
                move |index, _window, cx| {
                    view.update(cx, |this, cx| {
                        this.active_subtab = SettingsSubtab::from_index(*index);
                        cx.notify();
                    });
                }
            })
            .children(vec![
                Tab::new().label("General"),
                Tab::new().label("Transfer"),
                Tab::new().label("AI"),
            ]);

        let tab_content = match self.active_subtab {
            SettingsSubtab::General => div()
                .flex()
                .flex_col()
                .gap(spacing::lg())
                .child(render_appearance_section(state.clone(), &settings, cx))
                .child(render_updates_section(state.clone(), &settings, cx))
                .into_any_element(),
            SettingsSubtab::Transfer => div()
                .flex()
                .flex_col()
                .gap(spacing::lg())
                .child(render_transfer_section(
                    state.clone(),
                    &settings,
                    self.template_input_state.clone().unwrap(),
                    self.batch_size_input_state.clone().unwrap(),
                    cx,
                ))
                .into_any_element(),
            SettingsSubtab::Ai => div()
                .flex()
                .flex_col()
                .gap(spacing::lg())
                .child(render_ai_section(
                    view.clone(),
                    state.clone(),
                    &settings,
                    AiSectionUiState {
                        api_key_input_state: self.ai_api_key_input_state.clone().unwrap(),
                        ollama_base_url_input_state: self
                            .ai_ollama_base_url_input_state
                            .clone()
                            .unwrap(),
                        ai_test_in_flight: self.ai_test_in_flight,
                        ai_test_result: self.ai_test_result.clone(),
                    },
                    cx,
                ))
                .into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .bg(islands::content_bg(&appearance, cx))
            .child(header)
            .child(div().px(spacing::lg()).pt(spacing::md()).child(subtab_bar))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .p(spacing::lg())
                    .overflow_y_scrollbar()
                    .child(tab_content),
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
        gpui_component::button::Button::new("theme-dropdown")
            .compact()
            .label(current_theme.label())
            .dropdown_caret(true)
            .rounded(borders::radius_sm())
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu: PopupMenu, _window, _cx| {
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
                                |_window, cx| cx.quit(),
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
                                        |_window, cx| cx.quit(),
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
        gpui_component::checkbox::Checkbox::new("show-status-bar").checked(checked).on_click(
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
        gpui_component::checkbox::Checkbox::new("vibrancy").checked(checked).on_click(
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
                    |_window, cx| cx.quit(),
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

fn render_updates_section(
    state: Entity<AppState>,
    settings: &AppSettings,
    cx: &App,
) -> impl IntoElement {
    let auto_update = settings.auto_update;

    let auto_update_checkbox = {
        let state = state.clone();
        gpui_component::checkbox::Checkbox::new("auto-update").checked(auto_update).on_click(
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
            "Automatically download and install updates in the background",
            auto_update_checkbox,
            cx,
        )),
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
        gpui_component::button::Button::new("format-dropdown")
            .compact()
            .label(current_format.label())
            .dropdown_caret(true)
            .rounded(borders::radius_sm())
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu: PopupMenu, _window, _cx| {
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
        gpui_component::button::Button::new("import-mode-dropdown")
            .compact()
            .label(current_import_mode.label())
            .dropdown_caret(true)
            .rounded(borders::radius_sm())
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _window, _cx| {
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
            .compact()
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
                        })
                        .ok();
                    }
                })
                .detach();
            });

        let clear_button = if !is_empty {
            let state = state.clone();
            Some(
                crate::components::Button::new("clear-folder")
                    .ghost()
                    .compact()
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

        let placeholder_button = gpui_component::button::Button::new("placeholder-dropdown")
            .compact()
            .label("${}")
            .rounded(borders::radius_sm())
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |mut menu, _window, _cx| {
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

    section(
        "Transfer Defaults",
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
            )),
        cx,
    )
}

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
        gpui_component::checkbox::Checkbox::new("ai-enabled").checked(ai_enabled).on_click(
            move |_, _, cx| {
                state.update(cx, |state, cx| {
                    state.settings.ai.enabled = !ai_enabled;
                    state.save_settings();
                    cx.notify();
                });
            },
        )
    };

    let provider_dropdown = {
        let state = state.clone();
        let view = view.clone();
        gpui_component::button::Button::new("ai-provider-dropdown")
            .ghost()
            .compact()
            .label(current_provider.label())
            .dropdown_caret(true)
            .rounded(islands::radius_sm(&settings.appearance))
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _window, _cx| {
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

        gpui_component::button::Button::new("ai-model-dropdown")
            .ghost()
            .compact()
            .label(current_model)
            .dropdown_caret(true)
            .rounded(islands::radius_sm(&settings.appearance))
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _window, _cx| {
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
            .compact()
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

    section(
        "AI Assistant",
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
            })),
        cx,
    )
}

// Helper functions for building UI

fn section(title: &str, content: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
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
