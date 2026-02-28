//! AI assistant module — raw chat only.

pub mod blocks;
pub mod bridge;
pub mod budget;
pub mod errors;
pub mod provider;
pub mod settings;
pub mod telemetry;

pub use blocks::{AiChatEntry, AiChatState, AiTurn, ChatMessage, ChatRole};
pub use errors::{AiError, AiErrorKind};
pub use settings::{AiProvider, AiSettings};
