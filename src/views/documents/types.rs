//! Shared types for the documents view.

use gpui_kit::Entity;
use gpui_kit::component::input::InputState;

/// Inline editor state variants.
#[derive(Clone)]
pub enum InlineEditor {
    Text(Entity<InputState>),
    Number(Entity<InputState>),
    Bool(bool),
}
