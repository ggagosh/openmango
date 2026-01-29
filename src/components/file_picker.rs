//! File picker utilities using rfd crate for native file dialogs.

use std::path::PathBuf;

/// File picker mode - determines whether we're opening or saving.
#[derive(Clone, Copy, Debug)]
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

/// Open a file dialog synchronously.
/// Returns None if the user cancelled.
pub fn open_file_dialog(
    mode: FilePickerMode,
    filters: Vec<FileFilter>,
    default_name: Option<&str>,
) -> Option<PathBuf> {
    match mode {
        FilePickerMode::Open => {
            let mut dialog = rfd::FileDialog::new();

            for filter in &filters {
                let extensions: Vec<&str> = filter.extensions.iter().map(|s| s.as_str()).collect();
                dialog = dialog.add_filter(&filter.name, &extensions);
            }

            dialog.pick_file()
        }
        FilePickerMode::Save => {
            let mut dialog = rfd::FileDialog::new();

            for filter in &filters {
                let extensions: Vec<&str> = filter.extensions.iter().map(|s| s.as_str()).collect();
                dialog = dialog.add_filter(&filter.name, &extensions);
            }

            if let Some(name) = default_name {
                dialog = dialog.set_file_name(name);
            }

            dialog.save_file()
        }
    }
}

/// Open a folder picker dialog synchronously.
/// Returns None if the user cancelled.
#[allow(dead_code)]
pub fn open_folder_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
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
pub fn default_export_filename(
    database: &str,
    collection: &str,
    format: crate::state::TransferFormat,
) -> String {
    let base = if collection.is_empty() { database.to_string() } else { collection.to_string() };

    format!("{}.{}", base, format.extension())
}
