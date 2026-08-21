// Reusable UI components

fn should_capture_uri_change(internal_value: &mut Option<String>, value: &str) -> bool {
    if internal_value.as_deref() == Some(value) {
        false
    } else {
        *internal_value = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::should_capture_uri_change;

    #[test]
    fn repeated_internal_uri_events_stay_ignored() {
        let mut internal = Some("mongodb://user@localhost".to_string());

        assert!(!should_capture_uri_change(&mut internal, "mongodb://user@localhost"));
        assert!(!should_capture_uri_change(&mut internal, "mongodb://user@localhost"));
        assert!(should_capture_uri_change(&mut internal, "mongodb://other@localhost"));
        assert!(internal.is_none());
    }
}

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
