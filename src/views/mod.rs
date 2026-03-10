// Screen/page components
pub mod ai;
pub mod databases;
pub mod documents;
pub mod forge;
pub mod json_editor_detached;
pub mod results;
pub mod settings;
pub mod transfer;

pub use crate::changelog::ChangelogView;
pub use ai::AiView;
pub use databases::DatabaseView;
pub use documents::CollectionView;
pub use forge::ForgeView;
pub use json_editor_detached::DetachedJsonEditorView;
pub use settings::SettingsView;
pub use transfer::TransferView;
