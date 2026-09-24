//! Session error history.
//!
//! Every error lands here. Errors shown where they happened (a stage, a query panel, a dialog) are
//! only recorded; errors with no place of their own also raise a notification.

use std::collections::VecDeque;

use chrono::{DateTime, Local};
use uuid::Uuid;

use crate::error::ErrorReport;
use crate::state::app_state::types::SessionKey;

use super::AppState;

const ERROR_LOG_LIMIT: usize = 100;

/// A fix a notification can offer beside Copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorAction {
    Reconnect(Uuid),
    ReloadDocuments(SessionKey),
    /// Show the task in the Tasks tab.
    OpenTask(Uuid),
}

#[derive(Debug, Clone)]
pub struct ErrorEntry {
    pub id: u64,
    pub at: DateTime<Local>,
    pub report: ErrorReport,
    /// Raise a notification, because nothing else on screen shows this error.
    pub notify: bool,
    /// Keep the notification until dismissed, e.g. a file that failed to load at startup.
    pub sticky: bool,
    pub action: Option<ErrorAction>,
}

#[derive(Debug, Default)]
pub struct ErrorLog {
    entries: VecDeque<ErrorEntry>,
    last_id: u64,
    seen_id: u64,
    /// Why each connection's last attempt failed, until it connects again.
    connection_failures: std::collections::HashMap<Uuid, String>,
}

impl AppState {
    /// Record an error that is already visible where it happened.
    pub fn record_error(&mut self, report: ErrorReport) -> u64 {
        self.push_error(report, false, None)
    }

    /// Record an error and raise a notification for it.
    pub fn report_error(&mut self, report: ErrorReport) -> u64 {
        self.push_error(report, true, None)
    }

    pub fn report_error_with_action(&mut self, report: ErrorReport, action: ErrorAction) -> u64 {
        self.push_error(report, true, Some(action))
    }

    /// Report an error whose notification stays until dismissed.
    pub fn report_sticky_error(&mut self, report: ErrorReport) -> u64 {
        let id = self.push_error(report, true, None);
        if let Some(entry) = self.error_log.entries.back_mut() {
            entry.sticky = true;
        }
        id
    }

    fn push_error(
        &mut self,
        report: ErrorReport,
        notify: bool,
        action: Option<ErrorAction>,
    ) -> u64 {
        let log = &mut self.error_log;
        log.last_id += 1;
        log.entries.push_back(ErrorEntry {
            id: log.last_id,
            at: Local::now(),
            report,
            notify,
            sticky: false,
            action,
        });
        if log.entries.len() > ERROR_LOG_LIMIT {
            log.entries.pop_front();
        }
        log.last_id
    }

    /// Newest first.
    pub fn error_entries(&self) -> impl Iterator<Item = &ErrorEntry> {
        self.error_log.entries.iter().rev()
    }

    pub fn errors_after(&self, id: u64) -> impl Iterator<Item = &ErrorEntry> {
        self.error_log.entries.iter().filter(move |entry| entry.id > id)
    }

    pub fn last_error_id(&self) -> u64 {
        self.error_log.last_id
    }

    pub fn error_count(&self) -> usize {
        self.error_log.entries.len()
    }

    pub fn unseen_error_count(&self) -> usize {
        let seen = self.error_log.seen_id;
        self.error_log.entries.iter().filter(|entry| entry.id > seen).count()
    }

    pub fn mark_errors_seen(&mut self) {
        self.error_log.seen_id = self.error_log.last_id;
    }

    /// Transfers run in the background: notify unless their tab is the one on screen.
    pub fn report_transfer_error(&mut self, transfer_id: Uuid, report: ErrorReport) -> u64 {
        let on_screen = match self.active_tab() {
            super::ActiveTab::Index(index) => matches!(
                self.open_tabs().get(index),
                Some(super::TabKey::Transfer(key)) if key.id == transfer_id
            ),
            _ => false,
        };
        if on_screen { self.record_error(report) } else { self.report_error(report) }
    }

    pub fn report_compare_error(&mut self, id: Uuid, report: ErrorReport) -> u64 {
        if self.active_compare_tab_id() == Some(id) {
            self.record_error(report)
        } else {
            self.report_error(report)
        }
    }

    pub fn set_connection_failure(&mut self, connection_id: Uuid, message: Option<String>) {
        match message {
            Some(message) => self.error_log.connection_failures.insert(connection_id, message),
            None => self.error_log.connection_failures.remove(&connection_id),
        };
    }

    pub fn connection_failure(&self, connection_id: Uuid) -> Option<&str> {
        self.error_log.connection_failures.get(&connection_id).map(String::as_str)
    }

    pub fn clear_errors(&mut self) {
        self.error_log.entries.clear();
        self.error_log.seen_id = self.error_log.last_id;
    }
}

#[cfg(test)]
mod tests {
    use crate::error::ErrorReport;
    use crate::state::{AppState, StatusMessage};

    #[test]
    fn errors_are_kept_newest_first_and_status_errors_notify() {
        let mut state = AppState::new();
        let recorded = state.record_error(ErrorReport::new("Couldn't run stage 1", "Bad."));
        state.set_status_message(Some(StatusMessage::error("Drop failed: not allowed")));

        let entries: Vec<_> = state.error_entries().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].report.title, "Drop failed");
        assert!(entries[0].notify);
        assert!(!entries[1].notify);
        assert!(state.status_message().is_none());
        assert_eq!(state.errors_after(recorded).count(), 1);

        assert_eq!(state.unseen_error_count(), 2);
        state.mark_errors_seen();
        assert_eq!(state.unseen_error_count(), 0);
    }
}
