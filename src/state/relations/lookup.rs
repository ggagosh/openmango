//! One reference lookup, from the click to the answer.
//!
//! A lookup is what the peek popover renders. There is only ever one: a peek is a glance at one
//! value, and a second click replaces the first.

use mongodb::bson::Document;

use crate::bson::DocumentKey;
use crate::state::app_state::SessionKey;

use super::FieldRef;
use super::resolve::Reference;

/// Which row the click came from, so the popover opens over that value rather than somewhere
/// the pointer is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub session: SessionKey,
    pub document: DocumentKey,
    pub path: String,
}

/// What the click asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Show what is there. Never moves, whatever the answer.
    Peek,
    /// Go there as soon as there is exactly one answer.
    Open,
    /// Go there in a tab of its own.
    OpenInNewTab,
}

impl Intent {
    pub fn is_peek(self) -> bool {
        matches!(self, Intent::Peek)
    }
}

/// A collection that turned out to hold the value, and the document it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub target: FieldRef,
    pub document: Document,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LookupState {
    /// Querying. The popover shows a skeleton, but only once the wait is long enough to notice.
    Probing,
    Found(Candidate),
    /// More than one collection holds this `_id`. Rare with ObjectIds, and never guessed at.
    Ambiguous(Vec<Candidate>),
    /// Nothing holds it. An orphan is information, not an error.
    Missing {
        /// Collections asked. Names the size of the claim: "not in any of 12 collections".
        searched: usize,
        /// Collections left unasked because the search was capped.
        more: usize,
    },
    Failed(String),
}

/// A click on a reference, and what came back.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceLookup {
    pub anchor: Anchor,
    pub source: FieldRef,
    pub reference: Reference,
    pub intent: Intent,
    pub state: LookupState,
    /// Whether the answer came from a search rather than a stored relation. A searched answer
    /// is worth remembering; one that was already remembered is not.
    pub searched: bool,
    /// Whether to store what the search found. Checked by default: the next click on this field
    /// should not have to search again.
    pub remember: bool,
}

impl ReferenceLookup {
    pub fn probing(anchor: Anchor, source: FieldRef, reference: Reference, intent: Intent) -> Self {
        Self {
            anchor,
            source,
            reference,
            intent,
            state: LookupState::Probing,
            searched: false,
            remember: true,
        }
    }

    /// True when this lookup belongs to the value at `path` of `document`.
    pub fn is_at(&self, session: &SessionKey, document: &DocumentKey, path: &str) -> bool {
        &self.anchor.session == session
            && &self.anchor.document == document
            && self.anchor.path == path
    }

    /// The single answer, when there is one.
    pub fn resolved(&self) -> Option<&Candidate> {
        match &self.state {
            LookupState::Found(candidate) => Some(candidate),
            _ => None,
        }
    }
}
