// Design token system for OpenMango
// Theme colors are loaded from themes/*.json via gpui-component's theme system.
// Access them with `cx.theme().background`, `cx.theme().primary`, etc.
// This file only contains colors with no gpui-component equivalent, plus theme switching.

use std::rc::Rc;

use gpui_component::theme::{ThemeConfig, ThemeSet};

use crate::state::AppTheme;

// =============================================================================
// Theme Loading & Switching
// =============================================================================

const THEME_SOURCES: &[(&str, &str)] = &[
    ("vercel-dark", include_str!("../themes/openmango-dark.json")),
    ("darcula-dark", include_str!("../themes/darcula-dark.json")),
    ("tokyo-night", include_str!("../themes/tokyo-night.json")),
    ("nord", include_str!("../themes/nord.json")),
    ("one-dark", include_str!("../themes/one-dark.json")),
    ("catppuccin-mocha", include_str!("../themes/catppuccin-mocha.json")),
    ("catppuccin-latte", include_str!("../themes/catppuccin-latte.json")),
    ("solarized-light", include_str!("../themes/solarized-light.json")),
    ("solarized-dark", include_str!("../themes/solarized-dark.json")),
    ("rose-pine-dawn", include_str!("../themes/rose-pine-dawn.json")),
    ("rose-pine", include_str!("../themes/rose-pine.json")),
    ("gruvbox-light", include_str!("../themes/gruvbox-light.json")),
    ("gruvbox-dark", include_str!("../themes/gruvbox-dark.json")),
];

pub fn load_theme_config(theme_id: &str) -> Option<Rc<ThemeConfig>> {
    let json = THEME_SOURCES.iter().find(|(id, _)| *id == theme_id)?.1;
    let theme_set: ThemeSet = serde_json::from_str(json).ok()?;
    theme_set.themes.into_iter().next().map(Rc::new)
}

pub fn apply_theme(
    app_theme: AppTheme,
    vibrancy: bool,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) {
    if let Some(config) = load_theme_config(app_theme.theme_id()) {
        gpui_component::theme::Theme::global_mut(cx).apply_config(&config);

        // Re-apply font family overrides
        let theme = gpui_component::theme::Theme::global_mut(cx);
        theme.font_family = fonts::ui().into();
        theme.mono_font_family = fonts::mono().into();

        if vibrancy {
            apply_vibrancy(cx);
        }

        window.refresh();
    }
}

/// Reduce alpha on background/sidebar so the macOS blur effect shows through.
pub fn apply_vibrancy(cx: &mut gpui::App) {
    let theme = gpui_component::theme::Theme::global_mut(cx);
    theme.background.a = 0.82;
    theme.sidebar.a = 0.82;
}

// =============================================================================
// Custom Colors (theme-aware)
// =============================================================================

pub mod colors {
    use gpui::{App, Hsla};
    use gpui_component::ActiveTheme as _;

    // BSON Syntax Highlighting — reads from active theme's base colors
    pub fn syntax_key(cx: &App) -> Hsla {
        cx.theme().blue
    }
    pub fn syntax_string(cx: &App) -> Hsla {
        cx.theme().green
    }
    pub fn syntax_number(cx: &App) -> Hsla {
        cx.theme().blue
    }
    pub fn syntax_boolean(cx: &App) -> Hsla {
        cx.theme().blue
    }
    pub fn syntax_null(cx: &App) -> Hsla {
        cx.theme().muted_foreground
    }
    pub fn syntax_object_id(cx: &App) -> Hsla {
        cx.theme().cyan
    }
    pub fn syntax_date(cx: &App) -> Hsla {
        cx.theme().magenta
    }
    pub fn syntax_comment(cx: &App) -> Hsla {
        cx.theme().muted_foreground
    }

    // Dirty document highlight (warning color with alpha)
    pub fn bg_dirty(cx: &App) -> Hsla {
        let mut c = cx.theme().warning;
        c.a = 0.1;
        c
    }

    // Error background with alpha
    pub fn bg_error(cx: &App) -> Hsla {
        let mut c = cx.theme().danger;
        c.a = 0.1;
        c
    }

    // Fully transparent (for invisible default borders/backgrounds)
    pub fn transparent() -> Hsla {
        gpui::hsla(0.0, 0.0, 0.0, 0.0)
    }

    // Modal backdrop — theme background darkened with alpha
    pub fn backdrop(cx: &App) -> Hsla {
        let mut c = cx.theme().background;
        c.a = 0.5;
        c
    }

    // Warning background with alpha
    pub fn bg_warning(cx: &App) -> Hsla {
        let mut c = cx.theme().warning;
        c.a = 0.1;
        c
    }

    // Warning border with alpha
    pub fn border_warning(cx: &App) -> Hsla {
        let mut c = cx.theme().warning;
        c.a = 0.3;
        c
    }

    // Error/danger border with alpha
    pub fn border_error(cx: &App) -> Hsla {
        let mut c = cx.theme().danger;
        c.a = 0.3;
        c
    }
}

// =============================================================================
// Spacing
// =============================================================================

pub mod spacing {
    use gpui::{Pixels, px};

    pub fn xs() -> Pixels {
        px(4.0)
    }
    pub fn sm() -> Pixels {
        px(8.0)
    }
    pub fn md() -> Pixels {
        px(12.0)
    }
    pub fn lg() -> Pixels {
        px(16.0)
    }
}

// =============================================================================
// Sizing
// =============================================================================

pub mod sizing {
    use gpui::{Pixels, px};

    // Layout
    pub fn status_bar_height() -> Pixels {
        px(22.0)
    } // VS Code style thin status bar
    pub fn header_height() -> Pixels {
        px(36.0)
    }

    // Elements
    pub fn icon_sm() -> Pixels {
        px(14.0)
    }
    pub fn icon_md() -> Pixels {
        px(16.0)
    } // Standard icon size
    pub fn icon_lg() -> Pixels {
        px(20.0)
    }

    pub fn button_height() -> Pixels {
        px(28.0)
    }

    pub fn status_dot() -> Pixels {
        px(8.0)
    }
}

// =============================================================================
// Typography
// =============================================================================

pub mod typography {
    use gpui::{Pixels, px};

    pub fn text_xs() -> Pixels {
        px(10.0)
    }
    pub fn text_sm() -> Pixels {
        px(12.0)
    } // Standard UI text
}

// =============================================================================
// Fonts
// =============================================================================

pub mod fonts {
    use gpui::relative;

    pub fn ui() -> &'static str {
        "JetBrainsMono Nerd Font"
    }
    pub fn heading() -> &'static str {
        "JetBrainsMono Nerd Font"
    }
    pub fn mono() -> &'static str {
        "JetBrainsMono Nerd Font Mono"
    }
    pub fn ui_line_height() -> gpui::DefiniteLength {
        relative(1.45)
    }
}

// =============================================================================
// Borders
// =============================================================================

pub mod borders {
    use gpui::{Pixels, px};

    pub fn radius_sm() -> Pixels {
        px(3.0)
    } // Subtler rounded corners
}
