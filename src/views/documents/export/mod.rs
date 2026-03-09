//! WYSIWYG clipboard export — copy documents in multiple formats.

mod formats;
mod snapshot;

pub use formats::render_to_clipboard;
pub use snapshot::{ExportScope, ViewExportSnapshot};

use gpui_component::{Icon, IconName};
use serde::{Deserialize, Serialize};

/// Clipboard copy format — persisted as global preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CopyFormat {
    #[default]
    Json,
    JsonLines,
    Csv,
    Markdown,
    Tsv,
}

impl CopyFormat {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::JsonLines => "JSONL",
            Self::Csv => "CSV",
            Self::Markdown => "Markdown",
            Self::Tsv => "TSV",
        }
    }

    pub fn icon(&self) -> Icon {
        match self {
            Self::Json | Self::JsonLines => Icon::new(IconName::Braces),
            Self::Csv => Icon::new(IconName::File).path("icons/file-spreadsheet.svg"),
            Self::Markdown => Icon::new(IconName::File).path("icons/file-text.svg"),
            Self::Tsv => Icon::new(IconName::File).path("icons/table-2.svg"),
        }
    }

    pub fn all() -> &'static [CopyFormat] {
        &[
            CopyFormat::Json,
            CopyFormat::JsonLines,
            CopyFormat::Csv,
            CopyFormat::Markdown,
            CopyFormat::Tsv,
        ]
    }

    pub fn tree_formats() -> &'static [CopyFormat] {
        &[CopyFormat::Json, CopyFormat::JsonLines]
    }

    pub fn table_formats() -> &'static [CopyFormat] {
        &[
            CopyFormat::Json,
            CopyFormat::JsonLines,
            CopyFormat::Csv,
            CopyFormat::Markdown,
            CopyFormat::Tsv,
        ]
    }
}
