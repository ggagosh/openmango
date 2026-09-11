//! Application settings with persistence.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::app_state::updater::UpdateChannel;
use super::app_state::{InsertMode, TransferFormat};
use crate::ai::settings::AiSettings;

/// Application settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub appearance: AppearanceSettings,
    #[serde(default)]
    pub transfer: TransferSettings,
    #[serde(default)]
    pub ai: AiSettings,
    #[serde(default)]
    pub keybindings: KeybindingSettings,
    #[serde(default)]
    pub mcp: McpSettings,
    #[serde(default = "default_interactive_query_timeout_ms")]
    pub interactive_query_timeout_ms: u64,
    #[serde(default = "default_current_version")]
    pub last_seen_version: String,
    #[serde(default = "default_true")]
    pub auto_update: bool,
    #[serde(default)]
    pub update_channel: UpdateChannel,
    #[serde(default)]
    pub collection_double_click_action: CollectionDoubleClickAction,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            appearance: AppearanceSettings::default(),
            transfer: TransferSettings::default(),
            ai: AiSettings::default(),
            keybindings: KeybindingSettings::default(),
            mcp: McpSettings::default(),
            interactive_query_timeout_ms: default_interactive_query_timeout_ms(),
            last_seen_version: default_current_version(),
            auto_update: true,
            update_channel: UpdateChannel::default(),
            collection_double_click_action: CollectionDoubleClickAction::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionDoubleClickAction {
    #[default]
    Data,
    Forge,
}

impl CollectionDoubleClickAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Data => "Open Data",
            Self::Forge => "Open Forge",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub grants: Vec<McpClientGrant>,
    #[serde(default = "default_true")]
    pub legacy_access: bool,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self { enabled: false, port: 0, grants: Vec::new(), legacy_access: true }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpClientKind {
    #[default]
    Pi,
    ClaudeCode,
    Codex,
    Cursor,
    VsCode,
}

impl McpClientKind {
    pub const ALL: [Self; 5] =
        [Self::Pi, Self::ClaudeCode, Self::Codex, Self::Cursor, Self::VsCode];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pi => "Pi",
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex / ChatGPT",
            Self::Cursor => "Cursor",
            Self::VsCode => "VS Code / Copilot",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpClientGrant {
    pub id: Uuid,
    pub label: String,
    #[serde(default)]
    pub client: McpClientKind,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
}

impl McpClientGrant {
    pub fn active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

/// User overrides keyed by a stable binding ID.
/// Missing IDs use defaults, `null` disables a binding, and strings replace it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeybindingSettings {
    #[serde(default)]
    pub overrides: BTreeMap<String, Option<String>>,
}

fn default_interactive_query_timeout_ms() -> u64 {
    30_000
}

fn default_current_version() -> String {
    // Empty string so that upgrading users (whose JSON lacks this field)
    // will see the changelog on first launch after the update.
    String::new()
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
    #[serde(default)]
    pub islands: IslandsAppearanceSettings,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: AppTheme::default(),
            show_status_bar: true,
            vibrancy: false,
            islands: IslandsAppearanceSettings::default(),
        }
    }
}

/// Islands-specific appearance settings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct IslandsAppearanceSettings {
    #[serde(default = "default_true")]
    pub different_tool_window_background: bool,
    #[serde(default)]
    pub tab_style: IslandsTabStyle,
    #[serde(default)]
    pub corner_softness: IslandsCornerSoftness,
    #[serde(default, alias = "tab_style_migrated_to_datagrip")]
    pub tab_style_migrated_to_islands: bool,
}

