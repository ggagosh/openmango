//! Shared types for the documents view.

use gpui::Entity;
use gpui_component::input::InputState;

/// Inline editor state variants.
#[derive(Clone)]
pub enum InlineEditor {
    Text(Entity<InputState>),
    Number(Entity<InputState>),
    Bool(bool),
}
