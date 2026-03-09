//! AI assistant module — raw chat only.

pub mod blocks;
pub mod bridge;
pub mod budget;
pub mod context;
pub mod errors;
pub mod model_registry;
pub mod provider;
pub mod safety;
pub mod settings;
pub mod telemetry;
pub mod tools;

pub use blocks::{
    AiChatEntry, AiChatState, AiTurn, ChatMessage, ChatMessageTone, ChatRole, ContentBlock,
    ReportSheet, ToolActivity, ToolActivityStatus,
};
pub use errors::{AiError, AiErrorKind};
pub use settings::{AiProvider, AiSettings};
