#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use crate::assets::{Assets, embedded_fonts};
use crate::state::ConfigManager;
use gpui::*;
use gpui_component::Root;

mod app;
mod assets;
mod bson;
mod components;
mod connection;
mod error;
mod helpers;
mod keyboard;
mod models;
mod state;
mod theme;
mod views;

use app::AppRoot;

fn main() {
    env_logger::init();

    Application::new().with_assets(Assets).run(|cx: &mut gpui::App| {
        // Initialize gpui-component library
        gpui_component::init(cx);
        keyboard::bind_default_keymap(cx);
        if let Err(err) = cx.text_system().add_fonts(embedded_fonts()) {
            log::warn!("Failed to load embedded fonts: {err}");
        }

        // Set font families on the gpui-component theme so all widgets use our fonts
        {
            let theme = gpui_component::theme::Theme::global_mut(cx);
            theme.font_family = theme::fonts::ui().into();
            theme.mono_font_family = theme::fonts::mono().into();
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
                titlebar: Some(TitlebarOptions {
                    title: Some("OpenMango".into()),
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
