//! Integration tests for settings and workspace serialization.
//!
//! No MongoDB container needed — pure serialization/deserialization tests.

use openmango::state::CollectionSubview;
use openmango::state::settings::{
    AppSettings, AppTheme, DEFAULT_FILENAME_TEMPLATE, expand_filename_template,
};
use openmango::state::workspace::{WorkspaceTab, WorkspaceTabKind};

// =============================================================================
// Default settings verification
// =============================================================================

#[test]
fn test_default_settings() {
    let settings = AppSettings::default();

    // Appearance defaults
    assert_eq!(settings.appearance.theme, AppTheme::VercelDark);
    assert!(settings.appearance.show_status_bar);
    assert!(!settings.appearance.vibrancy);

    // Transfer defaults
    assert_eq!(settings.transfer.default_batch_size, 1000);
    assert_eq!(settings.transfer.export_filename_template, DEFAULT_FILENAME_TEMPLATE);
    assert!(settings.transfer.default_export_folder.is_empty());
}

// =============================================================================
// expand_filename_template
// =============================================================================

#[test]
fn test_expand_filename_template() {
    // Static placeholders only
    let result = expand_filename_template("${database}_${collection}", "mydb", "users");
    assert_eq!(result, "mydb_users");

    // No placeholders — returned as-is
    let result2 = expand_filename_template("export", "mydb", "users");
    assert_eq!(result2, "export");

    // Template with datetime — just verify the database/collection parts
    let result3 = expand_filename_template(DEFAULT_FILENAME_TEMPLATE, "testdb", "orders");
    assert!(result3.starts_with("testdb_orders_"));
    // The datetime portion should be non-empty
    assert!(result3.len() > "testdb_orders_".len());

    // Only ${date} placeholder
    let result4 = expand_filename_template("backup_${date}", "mydb", "users");
    assert!(result4.starts_with("backup_"));
    // Date format: YYYY-MM-DD = 10 chars
    assert_eq!(result4.len(), "backup_".len() + 10);
}

// =============================================================================
// WorkspaceTab — backward compat: missing fields use defaults
// =============================================================================

#[test]
fn test_workspace_tab_deserialize_missing_fields() {
    // Simulate an older workspace format missing `forge_content`
    let raw = r#"{
        "database": "admin",
        "collection": "",
        "kind": "Database",
        "transfer": null,
        "filter_raw": "",
        "sort_raw": "",
        "projection_raw": "",
        "aggregation_pipeline": [],
        "stats_open": false,
        "subview": "Documents"
    }"#;

    let tab: WorkspaceTab = serde_json::from_str(raw).expect("should deserialize");
    assert!(tab.forge_content.is_empty());
    assert_eq!(tab.kind, WorkspaceTabKind::Database);
    assert_eq!(tab.subview, CollectionSubview::Documents);

    // Even older format: missing `kind` as well
    let raw2 = r#"{
        "database": "test",
        "collection": "users",
        "filter_raw": "{}",
        "sort_raw": "",
        "projection_raw": ""
    }"#;

    let tab2: WorkspaceTab = serde_json::from_str(raw2).expect("should deserialize");
    assert_eq!(tab2.kind, WorkspaceTabKind::Collection); // default
    assert_eq!(tab2.database, "test");
    assert_eq!(tab2.collection, "users");
}

// =============================================================================
// WorkspaceTab — Forge kind roundtrip
// =============================================================================

#[test]
fn test_workspace_tab_forge_roundtrip() {
    let tab = WorkspaceTab {
        database: "admin".to_string(),
        collection: String::new(),
        kind: WorkspaceTabKind::Forge,
        transfer: None,
        filter_raw: String::new(),
        sort_raw: String::new(),
        projection_raw: String::new(),
        aggregation_pipeline: Vec::new(),
        stats_open: false,
        subview: CollectionSubview::Documents,
        forge_content: "db.getCollection(\"users\").find({})".to_string(),
    };

    let json = serde_json::to_string(&tab).expect("should serialize");
    let decoded: WorkspaceTab = serde_json::from_str(&json).expect("should deserialize");

    assert_eq!(decoded.kind, WorkspaceTabKind::Forge);
    assert_eq!(decoded.forge_content, tab.forge_content);
    assert_eq!(decoded.database, "admin");
}

// =============================================================================
// AppTheme — theme_id() roundtrip via from_theme_id()
// =============================================================================

#[test]
fn test_app_theme_id_roundtrip() {
    let all_themes: Vec<AppTheme> =
        AppTheme::dark_themes().iter().chain(AppTheme::light_themes()).copied().collect();

    for theme in &all_themes {
        let id = theme.theme_id();
        let restored = AppTheme::from_theme_id(id);
        assert_eq!(restored, Some(*theme), "theme_id roundtrip failed for {:?} (id={})", theme, id);
    }

    // Unknown theme_id → None
    assert_eq!(AppTheme::from_theme_id("nonexistent-theme"), None);
    assert_eq!(AppTheme::from_theme_id(""), None);
}
