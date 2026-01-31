//! Application settings with persistence.

use serde::{Deserialize, Serialize};

use super::app_state::{InsertMode, TransferFormat};

/// Application settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub appearance: AppearanceSettings,
    pub transfer: TransferSettings,
}

/// Appearance settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default = "default_true")]
    pub show_status_bar: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self { theme: Theme::default(), show_status_bar: true }
    }
}

/// Application theme
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    System,
}

impl Theme {
    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::System => "System",
        }
    }

    pub fn all() -> &'static [Theme] {
        &[Theme::Dark, Theme::Light, Theme::System]
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

/// Default filename template constant
pub const DEFAULT_FILENAME_TEMPLATE: &str = "${database}_${collection}_${datetime}";

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
        assert_eq!(settings.appearance.theme, Theme::Dark);
        assert!(settings.appearance.show_status_bar);
        assert_eq!(settings.transfer.default_batch_size, 1000);
        assert_eq!(settings.transfer.export_filename_template, DEFAULT_FILENAME_TEMPLATE);
    }
}
