//! WYSIWYG clipboard export — copy documents in multiple formats.

pub mod file_export;
mod formats;
mod snapshot;

pub use file_export::FileExportFormat;
pub use formats::render_to_clipboard;
pub use snapshot::{ExportScope, ViewExportSnapshot};

use gpui_kit::component::{Icon, IconName};
use serde::{Deserialize, Serialize};

/// Clipboard copy format — persisted as global preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CopyFormat {
    #[default]
    Json,
    /// Plain values with no type wrappers: what "normal JSON" means to everything that is
    /// not MongoDB. One way only; ids and dates come back as strings.
    PlainJson,
    JsonLines,
    Csv,
    Markdown,
    Tsv,
}

impl CopyFormat {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Json => "Extended JSON",
            Self::PlainJson => "Plain JSON",
            Self::JsonLines => "JSONL",
            Self::Csv => "CSV",
            Self::Markdown => "Markdown",
            Self::Tsv => "TSV",
        }
    }

    pub fn icon(&self) -> Icon {
        match self {
            Self::Json | Self::PlainJson | Self::JsonLines => {
                Icon::new(crate::assets::AppIcon::Braces)
            }
            Self::Csv => Icon::new(IconName::File).path("icons/file-spreadsheet.svg"),
            Self::Markdown => Icon::new(IconName::File).path("icons/file-text.svg"),
            Self::Tsv => Icon::new(IconName::File).path("icons/table-2.svg"),
        }
    }

    pub fn all() -> &'static [CopyFormat] {
        &[
            CopyFormat::Json,
            CopyFormat::PlainJson,
            CopyFormat::JsonLines,
            CopyFormat::Csv,
            CopyFormat::Markdown,
            CopyFormat::Tsv,
        ]
    }

    pub fn tree_formats() -> &'static [CopyFormat] {
        &[CopyFormat::Json, CopyFormat::PlainJson, CopyFormat::JsonLines]
    }

    pub fn table_formats() -> &'static [CopyFormat] {
        &[
            CopyFormat::Json,
            CopyFormat::PlainJson,
            CopyFormat::JsonLines,
            CopyFormat::Csv,
            CopyFormat::Markdown,
            CopyFormat::Tsv,
        ]
    }
}
