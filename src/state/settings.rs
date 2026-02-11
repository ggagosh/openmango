//! Application settings with persistence.

use serde::{Deserialize, Serialize};

use super::app_state::{InsertMode, TransferFormat};

/// Application settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub appearance: AppearanceSettings,
    pub transfer: TransferSettings,
    #[serde(default = "default_current_version")]
    pub last_seen_version: String,
}

fn default_current_version() -> String {
    env!("OPENMANGO_GIT_SHA").to_string()
}

/// Appearance settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    #[serde(default)]
    pub theme: AppTheme,
    #[serde(default = "default_true")]
    pub show_status_bar: bool,
    #[serde(default)]
    pub vibrancy: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self { theme: AppTheme::default(), show_status_bar: true, vibrancy: false }
    }
}

/// Application theme
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum AppTheme {
    #[default]
    VercelDark,
    DarculaDark,
    TokyoNight,
    Nord,
    OneDark,
    CatppuccinMocha,
    CatppuccinLatte,
    SolarizedLight,
    SolarizedDark,
    RosePineDawn,
    RosePine,
    GruvboxLight,
    GruvboxDark,
}

impl AppTheme {
    pub fn label(self) -> &'static str {
        match self {
            AppTheme::VercelDark => "Vercel Dark",
            AppTheme::DarculaDark => "Darcula Dark",
            AppTheme::TokyoNight => "Tokyo Night",
            AppTheme::Nord => "Nord",
            AppTheme::OneDark => "One Dark",
            AppTheme::CatppuccinMocha => "Catppuccin Mocha",
            AppTheme::CatppuccinLatte => "Catppuccin Latte",
            AppTheme::SolarizedLight => "Solarized Light",
            AppTheme::SolarizedDark => "Solarized Dark",
            AppTheme::RosePineDawn => "Rosé Pine Dawn",
            AppTheme::RosePine => "Rosé Pine",
            AppTheme::GruvboxLight => "Gruvbox Light",
            AppTheme::GruvboxDark => "Gruvbox Dark",
        }
    }

    pub fn theme_id(self) -> &'static str {
        match self {
            AppTheme::VercelDark => "vercel-dark",
            AppTheme::DarculaDark => "darcula-dark",
            AppTheme::TokyoNight => "tokyo-night",
            AppTheme::Nord => "nord",
            AppTheme::OneDark => "one-dark",
            AppTheme::CatppuccinMocha => "catppuccin-mocha",
            AppTheme::CatppuccinLatte => "catppuccin-latte",
            AppTheme::SolarizedLight => "solarized-light",
            AppTheme::SolarizedDark => "solarized-dark",
            AppTheme::RosePineDawn => "rose-pine-dawn",
            AppTheme::RosePine => "rose-pine",
            AppTheme::GruvboxLight => "gruvbox-light",
            AppTheme::GruvboxDark => "gruvbox-dark",
        }
    }

    pub fn from_theme_id(id: &str) -> Option<AppTheme> {
        Self::dark_themes().iter().chain(Self::light_themes()).find(|t| t.theme_id() == id).copied()
    }

    pub fn dark_themes() -> &'static [AppTheme] {
        &[
            AppTheme::VercelDark,
            AppTheme::DarculaDark,
            AppTheme::TokyoNight,
            AppTheme::Nord,
            AppTheme::OneDark,
            AppTheme::CatppuccinMocha,
            AppTheme::SolarizedDark,
            AppTheme::RosePine,
            AppTheme::GruvboxDark,
        ]
    }

    pub fn light_themes() -> &'static [AppTheme] {
        &[
            AppTheme::CatppuccinLatte,
            AppTheme::SolarizedLight,
            AppTheme::RosePineDawn,
            AppTheme::GruvboxLight,
        ]
    }
}

/// Transfer (import/export) default settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferSettings {
    #[serde(default)]
    pub default_export_format: TransferFormat,
    #[serde(default = "default_batch_size")]
    pub default_batch_size: u32,
    #[serde(default)]
    pub default_import_mode: InsertMode,
    #[serde(default)]
    pub default_export_folder: String,
    #[serde(default = "default_filename_template")]
    pub export_filename_template: String,
}

impl Default for TransferSettings {
    fn default() -> Self {
        Self {
            default_export_format: TransferFormat::default(),
            default_batch_size: default_batch_size(),
            default_import_mode: InsertMode::default(),
            default_export_folder: String::new(),
            export_filename_template: default_filename_template(),
        }
    }
}

fn default_batch_size() -> u32 {
    1000
}

fn default_true() -> bool {
    true
}

fn default_filename_template() -> String {
    "${database}_${collection}_${datetime}".to_string()
}

/// Default filename template constant (for collection scope)
pub const DEFAULT_FILENAME_TEMPLATE: &str = "${database}_${collection}_${datetime}";

/// Filename template for database scope (excludes ${collection})
pub const DATABASE_SCOPE_FILENAME_TEMPLATE: &str = "${database}_${datetime}";

/// Available filename template placeholders
pub const FILENAME_PLACEHOLDERS: &[(&str, &str)] = &[
    ("${datetime}", "Date and time (2026-01-30_20-15-30)"),
    ("${date}", "Date only (2026-01-30)"),
    ("${time}", "Time only (20-15-30)"),
    ("${database}", "Database name"),
    ("${collection}", "Collection name"),
];

/// Expand filename template placeholders
pub fn expand_filename_template(template: &str, database: &str, collection: &str) -> String {
    let now = chrono::Local::now();

    template
        .replace("${datetime}", &now.format("%Y-%m-%d_%H-%M-%S").to_string())
        .replace("${date}", &now.format("%Y-%m-%d").to_string())
        .replace("${time}", &now.format("%H-%M-%S").to_string())
        .replace("${database}", database)
        .replace("${collection}", collection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_filename_template() {
        let result = expand_filename_template("${database}_${collection}", "mydb", "users");
        assert_eq!(result, "mydb_users");
    }

    #[test]
    fn test_default_settings() {
        let settings = AppSettings::default();
        assert_eq!(settings.appearance.theme, AppTheme::VercelDark);
        assert!(settings.appearance.show_status_bar);
        assert!(!settings.appearance.vibrancy);
        assert_eq!(settings.transfer.default_batch_size, 1000);
        assert_eq!(settings.transfer.export_filename_template, DEFAULT_FILENAME_TEMPLATE);
    }
}
