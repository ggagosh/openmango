//! File picker utilities using rfd crate for native file dialogs.

use std::path::PathBuf;

/// File picker mode - determines whether we're opening or saving.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub enum FilePickerMode {
    Open,
    Save,
}

/// File type filter for file dialogs.
#[derive(Clone, Debug)]
pub struct FileFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

impl FileFilter {
    pub fn new(name: impl Into<String>, extensions: Vec<&str>) -> Self {
        Self {
            name: name.into(),
            extensions: extensions.into_iter().map(|s| s.to_string()).collect(),
        }
    }

    /// JSON Lines filter
    pub fn json_lines() -> Self {
        Self::new("JSON Lines", vec!["jsonl", "ndjson"])
    }

    /// JSON Array filter
    pub fn json_array() -> Self {
        Self::new("JSON", vec!["json"])
    }

    /// CSV filter
    pub fn csv() -> Self {
        Self::new("CSV", vec!["csv"])
    }

    /// BSON Archive filter
    pub fn bson_archive() -> Self {
        Self::new("BSON Archive", vec!["archive", "bson"])
    }

    /// All supported import formats
    #[allow(dead_code)]
    pub fn all_import() -> Self {
        Self::new("All Supported", vec!["json", "jsonl", "ndjson", "csv"])
    }

    /// All files
    pub fn all() -> Self {
        Self::new("All Files", vec!["*"])
    }
}

/// Open a file dialog asynchronously.
/// Returns None if the user cancelled.
pub async fn open_file_dialog_async(
    mode: FilePickerMode,
    filters: Vec<FileFilter>,
    default_name: Option<String>,
) -> Option<PathBuf> {
    match mode {
        FilePickerMode::Open => {
            let mut dialog = rfd::AsyncFileDialog::new();

            for filter in &filters {
                let extensions: Vec<&str> = filter.extensions.iter().map(|s| s.as_str()).collect();
                dialog = dialog.add_filter(&filter.name, &extensions);
            }

            dialog.pick_file().await.map(|f| f.path().to_path_buf())
        }
        FilePickerMode::Save => {
            let mut dialog = rfd::AsyncFileDialog::new();

            for filter in &filters {
                let extensions: Vec<&str> = filter.extensions.iter().map(|s| s.as_str()).collect();
                dialog = dialog.add_filter(&filter.name, &extensions);
            }

            if let Some(name) = default_name {
                dialog = dialog.set_file_name(&name);
            }

            dialog.save_file().await.map(|f| f.path().to_path_buf())
        }
    }
}

/// Open a folder picker dialog asynchronously.
/// Returns None if the user cancelled.
#[allow(dead_code)]
pub async fn open_folder_dialog_async() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new().pick_folder().await.map(|f| f.path().to_path_buf())
}

/// Get file filters for a specific transfer format.
pub fn filters_for_format(format: crate::state::TransferFormat) -> Vec<FileFilter> {
    use crate::state::TransferFormat;

    match format {
        TransferFormat::JsonLines => vec![FileFilter::json_lines(), FileFilter::all()],
        TransferFormat::JsonArray => vec![FileFilter::json_array(), FileFilter::all()],
        TransferFormat::Csv => vec![FileFilter::csv(), FileFilter::all()],
        TransferFormat::Bson => vec![FileFilter::bson_archive(), FileFilter::all()],
    }
}

/// Generate a default filename for export based on database/collection.
#[allow(dead_code)]
pub fn default_export_filename(
    database: &str,
    collection: &str,
    format: crate::state::TransferFormat,
) -> String {
    let base = if collection.is_empty() { database.to_string() } else { collection.to_string() };

    format!("{}.{}", base, format.extension())
}

/// Generate a default filename for export using settings template.
#[allow(dead_code)]
pub fn default_export_filename_from_settings(
    settings: &crate::state::AppSettings,
    database: &str,
    collection: &str,
    format: crate::state::TransferFormat,
) -> String {
    let base = crate::state::expand_filename_template(
        &settings.transfer.export_filename_template,
        database,
        collection,
    );
    format!("{}.{}", base, format.extension())
}

/// Generate a default file path for export using settings.
#[allow(dead_code)]
pub fn default_export_path_from_settings(
    settings: &crate::state::AppSettings,
    database: &str,
    collection: &str,
    format: crate::state::TransferFormat,
) -> String {
    let filename = default_export_filename_from_settings(settings, database, collection, format);
    if settings.transfer.default_export_folder.is_empty() {
        filename
    } else {
        let path = std::path::Path::new(&settings.transfer.default_export_folder).join(&filename);
        path.display().to_string()
    }
}

/// Generate export filename from template WITHOUT expanding placeholders.
/// Used when user browses for folder - shows template with placeholders visible.
/// For BSON format, use `unexpanded_export_filename_bson` instead to specify output mode.
pub fn unexpanded_export_filename(
    settings: &crate::state::AppSettings,
    format: crate::state::TransferFormat,
) -> String {
    let template = &settings.transfer.export_filename_template;
    format!("{}.{}", template, format.extension())
}

/// Generate export filename for BSON format based on output mode.
/// - Archive mode: uses `.archive` extension
/// - Folder mode: no extension (template is the folder name)
pub fn unexpanded_export_filename_bson(
    settings: &crate::state::AppSettings,
    output_mode: crate::state::BsonOutputFormat,
) -> String {
    let template = &settings.transfer.export_filename_template;
    match output_mode {
        crate::state::BsonOutputFormat::Archive => format!("{}.archive", template),
        crate::state::BsonOutputFormat::Folder => template.clone(),
    }
}
