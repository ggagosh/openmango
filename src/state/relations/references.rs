//! What points at one document.
//!
//! The graph knows which fields reference a collection; this is the result of asking each of
//! them about one particular `_id`. It is a result, not a place: a tab holds one answer and is
//! re-run rather than navigated.

use mongodb::bson::{Bson, Document, doc};

use super::{FieldRef, mongo_path};

/// Documents shown per group before the rest are left to "Open as filter".
///
/// A glance answers "who points at this and roughly how many"; reading all of them is what the
/// filtered collection view is for.
pub const GROUP_PREVIEW_LIMIT: usize = 20;

/// One incoming relation, and what it turned up for this document.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceGroup {
    /// The field that points here, e.g. `orders.items[].productId`.
    pub source: FieldRef,
    /// Whether the source path has an index. An unindexed lookup is a collection scan.
    pub indexed: bool,
    /// Whether its documents are shown. Which fields point here, and how many documents each
    /// found, is the answer to the question; the documents themselves are the detail.
    pub expanded: bool,
    pub state: GroupState,
}

impl ReferenceGroup {
    /// The query that finds the referring documents. `find` matches inside arrays natively, so
    /// the dotted path needs no `$unwind` and no array markers.
    pub fn filter(&self, id: &Bson) -> Document {
        doc! { mongo_path(&self.source.path): id.clone() }
    }

    pub fn label(&self) -> String {
        format!("{}.{}", self.source.collection, self.source.path)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GroupState {
    /// Not run. An unindexed scan on a production connection is not something to start by
    /// itself, so it waits behind an explicit "Run anyway".
    Held,
    Loading,
    Loaded {
        documents: Vec<Document>,
        /// More matches exist than were fetched, so the count reads as "20+".
        more: bool,
    },
    Failed(String),
}

impl GroupState {
    /// What the group's header shows next to its name.
    pub fn count_label(&self) -> Option<String> {
        match self {
            GroupState::Loaded { documents, more } => Some(if *more {
                format!("{}+", documents.len())
            } else {
                documents.len().to_string()
            }),
            _ => None,
        }
    }
}

/// One tab's answer to "what points at this document?".
#[derive(Debug, Clone, PartialEq)]
pub struct ReferencesTabState {
    /// The document being asked about.
    pub target: FieldRef,
    pub id: Bson,
    /// The `_id` shortened for a tab title and a heading.
    pub label: String,
    /// Still working out which fields point here.
    pub discovering: bool,
    pub groups: Vec<ReferenceGroup>,
}

impl ReferencesTabState {
    pub fn new(target: FieldRef, id: Bson) -> Self {
        Self { label: short_id(&id), target, id, discovering: true, groups: Vec::new() }
    }

    /// Groups whose answer is in. Used to tell "nothing points here" from "still asking".
    pub fn settled(&self) -> bool {
        !self.discovering
            && self.groups.iter().all(|group| !matches!(group.state, GroupState::Loading))
    }

    /// Documents found across every group.
    pub fn total_found(&self) -> usize {
        self.groups
            .iter()
            .map(|group| match &group.state {
                GroupState::Loaded { documents, .. } => documents.len(),
                _ => 0,
            })
            .sum()
    }

    pub fn group_mut(&mut self, source: &FieldRef) -> Option<&mut ReferenceGroup> {
        self.groups.iter_mut().find(|group| &group.source == source)
    }
}

/// An id short enough for a tab, long enough to tell two apart.
///
/// ObjectIds end in a counter, so the tail distinguishes ids created in the same second far
/// better than the head, which is a timestamp they share.
pub fn short_id(id: &Bson) -> String {
    match id {
        Bson::ObjectId(oid) => {
            let hex = oid.to_hex();
            format!("…{}", &hex[hex.len() - 6..])
        }
        Bson::String(text) if text.len() > 12 => format!("…{}", &text[text.len() - 6..]),
        other => crate::bson::bson_value_preview(other, 12),
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::oid::ObjectId;

    use super::*;

    fn group(path: &str, indexed: bool, state: GroupState) -> ReferenceGroup {
        ReferenceGroup {
            source: FieldRef::new("shop", "orders", path),
            indexed,
            expanded: false,
            state,
        }
    }

    #[test]
    fn a_group_queries_the_dotted_path_whatever_its_arrays() {
        let id = Bson::ObjectId(ObjectId::new());

        // find() matches inside arrays on its own, so the markers come off and no $unwind is
        // needed to ask the question.
        let nested = group("items[].productId", true, GroupState::Held);
        assert_eq!(nested.filter(&id), doc! { "items.productId": id.clone() });
        assert_eq!(nested.label(), "orders.items[].productId");

        let flat = group("userId", true, GroupState::Held);
        assert_eq!(flat.filter(&id), doc! { "userId": id });
    }

    #[test]
    fn a_full_page_of_results_reads_as_more_than_it_shows() {
        let documents: Vec<Document> = (0..GROUP_PREVIEW_LIMIT).map(|_| doc! {}).collect();
        assert_eq!(
            GroupState::Loaded { documents: documents.clone(), more: true }.count_label(),
            Some("20+".to_string())
        );
        assert_eq!(
            GroupState::Loaded { documents, more: false }.count_label(),
            Some("20".to_string())
        );
        assert_eq!(GroupState::Loading.count_label(), None, "a count is not guessed at");
    }

    #[test]
    fn a_tab_is_settled_only_when_every_group_has_answered() {
        let target = FieldRef::id_of("shop", "users");
        let mut tab = ReferencesTabState::new(target, Bson::ObjectId(ObjectId::new()));
        assert!(!tab.settled(), "still working out which fields point here");

        tab.discovering = false;
        tab.groups = vec![
            group("userId", true, GroupState::Loaded { documents: vec![doc! {}], more: false }),
            group("ownerId", true, GroupState::Loading),
        ];
        assert!(!tab.settled());
        assert_eq!(tab.total_found(), 1);

        tab.groups[1].state = GroupState::Held;
        assert!(tab.settled(), "a held group has answered: it is waiting to be asked");
    }

    #[test]
    fn a_short_id_keeps_the_end_that_tells_two_apart() {
        let oid = ObjectId::new();
        let short = short_id(&Bson::ObjectId(oid));
        let tail = short.strip_prefix('…').expect("the ellipsis marks what was dropped");
        assert_eq!(tail.len(), 6);
        assert!(oid.to_hex().ends_with(tail));
    }
}
