//! What the user sees when something fails.
//!
//! An [`ErrorReport`] turns driver and app errors into a human title and message, and keeps the
//! server's own words, code, and extra detail for the Details disclosure and Copy.

use mongodb::error::{ErrorKind as MongoKind, WriteFailure};

use super::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The user's input can't be used as written.
    Validation,
    /// The server rejected the operation.
    Server,
    /// Someone else changed the data first.
    Conflict,
    /// The server couldn't be reached or the connection broke.
    Connection,
    Auth,
    Timeout,
    Io,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorReport {
    /// What failed, e.g. "Couldn't run stage 1".
    pub title: String,
    /// Why, in plain words, e.g. "A $group stage needs an _id field."
    pub message: String,
    /// The server's own message, kept verbatim for searching and support.
    pub server_message: Option<String>,
    pub code: Option<i32>,
    pub code_name: Option<String>,
    /// Longer technical detail: errInfo, hints, connection traces, per-item failures.
    pub details: Option<String>,
    /// What the user was doing, for Copy and Ask AI (e.g. the stage body).
    pub context: Option<String>,
    pub kind: ErrorKind,
}

impl ErrorReport {
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            server_message: None,
            code: None,
            code_name: None,
            details: None,
            context: None,
            kind: ErrorKind::Other,
        }
    }

    pub fn kind(mut self, kind: ErrorKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn details(mut self, details: impl Into<String>) -> Self {
        let details = details.into();
        self.details = (!details.trim().is_empty()).then_some(details);
        self
    }

    pub fn context(mut self, context: impl Into<String>) -> Self {
        let context = context.into();
        self.context = (!context.trim().is_empty()).then_some(context);
        self
    }

    pub fn from_error(title: impl Into<String>, error: &Error) -> Self {
        let title = title.into();
        match error {
            Error::Mongo(error) => Self::from_mongo(title, error),
            Error::PartialTransfer { processed, source } => {
                let mut report = Self::from_error(title, source);
                report.message = format!(
                    "{} Stopped after {processed} {}.",
                    report.message,
                    plural(*processed, "document")
                );
                report
            }
            Error::ContinuedOperation { processed, failure_count, details } => Self::new(
                title,
                format!(
                    "{failure_count} {} failed; {processed} {} succeeded.",
                    plural(*failure_count as u64, "part"),
                    plural(*processed, "document")
                ),
            )
            .details(
                details.iter().map(|detail| format!("- {detail}")).collect::<Vec<_>>().join("\n"),
            )
            .kind(ErrorKind::Server),
            Error::Conflict(message) => Self::new(title, message.clone()).kind(ErrorKind::Conflict),
            Error::Timeout(message) => Self::new(title, sentence(message)).kind(ErrorKind::Timeout),
            Error::Cancelled(message) => Self::new(title, sentence(message)),
            Error::Ssh(error) => {
                Self::new(title, sentence(&format!("SSH: {error}"))).kind(ErrorKind::Connection)
            }
            Error::Io(error) => Self::new(title, sentence(&error.to_string())).kind(ErrorKind::Io),
            Error::Json(_) | Error::Csv(_) => {
                Self::new(title, sentence(&error.to_string())).kind(ErrorKind::Validation)
            }
            Error::Parse(text) => {
                let mut report = Self::from_message(title, text);
                report.kind = ErrorKind::Validation;
                report
            }
            Error::ToolNotFound(_) => Self::new(title, sentence(&error.to_string())),
        }
    }

    pub fn from_mongo(title: impl Into<String>, error: &mongodb::error::Error) -> Self {
        let title = title.into();
        match error.kind.as_ref() {
            MongoKind::Command(command) => {
                let mut report = Self::server(
                    title,
                    command.code,
                    Some(command.code_name.clone()),
                    &command.message,
                );
                let err_info = error
                    .server_response()
                    .and_then(|raw| mongodb::bson::Document::try_from(raw.as_ref()).ok())
                    .and_then(|reply| reply.get_document("errInfo").ok().cloned());
                if let Some(err_info) = err_info {
                    report = report.details(crate::bson::document_to_shell_string(&err_info));
                }
                report
            }
            MongoKind::Write(WriteFailure::WriteError(write)) => {
                let report =
                    Self::server(title, write.code, write.code_name.clone(), &write.message);
                match &write.details {
                    Some(details) => report.details(crate::bson::document_to_shell_string(details)),
                    None => report,
                }
            }
            MongoKind::Write(WriteFailure::WriteConcernError(concern)) => {
                Self::server(title, concern.code, Some(concern.code_name.clone()), &concern.message)
            }
            MongoKind::InsertMany(insert) => {
                let errors = insert.write_errors.as_deref().unwrap_or_default();
                match errors.first() {
                    Some(first) => {
                        let mut report = Self::server(
                            title,
                            first.code,
                            first.code_name.clone(),
                            &first.message,
                        );
                        if errors.len() > 1 {
                            report.message = format!(
                                "{} {} more {} failed.",
                                report.message,
                                errors.len() - 1,
                                plural(errors.len() as u64 - 1, "document")
                            );
                            report = report.details(
                                errors
                                    .iter()
                                    .map(|error| {
                                        format!("- #{}: {}", error.index + 1, error.message)
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                            );
                        }
                        report
                    }
                    None => match &insert.write_concern_error {
                        Some(concern) => Self::server(
                            title,
                            concern.code,
                            Some(concern.code_name.clone()),
                            &concern.message,
                        ),
                        None => Self::new(title, "The insert failed.").kind(ErrorKind::Server),
                    },
                }
            }
            MongoKind::BulkWrite(bulk) => {
                let mut errors: Vec<_> = bulk.write_errors.iter().collect();
                errors.sort_by_key(|(index, _)| **index);
                match errors.first() {
                    Some((_, first)) => {
                        Self::server(title, first.code, first.code_name.clone(), &first.message)
                            .details(
                                errors
                                    .iter()
                                    .map(|(index, error)| {
                                        format!("- #{}: {}", *index + 1, error.message)
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                            )
                    }
                    None => Self::new(title, "The bulk write failed.").kind(ErrorKind::Server),
                }
            }
            MongoKind::Authentication { message, .. } => Self::new(
                title,
                "Authentication failed. Check the username, password, and authentication database.",
            )
            .kind(ErrorKind::Auth)
            .details(message.clone()),
            MongoKind::ServerSelection { message, .. } => {
                let timed_out = message.to_ascii_lowercase().contains("timeout");
                Self::new(
                    title,
                    if timed_out {
                        "The server didn't respond in time. Check the host, port, and network access."
                    } else {
                        "No server is available. Check the host, port, and network access."
                    },
                )
                .kind(ErrorKind::Connection)
                .details(message.clone())
            }
            MongoKind::DnsResolve { message, .. } => {
                Self::new(title, "The host name couldn't be resolved. Check the address.")
                    .kind(ErrorKind::Connection)
                    .details(message.clone())
            }
            MongoKind::Io(io) => {
                let message = match io.kind() {
                    std::io::ErrorKind::TimedOut => "The connection timed out.",
                    std::io::ErrorKind::ConnectionRefused => {
                        "The server refused the connection. Check that it's running and the port is right."
                    }
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe => {
                        "The connection was closed by the server."
                    }
                    _ => "A network error interrupted the operation.",
                };
                let kind = if io.kind() == std::io::ErrorKind::TimedOut {
                    ErrorKind::Timeout
                } else {
                    ErrorKind::Connection
                };
                Self::new(title, message).kind(kind).details(io.to_string())
            }
            MongoKind::InvalidTlsConfig { message, .. } => {
                Self::new(title, sentence(&format!("TLS settings are invalid: {message}")))
                    .kind(ErrorKind::Connection)
            }
            MongoKind::ConnectionPoolCleared { message, .. } => {
                Self::new(title, "The connection was reset. Try again.")
                    .kind(ErrorKind::Connection)
                    .details(message.clone())
            }
            MongoKind::InvalidArgument { message, .. } => {
                Self::new(title, sentence(message)).kind(ErrorKind::Validation)
            }
            MongoKind::ProxyConnect { message, .. } => {
                Self::new(title, "The proxy connection failed. Check the proxy settings.")
                    .kind(ErrorKind::Connection)
                    .details(message.clone())
            }
            other => Self::new(title, sentence(&other.to_string())).kind(ErrorKind::Server),
        }
    }

    fn server(title: String, code: i32, code_name: Option<String>, server_message: &str) -> Self {
        let code_name = code_name.filter(|name| !name.is_empty());
        let (message, kind) = describe_code(code, server_message);
        Self {
            title,
            message,
            server_message: Some(server_message.to_string()),
            code: Some(code),
            code_name,
            details: None,
            context: None,
            kind,
        }
    }

    /// Build a report from text that was already formatted, such as `"Save failed: reason"`.
    /// The first line becomes the message; anything after it becomes details.
    pub fn from_text(text: &str) -> Self {
        let text = text.trim();
        let (head, _) = text.split_once('\n').unwrap_or((text, ""));
        match head.split_once(": ") {
            Some((title, _)) if title.len() <= 48 && !title.contains(['{', '[', '"']) => {
                Self::from_message(title.to_string(), text[title.len() + 2..].trim())
            }
            _ => Self::from_message(String::new(), text),
        }
    }

    /// A report whose first line is the message and whose remaining lines are details.
    pub fn from_message(title: impl Into<String>, text: &str) -> Self {
        let title = title.into();
        let text = text.trim();
        let (message, details) = match text.split_once('\n') {
            Some((first, rest)) => (first.trim(), rest.trim()),
            None => (text, ""),
        };
        Self::new(title, sentence(message)).details(details)
    }

    /// "Location15955 · 15955", or whichever part exists.
    pub fn code_label(&self) -> Option<String> {
        match (&self.code_name, self.code) {
            (Some(name), Some(code)) if name != &code.to_string() => {
                Some(format!("{name} · {code}"))
            }
            (Some(name), _) => Some(name.clone()),
            (None, Some(code)) => Some(format!("Code {code}")),
            (None, None) => None,
        }
    }

    /// The whole report as plain text for the clipboard, mongosh-style for server errors.
    pub fn copy_text(&self) -> String {
        let mut out = match (self.title.is_empty(), self.message.is_empty()) {
            (false, false) => format!("{}: {}", self.title, self.message),
            (true, _) => self.message.clone(),
            (_, true) => self.title.clone(),
        };
        if let Some(server_message) = &self.server_message {
            let name = self.code_name.clone().or(self.code.map(|code| code.to_string()));
            match name {
                Some(name) => {
                    out.push_str(&format!("\nMongoServerError[{name}]: {server_message}"))
                }
                None => out.push_str(&format!("\n{server_message}")),
            }
        }
        for section in [&self.details, &self.context].into_iter().flatten() {
            out.push_str("\n\n");
            out.push_str(section);
        }
        out
    }

    /// The human line first, then the server's own words and details on their own lines, so text
    /// passed around as a string can be split back apart by [`Self::from_message`].
    pub fn display_text(&self) -> String {
        let mut out = self.one_line();
        if let Some(server_message) = &self.server_message
            && sentence(server_message) != self.message
        {
            out.push('\n');
            out.push_str(server_message);
        }
        if let Some(details) = &self.details {
            out.push('\n');
            out.push_str(details);
        }
        out
    }

    /// One line for places with no room, e.g. a toast body or a list row.
    pub fn one_line(&self) -> String {
        match &self.code_name {
            Some(name) => format!("{} ({name})", self.message),
            None => self.message.clone(),
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self.kind, ErrorKind::Connection | ErrorKind::Timeout | ErrorKind::Io)
    }
}

/// Plain words for the server errors people hit most, keeping the server's text otherwise.
fn describe_code(code: i32, server_message: &str) -> (String, ErrorKind) {
    let known = match code {
        11000 => Some(match duplicate_key_fields(server_message) {
            Some(fields) => format!("A document with the same {fields} already exists."),
            None => "A document with the same unique key already exists.".to_string(),
        }),
        121 => Some("The document doesn't match the collection's validation rules.".to_string()),
        13 => Some("This user isn't allowed to run that operation.".to_string()),
        18 => Some(
            "Authentication failed. Check the username, password, and authentication database."
                .to_string(),
        ),
        50 => Some("The operation ran past its time limit.".to_string()),
        26 => Some("The database or collection doesn't exist.".to_string()),
        48 => Some("A collection with this name already exists.".to_string()),
        85 => Some("An index with these keys already exists with different options.".to_string()),
        86 => Some("An index with this name already exists with different keys.".to_string()),
        27 => Some("The index doesn't exist.".to_string()),
        15955 => Some("A $group stage needs an _id field.".to_string()),
        40323 => Some("Each pipeline stage needs exactly one operator.".to_string()),
        40324 => Some(match quoted(server_message) {
            Some(name) => format!("{name} isn't a pipeline stage."),
            None => "That isn't a pipeline stage.".to_string(),
        }),
        _ => None,
    };
    let kind = match code {
        11000 | 112 => ErrorKind::Conflict,
        13 | 18 => ErrorKind::Auth,
        50 | 89 | 262 => ErrorKind::Timeout,
        6 | 7 | 91 | 189 | 10107 | 11600 | 11602 | 13435 | 13436 => ErrorKind::Connection,
        _ => ErrorKind::Server,
    };
    (known.unwrap_or_else(|| sentence(server_message)), kind)
}

/// `dup key: { email: "a@b.c" }` → `email`.
fn duplicate_key_fields(message: &str) -> Option<String> {
    let body = message.split("dup key: {").nth(1)?.split('}').next()?;
    let fields: Vec<&str> = body
        .split(',')
        .filter_map(|pair| pair.split(':').next())
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .collect();
    (!fields.is_empty()).then(|| fields.join(" and "))
}

fn quoted(message: &str) -> Option<&str> {
    let start = message.find('\'')? + 1;
    let end = start + message[start..].find('\'')?;
    Some(&message[start..end])
}

/// Capitalize and end with a period, without touching text that already reads as a sentence.
pub fn sentence(text: &str) -> String {
    let text = text.trim();
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out: String = first.to_uppercase().chain(chars).collect();
    if !out.ends_with(['.', '!', '?', ')', ']', '}', '"', '\'']) {
        out.push('.');
    }
    out
}

fn plural(count: u64, noun: &str) -> String {
    if count == 1 { noun.to_string() } else { format!("{noun}s") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_server_codes_read_as_plain_sentences() {
        let group = ErrorReport::server(
            "Couldn't run stage 1".into(),
            15955,
            Some("Location15955".into()),
            "a group specification must include an _id",
        );
        assert_eq!(group.message, "A $group stage needs an _id field.");
        assert_eq!(group.code_label().as_deref(), Some("Location15955 · 15955"));
        assert_eq!(
            group.copy_text(),
            "Couldn't run stage 1: A $group stage needs an _id field.\n\
             MongoServerError[Location15955]: a group specification must include an _id"
        );

        let duplicate = describe_code(
            11000,
            "E11000 duplicate key error collection: app.users index: email_1 dup key: { email: \"a@b.c\" }",
        );
        assert_eq!(
            duplicate,
            ("A document with the same email already exists.".into(), ErrorKind::Conflict)
        );

        let stage = describe_code(40324, "Unrecognized pipeline stage name: '$sm'");
        assert_eq!(stage.0, "$sm isn't a pipeline stage.");

        let unknown = describe_code(2, "unknown operator: $foo");
        assert_eq!(unknown, ("Unknown operator: $foo.".into(), ErrorKind::Server));
    }

    #[test]
    fn app_errors_drop_internal_prefixes() {
        let conflict =
            Error::Conflict("Document changed on the server; reload before saving.".into());
        let report = ErrorReport::from_error("Couldn't save", &conflict);
        assert_eq!(report.kind, ErrorKind::Conflict);
        assert_eq!(report.message, "Document changed on the server; reload before saving.");
        assert_eq!(conflict.to_string(), "Document changed on the server; reload before saving.");

        let io = mongodb::error::Error::from(std::io::Error::from(
            std::io::ErrorKind::ConnectionRefused,
        ));
        let report = ErrorReport::from_mongo("Couldn't connect", &io);
        assert_eq!(report.kind, ErrorKind::Connection);
        assert!(report.is_retryable());
        assert!(!report.message.contains("Kind:"));
    }

    #[test]
    fn formatted_text_splits_into_title_message_and_details() {
        let report =
            ErrorReport::from_text("Save failed: document is too large\n\nHint:\n- trim it");
        assert_eq!(report.title, "Save failed");
        assert_eq!(report.message, "Document is too large.");
        assert_eq!(report.details.as_deref(), Some("Hint:\n- trim it"));

        let plain = ErrorReport::from_text("Clipboard is empty");
        assert_eq!((plain.title.as_str(), plain.message.as_str()), ("", "Clipboard is empty."));
    }
}
