//! Status messages for UI feedback.

#[derive(Clone)]
pub enum StatusLevel {
    Info,
    Error,
}

#[derive(Clone)]
pub struct StatusMessage {
    pub level: StatusLevel,
    pub text: String,
}

impl StatusMessage {
    pub fn info(text: impl Into<String>) -> Self {
        Self { level: StatusLevel::Info, text: text.into() }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self { level: StatusLevel::Error, text: text.into() }
    }
}
