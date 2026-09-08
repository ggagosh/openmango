#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use gpui_kit::component::{Root, TitleBar};
use gpui_kit::*;
use openmango::app::AppRoot;
use openmango::assets::{Assets, embedded_fonts};
use openmango::keyboard;
use openmango::state::ConfigManager;
use openmango::theme;

fn main() {
    openmango::helpers::support::init_logging();

    gpui_kit::application().with_assets(Assets).run(|cx: &mut gpui_kit::App| {
        // Initialize the toolkit before applying the app keymap and theme.
        gpui_kit::init(cx);
        let saved_settings = ConfigManager::default().load_settings().unwrap_or_default();
        keyboard::bind_keymap(cx, &saved_settings.keybindings);
        if let Err(err) = cx.text_system().add_fonts(embedded_fonts()) {
            log::warn!("Failed to load embedded fonts: {err}");
        }

        // Load saved appearance.

        let saved_theme = saved_settings.appearance.theme;
        let vibrancy = theme::effective_vibrancy(saved_theme, saved_settings.appearance.vibrancy);

        // Load the saved theme (or default)
        {
            if let Some(config) = theme::load_theme_config(saved_theme.theme_id()) {
                gpui_kit::component::theme::Theme::global_mut(cx).apply_config(&config);
            }
        }

        // Override font families (after apply_config so they take precedence)
        {
            let theme = gpui_kit::component::theme::Theme::global_mut(cx);
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
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
            },
            |window, cx| {
                let app_view = cx.new(|cx| AppRoot::new(window, cx));
                let app_view_for_close = app_view.clone();

                window.on_window_should_close(cx, move |this_window, cx| {
                    app_view_for_close.update(cx, |view, cx| {
                        view.request_quit(this_window, cx);
                    });
                    false
                });

                cx.new(|cx| Root::new(app_view, window, cx))
            },
        )
        .unwrap();
    });
}
