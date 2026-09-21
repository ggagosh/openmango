//! What a click on a value means, decided before any query runs.
//!
//! Split from the queries on purpose: which values are references, and which collections are
//! worth asking, is the part worth testing without a server.

use mongodb::bson::{Bson, Document};

use super::{FieldRef, RelationGraph, rank_target_collections};

/// Candidate collections a single search will probe.
///
/// A search is a query per collection. Past a few dozen the wait stops feeling like a click, so
/// the best-named ones go first and the rest wait behind an explicit "Search all".
pub const MAX_PROBED_COLLECTIONS: usize = 50;

/// How sure a candidate relation must be before a click follows it without asking. Below this
/// the value is still probed, so the floor only decides whether a guess is silent.
pub const NAVIGATION_CONFIDENCE: f32 = 0.8;

/// A value that can be followed.
#[derive(Debug, Clone, PartialEq)]
pub enum Reference {
    /// A bare ObjectId. Which collection it belongs to is not in the value.
    Id(Bson),
    /// A DBRef: the document names the collection itself, so nothing has to be guessed.
    DbRef { database: Option<String>, collection: String, id: Bson },
    /// Every id of an array, to be opened together. Never empty. They are taken to live in one
    /// collection, which is what an array of references is, so the first one is asked about and
    /// the rest follow it.
    Ids(Vec<Bson>),
}

impl Reference {
    /// The id that is looked up to find out where this points.
    pub fn id(&self) -> &Bson {
        match self {
            Reference::Id(id) => id,
            Reference::DbRef { id, .. } => id,
            Reference::Ids(ids) => &ids[0],
        }
    }

    /// The filter that shows what this points at, once the collection is known.
    pub fn filter(&self) -> mongodb::bson::Document {
        match self {
            Reference::Ids(ids) => mongodb::bson::doc! { "_id": { "$in": ids.clone() } },
            _ => mongodb::bson::doc! { "_id": self.id().clone() },
        }
    }
}

/// The ids of an array that holds nothing but ObjectIds, ready to be opened together.
pub fn references_in(value: &Bson) -> Option<Reference> {
    let Bson::Array(items) = value else {
        return None;
    };
    let all_ids = !items.is_empty() && items.iter().all(|item| matches!(item, Bson::ObjectId(_)));
    all_ids.then(|| Reference::Ids(items.clone()))
}

/// Whether a value at `path` can be followed.
///
/// Only ObjectIds, and only away from the root `_id`: a document's own id is where you already
/// are, and linkifying integers or strings would turn every quantity into a link. Strings
/// holding a 24-character hex id are a known follow-up, deliberately not guessed at here.
pub fn reference_at(path: &str, value: &Bson) -> Option<Reference> {
    if let Bson::Document(document) = value {
        return dbref(document);
    }
    if path == "_id" || !matches!(value, Bson::ObjectId(_)) {
        return None;
    }
    Some(Reference::Id(value.clone()))
}

/// A DBRef is `{ $ref: <collection>, $id: <value>, $db: <database>? }`. Extra keys are allowed
/// by the spec, so this checks for the two required ones rather than the document's shape.
fn dbref(document: &Document) -> Option<Reference> {
    let collection = document.get_str("$ref").ok()?;
    let id = document.get("$id")?;
    Some(Reference::DbRef {
        database: document.get_str("$db").ok().map(str::to_string),
        collection: collection.to_string(),
        id: id.clone(),
    })
}

/// What to do about a click, before asking the server anything.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// One collection to ask. Fetching the document both confirms the jump and fills the peek.
    Target { target: FieldRef, remembered: bool },
    /// Nothing known yet. Probe these, in this order.
    Search { candidates: Vec<String>, more: usize },
}

