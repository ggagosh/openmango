#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use gpui::*;
use gpui_component::Root;
use openmango::app::AppRoot;
use openmango::assets::{Assets, embedded_fonts};
use openmango::keyboard;
use openmango::state::ConfigManager;
use openmango::theme;

fn main() {
    env_logger::init();

    Application::new().with_assets(Assets).run(|cx: &mut gpui::App| {
        // Initialize gpui-component library
        gpui_component::init(cx);
        keyboard::bind_default_keymap(cx);
        if let Err(err) = cx.text_system().add_fonts(embedded_fonts()) {
            log::warn!("Failed to load embedded fonts: {err}");
        }

        // Load saved settings
        let saved_settings = ConfigManager::default().load_settings().unwrap_or_default();
        let vibrancy = saved_settings.appearance.vibrancy;

        // Load the saved theme (or default)
        {
            let saved_theme = saved_settings.appearance.theme;
            if let Some(config) = theme::load_theme_config(saved_theme.theme_id()) {
                gpui_component::theme::Theme::global_mut(cx).apply_config(&config);
            }
        }

        // Override font families (after apply_config so they take precedence)
        {
            let theme = gpui_component::theme::Theme::global_mut(cx);
            theme.font_family = theme::fonts::ui().into();
            theme.mono_font_family = theme::fonts::mono().into();
        }

        // Apply vibrancy alpha overrides after theme is fully configured
        if vibrancy {
            theme::apply_vibrancy(cx);
        }

        let workspace = ConfigManager::default().load_workspace().unwrap_or_default();
        let default_bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        let window_bounds = workspace
            .window_state
            .as_ref()
            .map(|state| state.to_bounds())
            .unwrap_or(WindowBounds::Windowed(default_bounds));

        cx.open_window(
            WindowOptions {
                window_bounds: Some(window_bounds),
                window_background: if vibrancy {
                    WindowBackgroundAppearance::Blurred
                } else {
                    WindowBackgroundAppearance::Opaque
                },
                titlebar: Some(TitlebarOptions {
                    title: Some("OpenMango".into()),
                    appears_transparent: vibrancy,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                // Quit the app when the window is closed (standard macOS single-window behavior)
                window.on_window_should_close(cx, |_window, cx| {
                    cx.quit();
                    true
                });

                let app_view = cx.new(|cx| AppRoot::new(window, cx));
                cx.new(|cx| Root::new(app_view, window, cx))
            },
        )
        .unwrap();
    });
}
