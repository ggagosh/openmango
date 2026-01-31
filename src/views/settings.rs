//! Settings view for application configuration.

use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{Sizable as _, Size};

use crate::state::{
    AppSettings, AppState, DEFAULT_FILENAME_TEMPLATE, FILENAME_PLACEHOLDERS, InsertMode, Theme,
    TransferFormat,
};
use crate::theme::{borders, colors, sizing, spacing};

pub struct SettingsView {
    state: Entity<AppState>,
    _subscriptions: Vec<Subscription>,
    // Input states (lazily initialized)
    template_input_state: Option<Entity<InputState>>,
    batch_size_input_state: Option<Entity<InputState>>,
}

impl SettingsView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];

        Self {
            state,
            _subscriptions: subscriptions,
            template_input_state: None,
            batch_size_input_state: None,
        }
    }

    /// Initialize input states on first render (when window is available)
    fn ensure_input_states(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.template_input_state.is_some() {
            return; // Already initialized
        }

        let template = self.state.read(cx).settings.transfer.export_filename_template.clone();
        let batch_size = self.state.read(cx).settings.transfer.default_batch_size;

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

        self.template_input_state = Some(template_input_state);
        self.batch_size_input_state = Some(batch_size_input_state);
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_input_states(window, cx);

        let state = self.state.clone();
        let settings = self.state.read(cx).settings.clone();

        // Header
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .h(sizing::header_height())
            .px(spacing::lg())
            .bg(colors::bg_header())
            .border_b_1()
            .border_color(colors::border())
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors::text_primary())
                    .child("Settings"),
            );

        // Appearance section
        let appearance_section = render_appearance_section(state.clone(), &settings);

        // Transfer section
        let transfer_section = render_transfer_section(
            state.clone(),
            &settings,
            self.template_input_state.clone().unwrap(),
            self.batch_size_input_state.clone().unwrap(),
        );

        div().flex().flex_col().flex_1().min_w(px(0.0)).child(header).child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap(spacing::lg())
                .p(spacing::lg())
                .overflow_y_scrollbar()
                .child(appearance_section)
                .child(transfer_section),
        )
    }
}

fn render_appearance_section(state: Entity<AppState>, settings: &AppSettings) -> impl IntoElement {
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
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _window, _cx| {
                let mut m = menu;
                for theme in Theme::all() {
                    let s = state.clone();
                    let t = *theme;
                    m = m.item(PopupMenuItem::new(theme.label()).on_click(move |_, _, cx| {
                        s.update(cx, |state, cx| {
                            state.settings.appearance.theme = t;
                            state.save_settings();
                            cx.notify();
                        });
                    }));
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

    section(
        "Appearance",
        div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .child(setting_row("Theme", theme_dropdown))
            .child(setting_row_with_description(
                "Show status bar",
                "Display the status bar at the bottom of the window",
                status_bar_checkbox,
            )),
    )
}

fn render_transfer_section(
    state: Entity<AppState>,
    settings: &AppSettings,
    template_input_state: Entity<InputState>,
    batch_size_input_state: Entity<InputState>,
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
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _window, _cx| {
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
                    .bg(colors::bg_sidebar())
                    .border_1()
                    .border_color(colors::border_subtle())
                    .rounded(borders::radius_sm())
                    .text_sm()
                    .text_color(if is_empty {
                        colors::text_muted()
                    } else {
                        colors::text_primary()
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
                    .child(setting_row("Default format", format_dropdown))
                    .child(setting_row_with_description(
                        "Target folder",
                        "Default folder for exported files",
                        folder_control,
                    ))
                    .child(setting_row_with_description(
                        "Filename template",
                        "Template for generated filenames",
                        template_control,
                    )),
            ))
            .child(group(
                "Import",
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .child(setting_row("Default import mode", import_mode_dropdown))
                    .child(setting_row("Batch size", batch_size_input)),
            )),
    )
}

// Helper functions for building UI

fn section(title: &str, content: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(spacing::md())
        .child(
            div()
                .text_base()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::text_primary())
                .child(title.to_string()),
        )
        .child(content)
}

fn group(title: &str, content: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .p(spacing::md())
        .bg(colors::bg_header())
        .border_1()
        .border_color(colors::border_subtle())
        .rounded(borders::radius_sm())
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors::text_secondary())
                .child(title.to_string()),
        )
        .child(content)
}

fn setting_row(label: &str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::md())
        .child(div().text_sm().text_color(colors::text_secondary()).child(label.to_string()))
        .child(control)
}

fn setting_row_with_description(label: &str, description: &str, control: impl IntoElement) -> Div {
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
                    div().text_sm().text_color(colors::text_secondary()).child(label.to_string()),
                )
                .child(
                    div().text_xs().text_color(colors::text_muted()).child(description.to_string()),
                ),
        )
        .child(control)
}