/// Decide where a reference points.
///
/// The order is trust-first: a DBRef names its own collection, then a stored relation, then a
/// name-ranked search. The search is what makes this work with no setup at all — the case the
/// Compass request describes, where the user does not know which collection the id lives in.
pub fn plan(
    graph: &RelationGraph,
    source: &FieldRef,
    reference: &Reference,
    collections: &[String],
) -> Plan {
    if let Reference::DbRef { database, collection, .. } = reference {
        let database = database.clone().unwrap_or_else(|| source.database.clone());
        return Plan::Target { target: FieldRef::id_of(database, collection), remembered: false };
    }

    let known =
        graph.outgoing(&source.database, &source.collection, &source.path, NAVIGATION_CONFIDENCE);
    if let Some(relation) = known.first() {
        return Plan::Target { target: relation.target.clone(), remembered: true };
    }

    let probeable: Vec<String> = collections
        .iter()
        .filter(|collection| !collection.starts_with("system."))
        .cloned()
        .collect();
    let ranked = rank_target_collections(&source.path, &probeable);
    let more = ranked.len().saturating_sub(MAX_PROBED_COLLECTIONS);
    Plan::Search {
        candidates: ranked.into_iter().take(MAX_PROBED_COLLECTIONS).map(str::to_string).collect(),
        more,
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{doc, oid::ObjectId};

    use super::super::{Origin, Relation};
    use super::*;

    fn source() -> FieldRef {
        FieldRef::new("shop", "orders", "userId")
    }

    fn collections() -> Vec<String> {
        ["orders", "users", "products", "system.views"].map(String::from).to_vec()
    }

    #[test]
    fn an_array_of_nothing_but_ids_opens_together() {
        let ids = vec![Bson::ObjectId(ObjectId::new()), Bson::ObjectId(ObjectId::new())];
        let reference = references_in(&Bson::Array(ids.clone())).expect("an array of ids");

        // Where it points is asked of the first; all of them are shown once that is known.
        assert_eq!(reference.id(), &ids[0]);
        assert_eq!(reference.filter(), mongodb::bson::doc! { "_id": { "$in": ids.clone() } });
        assert_eq!(
            Reference::Id(ids[0].clone()).filter(),
            mongodb::bson::doc! { "_id": ids[0].clone() }
        );

        // Mixed, empty and scalar values are not a set of references.
        assert_eq!(references_in(&Bson::Array(vec![ids[0].clone(), Bson::Int32(1)])), None);
        assert_eq!(references_in(&Bson::Array(Vec::new())), None);
        assert_eq!(references_in(&ids[0]), None);
    }

    #[test]
    fn only_object_ids_away_from_the_root_id_are_links() {
        let id = Bson::ObjectId(ObjectId::new());
        assert_eq!(reference_at("userId", &id), Some(Reference::Id(id.clone())));
        assert_eq!(reference_at("items[].productId", &id), Some(Reference::Id(id.clone())));

        // The document's own id is where you already are.
        assert_eq!(reference_at("_id", &id), None);
        // Linkifying these would turn every quantity and name into a link.
        assert_eq!(reference_at("quantity", &Bson::Int32(7)), None);
        assert_eq!(reference_at("sku", &Bson::String("64f0c0de".into())), None);
    }

    #[test]
    fn a_dbref_names_its_own_collection() {
        let id = Bson::ObjectId(ObjectId::new());
        let value = Bson::Document(doc! { "$ref": "users", "$id": id.clone() });
        assert_eq!(
            reference_at("owner", &value),
            Some(Reference::DbRef { database: None, collection: "users".into(), id: id.clone() })
        );

        // Extra keys are legal, and $db points somewhere else entirely.
        let cross = Bson::Document(
            doc! { "$ref": "users", "$id": id.clone(), "$db": "auth", "note": "legacy" },
        );
        let Some(Reference::DbRef { database, collection, .. }) = reference_at("owner", &cross)
        else {
            panic!("a DBRef with $db should still parse");
        };
        assert_eq!(database.as_deref(), Some("auth"));
        assert_eq!(collection, "users");

        // A plain sub-document is not a reference.
        assert_eq!(reference_at("shipping", &Bson::Document(doc! { "city": "Tbilisi" })), None);
    }

    #[test]
    fn a_dbref_needs_no_graph_and_no_probe() {
        let graph = RelationGraph::new();
        let id = Bson::ObjectId(ObjectId::new());
        let reference =
            Reference::DbRef { database: None, collection: "users".into(), id: id.clone() };

        let Plan::Target { target, remembered } =
            plan(&graph, &source(), &reference, &collections())
        else {
            panic!("a DBRef says where it points");
        };
        assert_eq!(target, FieldRef::id_of("shop", "users"));
        assert_eq!(target.database, "shop", "no $db means the document's own database");
        assert!(!remembered, "nothing was learned; the document already said so");
    }

    #[test]
    fn a_stored_relation_skips_the_search() {
        let mut graph = RelationGraph::new();
        graph.upsert(Relation::asserted(source(), FieldRef::id_of("shop", "users"), Origin::Probe));

        let reference = Reference::Id(Bson::ObjectId(ObjectId::new()));
        assert_eq!(
            plan(&graph, &source(), &reference, &collections()),
            Plan::Target { target: FieldRef::id_of("shop", "users"), remembered: true }
        );
    }

    #[test]
    fn a_rejected_relation_falls_back_to_searching() {
        let mut graph = RelationGraph::new();
        graph.upsert(Relation::asserted(source(), FieldRef::id_of("shop", "users"), Origin::Probe));
        graph.set_status(
            &source(),
            &FieldRef::id_of("shop", "users"),
            super::super::Status::Rejected,
        );

        let reference = Reference::Id(Bson::ObjectId(ObjectId::new()));
        let Plan::Search { candidates, .. } = plan(&graph, &source(), &reference, &collections())
        else {
            panic!("a rejected relation must not be followed");
        };
        assert!(candidates.contains(&"users".to_string()), "it stays probeable, just not assumed");
    }

    #[test]
    fn a_low_confidence_guess_is_probed_rather_than_followed() {
        let mut graph = RelationGraph::new();
        graph.upsert(Relation::candidate(
            source(),
            FieldRef::id_of("shop", "users"),
            0.4,
            super::super::Evidence {
                probed: 10,
                hits: 10,
                sampled: 1000,
                sampled_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            },
        ));

        let reference = Reference::Id(Bson::ObjectId(ObjectId::new()));
        assert!(matches!(plan(&graph, &source(), &reference, &collections()), Plan::Search { .. }));
    }

    #[test]
    fn a_search_ranks_by_name_and_skips_system_collections() {
        let graph = RelationGraph::new();
        let reference = Reference::Id(Bson::ObjectId(ObjectId::new()));

        let Plan::Search { candidates, more } = plan(&graph, &source(), &reference, &collections())
        else {
            panic!("an unknown field is searched");
        };
        assert_eq!(candidates[0], "users", "the field is named after its target");
        assert!(!candidates.iter().any(|name| name.starts_with("system.")));
        assert_eq!(more, 0);
    }

    #[test]
    fn a_huge_database_probes_the_best_named_and_reports_the_rest() {
        let graph = RelationGraph::new();
        let mut collections: Vec<String> = (0..200).map(|n| format!("col_{n:03}")).collect();
        collections.push("users".into());
        let reference = Reference::Id(Bson::ObjectId(ObjectId::new()));

        let Plan::Search { candidates, more } = plan(&graph, &source(), &reference, &collections)
        else {
            panic!("an unknown field is searched");
        };
        assert_eq!(candidates.len(), MAX_PROBED_COLLECTIONS);
        assert_eq!(candidates[0], "users", "the best name still goes first");
        assert_eq!(more, 151, "the rest are offered behind Search all, not dropped silently");
    }
}