impl Default for IslandsAppearanceSettings {
    fn default() -> Self {
        Self {
            different_tool_window_background: true,
            tab_style: IslandsTabStyle::default(),
            corner_softness: IslandsCornerSoftness::default(),
            tab_style_migrated_to_islands: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum IslandsTabStyle {
    #[default]
    #[serde(rename = "Islands", alias = "DataGrip")]
    Islands,
    Segmented,
    Underline,
}

impl IslandsTabStyle {
    pub fn label(self) -> &'static str {
        match self {
            IslandsTabStyle::Islands => "Islands",
            IslandsTabStyle::Segmented => "Segmented",
            IslandsTabStyle::Underline => "Underline",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum IslandsCornerSoftness {
    Compact,
    #[default]
    Medium,
    Soft,
}

impl IslandsCornerSoftness {
    pub fn label(self) -> &'static str {
        match self {
            IslandsCornerSoftness::Compact => "Compact",
            IslandsCornerSoftness::Medium => "Medium",
            IslandsCornerSoftness::Soft => "Soft",
        }
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

pub fn migrate_islands_tab_style_to_islands(settings: &mut AppSettings) -> bool {
    if settings.appearance.islands.tab_style_migrated_to_islands {
        return false;
    }

    if settings.appearance.islands.tab_style == IslandsTabStyle::Segmented {
        settings.appearance.islands.tab_style = IslandsTabStyle::Islands;
    }
    settings.appearance.islands.tab_style_migrated_to_islands = true;
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
        assert!(settings.appearance.islands.different_tool_window_background);
        assert_eq!(settings.appearance.islands.tab_style, IslandsTabStyle::Islands);
        assert_eq!(settings.appearance.islands.corner_softness, IslandsCornerSoftness::Medium);
        assert!(settings.appearance.islands.tab_style_migrated_to_islands);
        assert_eq!(settings.transfer.default_batch_size, 1000);
        assert_eq!(settings.transfer.export_filename_template, DEFAULT_FILENAME_TEMPLATE);
        assert!(settings.keybindings.overrides.is_empty());
        assert!(!settings.mcp.enabled);
        assert_eq!(settings.mcp.port, 0);
        assert!(settings.mcp.grants.is_empty());
        assert!(settings.mcp.legacy_access);
        assert!(!settings.ai.enabled);
        assert_eq!(settings.ai.model, "gemini-3-flash-preview");
        assert_eq!(settings.interactive_query_timeout_ms, 30_000);
    }

    #[test]
    fn keybinding_overrides_round_trip_and_missing_settings_default() {
        let mut settings = AppSettings::default();
        settings
            .keybindings
            .overrides
            .insert("open-settings.workspace".into(), Some("cmd-alt-s".into()));
        settings.keybindings.overrides.insert("unknown.future-binding".into(), None);

        let json = serde_json::to_string(&settings).unwrap();
        let restored: AppSettings = serde_json::from_str(&json).unwrap();
        let legacy: AppSettings = serde_json::from_str("{}").unwrap();

        assert_eq!(restored.keybindings, settings.keybindings);
        assert!(legacy.keybindings.overrides.is_empty());
    }

    #[test]
    fn mcp_grants_round_trip_without_tokens() {
        let mut settings = AppSettings::default();
        settings.mcp.grants.push(McpClientGrant {
            id: Uuid::new_v4(),
            label: "Pi".into(),
            client: McpClientKind::Pi,
            created_at: Utc::now(),
            last_used_at: None,
            revoked_at: None,
        });

        let json = serde_json::to_string(&settings).unwrap();
        let restored: AppSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.mcp.grants, settings.mcp.grants);
        assert!(!json.contains("token"));
    }

    #[test]
    fn legacy_mcp_grants_default_to_pi() {
        let id = Uuid::new_v4();
        let json = format!(r#"{{"id":"{id}","label":"Pi","created_at":"2026-01-01T00:00:00Z"}}"#);

        let grant: McpClientGrant = serde_json::from_str(&json).unwrap();

        assert_eq!(grant.client, McpClientKind::Pi);
    }

    #[test]
    fn test_migrate_islands_tab_style_to_islands() {
        let mut settings = AppSettings::default();
        settings.appearance.islands.tab_style = IslandsTabStyle::Segmented;
        settings.appearance.islands.tab_style_migrated_to_islands = false;

        let changed = migrate_islands_tab_style_to_islands(&mut settings);
        assert!(changed);
        assert_eq!(settings.appearance.islands.tab_style, IslandsTabStyle::Islands);
        assert!(settings.appearance.islands.tab_style_migrated_to_islands);
    }

    #[test]
    fn test_legacy_datagrip_style_deserializes_to_islands() {
        let raw = r#"{
            "appearance": {
                "theme": "VercelDark",
                "islands": {
                    "tab_style": "DataGrip"
                }
            }
        }"#;

        let settings: AppSettings = serde_json::from_str(raw).expect("should deserialize");
        assert_eq!(settings.appearance.islands.tab_style, IslandsTabStyle::Islands);
        assert_eq!(settings.interactive_query_timeout_ms, 30_000);
    }

    #[test]
    fn test_legacy_migration_flag_deserializes() {
        let raw = r#"{
            "appearance": {
                "theme": "VercelDark",
                "islands": {
                    "tab_style_migrated_to_datagrip": false
                }
            }
        }"#;

        let settings: AppSettings = serde_json::from_str(raw).expect("should deserialize");
        assert!(!settings.appearance.islands.tab_style_migrated_to_islands);
    }
}
