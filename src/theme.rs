// Design token system for OpenMango
// Theme colors are loaded from themes/*.json via gpui-component's theme system.
// Access them with `cx.theme().background`, `cx.theme().primary`, etc.
// This file only contains colors with no gpui-component equivalent, plus theme switching.

use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::theme::{ThemeConfig, ThemeSet};
use gpui_kit::{App, Entity, Hsla, Pixels, Styled as _, Window, WindowAppearance, px};

use crate::state::{AppState, AppTheme, AppearanceSettings, IslandsTabStyle};

// =============================================================================
// Theme Loading & Switching
// =============================================================================

const THEME_SOURCES: &[(&str, &str)] = &[
    ("mango-dark", include_str!("../themes/mango-dark.json")),
    ("mango-light", include_str!("../themes/mango-light.json")),
    ("vercel-dark", include_str!("../themes/vercel-dark.json")),
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

/// Keep application geometry and typography consistent across all color themes.
pub fn apply_design_tokens(cx: &mut App) {
    let theme = gpui_kit::component::theme::Theme::global_mut(cx);
    theme.font_family = fonts::ui().into();
    theme.mono_font_family = fonts::mono().into();
    theme.radius = borders::radius_sm();
    theme.radius_lg = borders::radius_md();
}

pub fn apply_theme(
    app_theme: AppTheme,
    vibrancy: bool,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) {
    if let Some(config) = load_theme_config(app_theme.theme_id()) {
        gpui_kit::component::theme::Theme::global_mut(cx).apply_config(&config);

        apply_design_tokens(cx);

        if vibrancy {
            apply_vibrancy(cx);
        }

        window.refresh();
    }
}

/// The theme to show: the Mango theme matching the system when following it, else the saved one.
pub fn resolved_theme(appearance: &AppearanceSettings, system: WindowAppearance) -> AppTheme {
    if !appearance.follow_system {
        return appearance.theme;
    }
    match system {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => AppTheme::MangoDark,
        WindowAppearance::Light | WindowAppearance::VibrantLight => AppTheme::MangoLight,
    }
}

/// Saves and applies a theme the user picked, which stops following the system appearance.
pub fn pick_theme(state: &Entity<AppState>, theme: AppTheme, window: &mut Window, cx: &mut App) {
    save_and_apply(state, theme, false, window, cx);
}

/// Turns system appearance following on, applying the matching Mango theme, or off.
pub fn set_follow_system(
    state: &Entity<AppState>,
    follow: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let mut appearance = state.read(cx).settings.appearance.clone();
    appearance.follow_system = follow;
    let theme = resolved_theme(&appearance, window.appearance());
    save_and_apply(state, theme, follow, window, cx);
}

/// Re-applies the matching Mango theme after the system appearance changes.
pub fn sync_system_theme(state: &Entity<AppState>, window: &mut Window, cx: &mut App) {
    let appearance = &state.read(cx).settings.appearance;
    let theme = resolved_theme(appearance, window.appearance());
    if appearance.follow_system && theme != appearance.theme {
        save_and_apply(state, theme, true, window, cx);
    }
}

fn save_and_apply(
    state: &Entity<AppState>,
    theme: AppTheme,
    follow_system: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let (user_vibrancy, startup_vibrancy) = state.update(cx, |state, cx| {
        state.settings.appearance.theme = theme;
        state.settings.appearance.follow_system = follow_system;
        state.save_settings();
        cx.notify();
        (state.settings.appearance.vibrancy, state.startup_vibrancy)
    });
    apply_theme(theme, effective_vibrancy(theme, user_vibrancy), window, cx);
    if requires_vibrancy_restart(startup_vibrancy, theme, user_vibrancy) {
        crate::components::open_confirm_dialog(
            window,
            cx,
            "Restart required",
            "Switching this theme changes window vibrancy mode. Restart now to fully apply it.",
            "Restart now",
            false,
            {
                let state = state.clone();
                move |window, cx| crate::components::request_app_quit(state.clone(), window, cx)
            },
        );
    }
}

pub fn effective_vibrancy(_app_theme: AppTheme, user_vibrancy: bool) -> bool {
    cfg!(target_os = "macos") && user_vibrancy
}

pub fn requires_vibrancy_restart(
    startup_vibrancy: bool,
    target_theme: AppTheme,
    user_vibrancy: bool,
) -> bool {
    startup_vibrancy != effective_vibrancy(target_theme, user_vibrancy)
}

/// Reduce alpha on background/sidebar so the macOS blur effect shows through.
pub fn apply_vibrancy(cx: &mut gpui_kit::App) {
    let theme = gpui_kit::component::theme::Theme::global_mut(cx);
    theme.background.a = 0.82;
    theme.sidebar.a = 0.82;
}

// =============================================================================
// Custom Colors (theme-aware)
// =============================================================================

pub mod colors {
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::{App, Hsla};

    use crate::models::ConnectionColor;

    pub fn connection_accent(color: ConnectionColor, cx: &App) -> Hsla {
        match color {
            ConnectionColor::Red => cx.theme().red,
            ConnectionColor::Yellow => cx.theme().yellow,
            ConnectionColor::Green => cx.theme().green,
            ConnectionColor::Cyan => cx.theme().cyan,
            ConnectionColor::Blue => cx.theme().blue,
            ConnectionColor::Magenta => cx.theme().magenta,
        }
    }

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
        gpui_kit::hsla(0.0, 0.0, 0.0, 0.0)
    }

    // Modal backdrop — theme background darkened with alpha
    pub fn backdrop(cx: &App) -> Hsla {
        let mut c = cx.theme().background;
        c.a = 0.85;
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
// Islands helpers
// =============================================================================

pub mod islands {
    use super::*;

    pub fn tab_bar(bar: TabBar, appearance: &AppearanceSettings) -> TabBar {
        let bar = bar.min_w(px(0.0)).max_width(px(260.0)).rounded(borders::radius_sm());
        match appearance.islands.tab_style {
            // Keep the native selected surface neutral so connection colors and
            // status badges remain legible in the surrounding application theme.
            IslandsTabStyle::Islands => bar.segmented().bg(colors::transparent()),
            IslandsTabStyle::Segmented => bar.segmented(),
            IslandsTabStyle::Underline => bar.underline(),
        }
    }

    pub fn radius_sm(appearance: &AppearanceSettings) -> Pixels {
        let _ = appearance;
        borders::radius_sm()
    }

    pub fn radius_md(appearance: &AppearanceSettings) -> Pixels {
        let _ = appearance;
        borders::radius_md()
    }

    pub fn panel_border(_appearance: &AppearanceSettings, cx: &App) -> Hsla {
        cx.theme().sidebar_border
    }

    pub fn canvas_bg(_appearance: &AppearanceSettings, cx: &App) -> Hsla {
        cx.theme().tab_bar
    }

    pub fn tool_bg(_appearance: &AppearanceSettings, cx: &App) -> Hsla {
        cx.theme().sidebar
    }

    pub fn content_bg(_appearance: &AppearanceSettings, cx: &App) -> Hsla {
        cx.theme().background
    }

    pub fn card_bg(_appearance: &AppearanceSettings, cx: &App) -> Hsla {
        cx.theme().tab_bar
    }

    pub fn ai_shell_bg(_appearance: &AppearanceSettings, cx: &App) -> Hsla {
        cx.theme().sidebar
    }

    pub fn ai_header_bg(_appearance: &AppearanceSettings, cx: &App) -> Hsla {
        cx.theme().sidebar.opacity(0.92)
    }

    pub fn ai_surface_bg(_appearance: &AppearanceSettings, cx: &App) -> Hsla {
        cx.theme().tab_bar.opacity(0.82)
    }

    pub fn ai_surface_muted_bg(_appearance: &AppearanceSettings, cx: &App) -> Hsla {
        cx.theme().tab_bar.opacity(0.62)
    }

    pub fn ai_border(_appearance: &AppearanceSettings, cx: &App) -> Hsla {
        cx.theme().sidebar_border.opacity(0.78)
    }
}

#[cfg(test)]
mod tests {
    use super::{effective_vibrancy, requires_vibrancy_restart};
    use crate::state::AppTheme;

    #[gpui_kit::test]
    fn every_color_theme_uses_the_shared_radius_scale(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            for (id, _) in super::THEME_SOURCES {
                let config = super::load_theme_config(id).expect("valid bundled theme");
                let theme = gpui_kit::component::Theme::global_mut(cx);
                theme.radius = gpui_kit::px(30.);
                theme.radius_lg = gpui_kit::px(40.);
                theme.apply_config(&config);
                super::apply_design_tokens(cx);
                let theme = gpui_kit::component::Theme::global(cx);
                assert_eq!(theme.radius, super::borders::radius_sm(), "{id}");
                assert_eq!(theme.radius_lg, super::borders::radius_md(), "{id}");
                let base = gpui_kit::base::Theme::global(cx);
                assert_eq!(base.tokens.radius.sm, super::borders::radius_xs(), "{id}");
                assert_eq!(base.tokens.radius.md, super::borders::radius_sm(), "{id}");
                assert_eq!(base.tokens.radius.lg, super::borders::radius_md(), "{id}");
            }
        });
    }

    #[test]
    fn every_listed_theme_loads_and_mango_leads_each_mode() {
        let dark = AppTheme::dark_themes();
        let light = AppTheme::light_themes();
        assert_eq!(
            (AppTheme::default(), dark[0], light[0]),
            (AppTheme::MangoDark, AppTheme::MangoDark, AppTheme::MangoLight)
        );
        for (themes, is_dark) in [(dark, true), (light, false)] {
            for theme in themes {
                let config = super::load_theme_config(theme.theme_id())
                    .unwrap_or_else(|| panic!("{} has no bundled theme", theme.theme_id()));
                assert_eq!(config.mode.is_dark(), is_dark, "{}", theme.theme_id());
            }
        }
    }

    /// The Mango themes are ours, so their text colors must meet WCAG AA on the
    /// surfaces they render on: content, sidebar and hover.
    #[gpui_kit::test]
    fn mango_theme_text_meets_contrast_on_every_surface(cx: &mut gpui_kit::TestAppContext) {
        fn luminance(color: gpui_kit::Hsla) -> f32 {
            let rgb = color.to_rgb();
            let channel = |c: f32| {
                if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * channel(rgb.r) + 0.7152 * channel(rgb.g) + 0.0722 * channel(rgb.b)
        }
        fn ratio(a: gpui_kit::Hsla, b: gpui_kit::Hsla) -> f32 {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        }

        cx.update(|cx| {
            gpui_kit::init(cx);
            for theme_id in [AppTheme::MangoDark.theme_id(), AppTheme::MangoLight.theme_id()] {
                let config = super::load_theme_config(theme_id).expect("bundled theme");
                gpui_kit::component::Theme::global_mut(cx).apply_config(&config);
                let t = gpui_kit::component::Theme::global(cx);
                let surfaces =
                    [("background", t.background), ("sidebar", t.sidebar), ("hover", t.accent)];
                for (surface, bg) in surfaces {
                    let text = [
                        ("foreground", t.foreground, 7.0),
                        ("muted", t.muted_foreground, 4.5),
                        ("link", t.link, 4.5),
                        ("red", t.red, 4.5),
                        ("yellow", t.yellow, 4.5),
                        ("green", t.green, 4.5),
                        ("blue", t.blue, 4.5),
                        ("magenta", t.magenta, 4.5),
                        ("cyan", t.cyan, 4.5),
                    ];
                    for (name, fg, min) in text {
                        let got = ratio(fg, bg);
                        assert!(
                            got >= min,
                            "{theme_id}: {name} on {surface} is {got:.2}, needs {min}"
                        );
                    }
                }
                let fills = [
                    ("primary", t.primary_foreground, t.primary),
                    ("primary hover", t.primary_foreground, t.primary_hover),
                    ("danger", t.danger_foreground, t.danger),
                    ("warning", t.warning_foreground, t.warning),
                    ("success", t.success_foreground, t.success),
                    ("info", t.info_foreground, t.info),
                ];
                for (name, fg, bg) in fills {
                    let got = ratio(fg, bg);
                    assert!(got >= 4.5, "{theme_id}: text on {name} is {got:.2}, needs 4.5");
                }
                let ring = ratio(t.ring, t.background);
                assert!(ring >= 3.0, "{theme_id}: focus ring is {ring:.2}, needs 3");
            }
        });
    }

    #[test]
    fn following_the_system_picks_the_matching_mango_theme() {
        use gpui_kit::WindowAppearance::{Dark, VibrantDark, VibrantLight};
        let mut appearance =
            crate::state::AppearanceSettings { theme: AppTheme::Nord, ..Default::default() };
        assert!(appearance.follow_system, "new installs follow the system");
        assert_eq!(super::resolved_theme(&appearance, Dark), AppTheme::MangoDark);
        assert_eq!(super::resolved_theme(&appearance, VibrantDark), AppTheme::MangoDark);
        assert_eq!(super::resolved_theme(&appearance, VibrantLight), AppTheme::MangoLight);
        appearance.follow_system = false;
        assert_eq!(super::resolved_theme(&appearance, Dark), AppTheme::Nord);
    }

    #[test]
    fn vibrancy_follows_user_toggle() {
        assert!(!effective_vibrancy(AppTheme::VercelDark, false));
        assert_eq!(effective_vibrancy(AppTheme::VercelDark, true), cfg!(target_os = "macos"));
    }

    #[test]
    fn restart_required_when_vibrancy_changes() {
        assert!(requires_vibrancy_restart(true, AppTheme::VercelDark, false));
        assert_eq!(
            requires_vibrancy_restart(false, AppTheme::VercelDark, true),
            cfg!(target_os = "macos")
        );
        assert!(!requires_vibrancy_restart(false, AppTheme::VercelDark, false));
        assert_eq!(
            requires_vibrancy_restart(true, AppTheme::VercelDark, true),
            !cfg!(target_os = "macos")
        );
    }
}

// =============================================================================
// Spacing
// =============================================================================

pub mod spacing {
    use gpui_kit::{Pixels, px};

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
    use gpui_kit::{Pixels, px};

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

    pub fn status_dot() -> Pixels {
        px(8.0)
    }
}

// =============================================================================
// Typography
// =============================================================================

pub mod typography {
    use gpui_kit::{Pixels, px};

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
    use gpui_kit::relative;

    pub fn ui() -> &'static str {
        "JetBrains Mono"
    }
    pub fn heading() -> &'static str {
        "JetBrains Mono"
    }
    pub fn mono() -> &'static str {
        "JetBrains Mono"
    }
    pub fn tabs() -> &'static str {
        ui()
    }
    pub fn ui_line_height() -> gpui_kit::DefiniteLength {
        relative(1.45)
    }
}

// =============================================================================
// Borders
// =============================================================================

pub mod borders {
    use gpui_kit::{Pixels, px};

    /// Small indicators and chart marks; matches the toolkit's small radius tier.
    pub fn radius_xs() -> Pixels {
        radius_sm() / 2.0
    }

    /// Buttons, inputs, tabs, rows, menus, and small labels.
    pub fn radius_sm() -> Pixels {
        px(6.0)
    }

    /// Panels, dialogs, cards, and other larger surfaces.
    pub fn radius_md() -> Pixels {
        px(8.0)
    }
}
