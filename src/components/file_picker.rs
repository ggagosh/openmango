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

    /// Excel filter
    pub fn excel() -> Self {
        Self::new("Excel", vec!["xlsx"])
    }

    /// OpenMango Connections (.json)
    pub fn connections_json() -> Self {
        Self::new("OpenMango Connections", vec!["json"])
    }

    /// OpenMango Query Library (.json)
    pub fn query_library_json() -> Self {
        Self::new("OpenMango Query Library", vec!["json"])
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

/// Database-scope template (excludes ${collection})
const DATABASE_SCOPE_TEMPLATE: &str = "${database}_${datetime}";

/// Generate export filename from template, using scope-appropriate template.
/// For database scope, uses a template without ${collection} and no extension (it's a directory).
/// For collection scope, uses the full template from settings with file extension.
pub fn unexpanded_export_filename_for_scope(
    settings: &crate::state::AppSettings,
    format: crate::state::TransferFormat,
    scope: crate::state::TransferScope,
) -> String {
    match scope {
        crate::state::TransferScope::Database => {
            // Database scope: path is a directory, no extension
            DATABASE_SCOPE_TEMPLATE.to_string()
        }
        crate::state::TransferScope::Collection => {
            // Collection scope: path is a file with extension
            format!("{}.{}", settings.transfer.export_filename_template, format.extension())
        }
    }
}

/// Generate export filename for BSON format based on output mode and scope.
/// For database scope, uses a template without ${collection}.
/// For collection scope, uses the full template from settings.
pub fn unexpanded_export_filename_bson_for_scope(
    settings: &crate::state::AppSettings,
    output_mode: crate::state::BsonOutputFormat,
    scope: crate::state::TransferScope,
) -> String {
    let template = match scope {
        crate::state::TransferScope::Database => DATABASE_SCOPE_TEMPLATE,
        crate::state::TransferScope::Collection => &settings.transfer.export_filename_template,
    };
    match output_mode {
        crate::state::BsonOutputFormat::Archive => format!("{}.archive", template),
        crate::state::BsonOutputFormat::Folder => template.to_string(),
    }
}
