// Reusable UI components

pub mod action_bar;
pub mod ai_blocks;
pub mod button;
pub mod confirm;
pub mod connection_dialog;
pub mod connection_identity;
pub mod connection_manager;
mod content;
pub mod dialog_helpers;
pub mod file_picker;
pub mod filter_builder;
pub mod form_field;
pub mod query_library;
mod status_bar;
mod unsaved_guard;
pub use button::Button;
pub(crate) use confirm::with_scoped_production_authorizations;
pub use confirm::{WriteConfirmation, WriteRequest, open_confirm_dialog, request_connection_write};
pub use connection_dialog::ConnectionDialog;
pub use connection_identity::{
    ConnectionIdentity, connection_identity_badge, connection_identity_for,
};
pub use connection_manager::ConnectionManager;
pub use content::ContentArea;
pub use dialog_helpers::{cancel_button, primary_button};
pub use filter_builder::FilterBuilderPanel;
pub use form_field::FormField;
pub use query_library::{QueryLibraryDialog, QueryLibraryTarget};
pub use status_bar::StatusBar;
pub use unsaved_guard::{
    request_app_quit, request_disconnect_connection, request_preview_collection,
    request_remove_connection, request_unsaved_action,
};
