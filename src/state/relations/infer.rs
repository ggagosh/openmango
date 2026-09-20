//! Finding relations without being told: sample a collection, notice which fields hold
//! ObjectIds, and confirm each guess against the collection it seems to name.
//!
//! A foreign key is an *inclusion dependency* — every value of the field appears among the
//! values of the target's key — that is also plausible on names and types. Sampling can only
//! estimate the first half, so a candidate carries the size of the sample that produced it and
//! is never treated as proof.

use chrono::{DateTime, Utc};
use mongodb::bson::{Bson, Document, oid::ObjectId};

use super::resolve::{Reference, reference_at};
use super::{
    Evidence, FieldRef, Origin, Relation, Status, rank_target_collections, target_name_score,
};

/// How much of a field's sampled values must be ObjectIds before it is worth probing.
///
/// Below this the field is holding something else and happens to contain ids — a mixed bag
/// that would waste probes and produce a relation nobody wants.
pub const MIN_OBJECT_ID_RATIO: f32 = 0.9;

/// Distinct ids kept per field. The probe escalation tops out at this, so keeping more would
/// only cost memory.
pub const MAX_SAMPLED_IDS: usize = 200;

/// Collections probed per field, in name order.
///
/// ponytail: the true target is all but always among the best-named few, and anything missed
/// here is still found the moment someone clicks the field, which searches the whole database.
/// Widen this, or add the `_id` time-range prune the feature doc describes, if real databases
/// turn out to name their fields less helpfully than this assumes.
pub const CANDIDATES_PER_FIELD: usize = 8;

/// How many ids each round of probing sends. All of a round found escalates to the next; none
/// found rejects the candidate outright.
pub const PROBE_ROUNDS: [usize; 3] = [10, 50, 200];

/// What sampling found at one field path.
#[derive(Debug, Clone, PartialEq)]
pub struct PathProfile {
    /// The relation path, with every array level marked: `items[].productId`.
    pub path: String,
    /// Values seen at this path across the sample.
    pub seen: u64,
    /// How many of them were ObjectIds.
    pub object_ids: u64,
    /// Distinct ids kept, in the order they were met, capped at [`MAX_SAMPLED_IDS`].
    pub ids: Vec<ObjectId>,
}

impl PathProfile {
    pub fn object_id_ratio(&self) -> f32 {
        if self.seen == 0 { 0.0 } else { self.object_ids as f32 / self.seen as f32 }
    }

    /// Whether this field looks like a reference at all.
    pub fn is_reference_shaped(&self) -> bool {
        !self.ids.is_empty() && self.object_id_ratio() >= MIN_OBJECT_ID_RATIO
    }
}

/// Profile every field path in a sample that might hold references.
///
/// ponytail: a walker of its own rather than the schema view's. That one produces display
/// strings for a table; this one needs the raw ids to probe with, and sizes its sample
/// differently. Merge them if a third walker ever shows up.
pub fn profile_reference_paths(documents: &[Document]) -> Vec<PathProfile> {
    let mut found: Vec<PathProfile> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for document in documents {
        walk(document, "", &mut found, &mut index);
    }
    found.retain(PathProfile::is_reference_shaped);
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

fn walk(
    document: &Document,
    prefix: &str,
    found: &mut Vec<PathProfile>,
    index: &mut std::collections::HashMap<String, usize>,
) {
    for (key, value) in document {
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        // A document's own id is where a jump lands, never where one starts.
        if path == "_id" {
            continue;
        }
        record(&path, value, found, index);
    }
}

fn record(
    path: &str,
    value: &Bson,
    found: &mut Vec<PathProfile>,
    index: &mut std::collections::HashMap<String, usize>,
) {
    match value {
        // A DBRef says where it points, so it needs no inference — and its `$ref` and `$id` are
        // the reference's own parts, not fields of the document holding it.
        Bson::Document(nested) => {
            if !matches!(reference_at(path, value), Some(Reference::DbRef { .. })) {
                walk(nested, path, found, index);
            }
        }
        Bson::Array(items) => {
            // The array level belongs to the field that owns it, so every element folds into
            // one `items[]` path however deep the nesting goes.
            let element = format!("{path}[]");
            for item in items {
                record(&element, item, found, index);
            }
        }
        other => {
            let slot = *index.entry(path.to_string()).or_insert_with(|| {
                found.push(PathProfile {
                    path: path.to_string(),
                    seen: 0,
                    object_ids: 0,
                    ids: Vec::new(),
                });
                found.len() - 1
            });
            let profile = &mut found[slot];
            profile.seen += 1;
            if let Bson::ObjectId(id) = other {
                profile.object_ids += 1;
                if profile.ids.len() < MAX_SAMPLED_IDS && !profile.ids.contains(id) {
                    profile.ids.push(*id);
                }
            }
        }
    }
}

/// Relations the documents assert outright.
///
/// A DBRef names its collection, so it is read rather than guessed at, and stored with an
/// origin no later inference can overwrite.
pub fn declared_relations(
    database: &str,
    collection: &str,
    documents: &[Document],
) -> Vec<Relation> {
    let mut found: Vec<Relation> = Vec::new();
    for document in documents {
        collect_dbrefs(database, collection, document, "", &mut found);
    }
    // Sorted, so the same sample always produces the same list — in the file and on screen.
    found.sort_by(|a, b| (&a.source, &a.target).cmp(&(&b.source, &b.target)));
    found
}

fn collect_dbrefs(
    database: &str,
    collection: &str,
    document: &Document,
    prefix: &str,
    found: &mut Vec<Relation>,
) {
    for (key, value) in document {
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match value {
            Bson::Document(nested) => match reference_at(&path, value) {
                Some(Reference::DbRef { database: target_db, collection: target, .. }) => {
                    let relation = Relation::asserted(
                        FieldRef::new(database, collection, &path),
                        FieldRef::id_of(target_db.unwrap_or_else(|| database.to_string()), target),
                        Origin::DbRef,
                    );
                    if !found.contains(&relation) {
                        found.push(relation);
                    }
                }
                _ => collect_dbrefs(database, collection, nested, &path, found),
            },
            Bson::Array(items) => {
                let element = format!("{path}[]");
                for item in items {
                    if let Bson::Document(nested) = item {
                        match reference_at(&element, item) {
                            Some(Reference::DbRef {
                                database: target_db,
                                collection: target,
                                ..
                            }) => {
                                let relation = Relation::asserted(
                                    FieldRef::new(database, collection, &element),
                                    FieldRef::id_of(
                                        target_db.unwrap_or_else(|| database.to_string()),
                                        target,
                                    ),
                                    Origin::DbRef,
                                );
                                if !found.contains(&relation) {
                                    found.push(relation);
                                }
                            }
                            _ => collect_dbrefs(database, collection, nested, &element, found),
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// A field and the collection it might point at, with the ids that will settle it.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub source: FieldRef,
    pub target: FieldRef,
    pub ids: Vec<ObjectId>,
}

impl Candidate {
    /// The ids this round should send.
    pub fn round(&self, round: usize) -> &[ObjectId] {
        let size = PROBE_ROUNDS.get(round).copied().unwrap_or(0).min(self.ids.len());
        &self.ids[..size]
    }
}

/// Pair every reference-shaped field with the collections worth asking, best-named first.
///
/// A collection is never paired with itself on a field: a self-reference is real but rare, and
/// including it would probe every collection against its own `_id`, which always hits.
pub fn candidates(
    database: &str,
    collection: &str,
    profiles: &[PathProfile],
    collections: &[String],
) -> Vec<Candidate> {
    let probeable: Vec<String> = collections
        .iter()
        .filter(|name| name.as_str() != collection && !name.starts_with("system."))
        .cloned()
        .collect();

    profiles
        .iter()
        .flat_map(|profile| {
            let source = FieldRef::new(database, collection, &profile.path);
            rank_target_collections(&profile.path, &probeable)
                .into_iter()
                .take(CANDIDATES_PER_FIELD)
                .map(|target| Candidate {
                    source: source.clone(),
                    target: FieldRef::id_of(database, target),
                    ids: profile.ids.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Turn a round of probing into a relation, or nothing when the evidence rejects the guess.
///
/// Finding none of the probed ids rejects outright: a foreign key whose values are nowhere in
/// the target is not a foreign key. Finding all of them gives the rule-of-three bound; finding
/// some gives the observed rate, which is lower, so partial containment reads as the weaker
/// evidence it is.
pub fn score(
    candidate: &Candidate,
    probed: u32,
    hits: u32,
    sampled: u64,
    at: DateTime<Utc>,
) -> Option<Relation> {
    if probed == 0 || hits == 0 {
        return None;
    }
    let observed = hits as f32 / probed as f32;
    let bound = 1.0 - 3.0 / probed as f32;
    let confidence = observed.min(bound).clamp(0.0, 1.0);

    Some(Relation::candidate(
        candidate.source.clone(),
        candidate.target.clone(),
        confidence,
        Evidence { probed, hits, sampled, sampled_at: at },
    ))
}

/// Whether a round's result is worth sending a bigger one.
///
/// Only a clean sweep escalates. A partial hit has already told us what it is going to tell us:
/// more ids would raise the bound but the observed rate caps the confidence anyway.
pub fn should_escalate(probed: usize, hits: usize, available: usize) -> bool {
    hits == probed && probed < available && probed < MAX_SAMPLED_IDS
}

/// The relation to keep for a field, out of everything that survived probing.
///
/// A field points at one collection. When two both hold every probed id — `users` and a
/// `user_profiles` that mirrors its keys — the stronger evidence wins, and the loser is dropped
/// rather than stored as a rival that would make every later click ambiguous.
pub fn best_per_field(mut relations: Vec<Relation>) -> Vec<Relation> {
    relations.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then(b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
            // Two collections can both hold every probed id — an extension table that mirrors
            // its parent's keys. The evidence cannot separate them, so the name does.
            .then(
                target_name_score(&b.source.path, &b.target.collection)
                    .cmp(&target_name_score(&a.source.path, &a.target.collection)),
            )
    });
    relations.dedup_by(|a, b| a.source == b.source);
    relations
}

/// A relation search across a database, while it runs.
///
/// Collections are taken one at a time so the server sees one collection's worth of work at a
/// time and the whole thing can be stopped between them.
#[derive(Debug, Clone)]
pub struct InferenceRun {
    pub database: String,
    /// The collection being read right now.
    pub collection: String,
    pub done: usize,
    pub total: usize,
    /// Relations found so far, across every collection finished.
    pub found: usize,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl InferenceRun {
    pub fn new(database: String, total: usize) -> Self {
        Self {
            database,
            collection: String::new(),
            done: 0,
            total,
            found: 0,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// A handle the running job checks between collections.
    pub fn cancel_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.cancelled.clone()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Relations an inference pass produced, ready to store.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Inferred {
    pub relations: Vec<Relation>,
    /// Fields that looked like references but matched nothing. Worth reporting: it is the
    /// difference between "no references here" and "references we could not place".
    pub unresolved: Vec<String>,
}

impl Inferred {
    pub fn accepted_count(&self) -> usize {
        self.relations.iter().filter(|r| r.status == Status::Accepted).count()
    }

    /// Relations confident enough that a click would follow them without asking.
    pub fn confident_count(&self, floor: f32) -> usize {
        self.relations
            .iter()
            .filter(|r| r.origin == Origin::Inferred && r.confidence >= floor)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::doc;

    use super::*;

    fn ids(count: usize) -> Vec<ObjectId> {
        (0..count).map(|_| ObjectId::new()).collect()
    }

    #[test]
    fn sampling_finds_reference_shaped_fields_at_every_depth() {
        let user = ObjectId::new();
        let product = ObjectId::new();
        let asset = ObjectId::new();
        let documents = vec![doc! {
            "_id": ObjectId::new(),
            "userId": user,
            "quantity": 3,
            "items": [ { "productId": product, "qty": 1 } ],
            "sections": [ { "blocks": [ { "assetId": asset } ] } ],
            "shipping": { "countryId": ObjectId::new() },
        }];

        let profiles = profile_reference_paths(&documents);
        let paths: Vec<&str> = profiles.iter().map(|p| p.path.as_str()).collect();

        assert_eq!(
            paths,
            ["items[].productId", "sections[].blocks[].assetId", "shipping.countryId", "userId",]
        );
    }

    #[test]
    fn the_documents_own_id_is_never_a_source() {
        let documents = vec![doc! { "_id": ObjectId::new(), "userId": ObjectId::new() }];
        let profiles = profile_reference_paths(&documents);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].path, "userId");
    }

    #[test]
    fn a_field_that_only_sometimes_holds_an_id_is_not_a_reference() {
        // Nine strings to one ObjectId: something else that happens to contain an id.
        let mut documents: Vec<Document> =
            (0..9).map(|n| doc! { "ref": format!("sku-{n}") }).collect();
        documents.push(doc! { "ref": ObjectId::new() });

        assert!(profile_reference_paths(&documents).is_empty());

        // The other way round is a reference with one bad row.
        let mut mostly: Vec<Document> = (0..19).map(|_| doc! { "ref": ObjectId::new() }).collect();
        mostly.push(doc! { "ref": "sku-broken" });
        let profiles = profile_reference_paths(&mostly);
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].object_id_ratio() >= MIN_OBJECT_ID_RATIO);
    }

    #[test]
    fn ids_are_kept_distinct_and_capped() {
        let repeated = ObjectId::new();
        let mut documents: Vec<Document> = (0..5).map(|_| doc! { "userId": repeated }).collect();
        documents.extend((0..MAX_SAMPLED_IDS + 50).map(|_| doc! { "userId": ObjectId::new() }));

        let profiles = profile_reference_paths(&documents);
        assert_eq!(profiles[0].ids.len(), MAX_SAMPLED_IDS);
        assert_eq!(
            profiles[0].ids.iter().filter(|id| **id == repeated).count(),
            1,
            "probing the same id twice proves nothing twice"
        );
    }

    #[test]
    fn a_dbref_is_read_rather_than_guessed_at() {
        let user = ObjectId::new();
        let documents = vec![doc! {
            "_id": ObjectId::new(),
            "owner": { "$ref": "users", "$id": user },
            "audit": { "$ref": "events", "$id": ObjectId::new(), "$db": "logs" },
            "shipping": { "countryId": ObjectId::new() },
        }];

        // Its `$ref` and `$id` are parts of the reference, not fields to profile.
        let paths: Vec<String> =
            profile_reference_paths(&documents).into_iter().map(|p| p.path).collect();
        assert_eq!(paths, ["shipping.countryId"]);

        let declared = declared_relations("shop", "orders", &documents);
        let pairs: Vec<(&str, &str, &str)> = declared
            .iter()
            .map(|r| {
                (r.source.path.as_str(), r.target.database.as_str(), r.target.collection.as_str())
            })
            .collect();
        assert_eq!(pairs, [("audit", "logs", "events"), ("owner", "shop", "users")]);
        assert!(declared.iter().all(|r| r.origin == Origin::DbRef));
        assert!(declared.iter().all(|r| r.status == Status::Accepted));
    }

    #[test]
    fn the_same_dbref_in_every_document_is_one_relation() {
        let documents: Vec<Document> = (0..10)
            .map(|_| doc! { "owner": { "$ref": "users", "$id": ObjectId::new() } })
            .collect();
        assert_eq!(declared_relations("shop", "orders", &documents).len(), 1);
    }

    #[test]
    fn an_even_tie_goes_to_the_better_named_collection() {
        // An extension table that mirrors its parent's keys: both hold every probed id, so the
        // evidence cannot separate them.
        let source = FieldRef::new("shop", "orders", "userId");
        let evidence = Evidence { probed: 10, hits: 10, sampled: 1000, sampled_at: Utc::now() };
        let mirror = Relation::candidate(
            source.clone(),
            FieldRef::id_of("shop", "user_profiles"),
            0.7,
            evidence.clone(),
        );
        let real = Relation::candidate(source, FieldRef::id_of("shop", "users"), 0.7, evidence);

        for order in [vec![mirror.clone(), real.clone()], vec![real, mirror]] {
            let kept = best_per_field(order);
            assert_eq!(kept.len(), 1);
            assert_eq!(kept[0].target.collection, "users");
        }
    }

    #[test]
    fn candidates_are_the_best_named_collections_and_never_the_source() {
        let profiles =
            vec![PathProfile { path: "userId".into(), seen: 100, object_ids: 100, ids: ids(10) }];
        let collections: Vec<String> =
            ["orders", "users", "user_profiles", "system.views", "audit"]
                .map(String::from)
                .to_vec();

        let built = candidates("shop", "orders", &profiles, &collections);

        let targets: Vec<&str> = built.iter().map(|c| c.target.collection.as_str()).collect();
        assert_eq!(targets[0], "users");
        assert!(!targets.contains(&"orders"), "a collection is not probed against itself");
        assert!(!targets.iter().any(|name| name.starts_with("system.")));
        assert_eq!(built[0].source.path, "userId");
    }

    #[test]
    fn finding_nothing_rejects_the_guess() {
        let candidate = Candidate {
            source: FieldRef::new("shop", "orders", "userId"),
            target: FieldRef::id_of("shop", "audit"),
            ids: ids(10),
        };
        assert!(score(&candidate, 10, 0, 1000, Utc::now()).is_none());
        assert!(score(&candidate, 0, 0, 1000, Utc::now()).is_none());
    }

    #[test]
    fn confidence_rises_with_the_size_of_a_clean_sweep() {
        let candidate = Candidate {
            source: FieldRef::new("shop", "orders", "userId"),
            target: FieldRef::id_of("shop", "users"),
            ids: ids(200),
        };
        let at = Utc::now();

        // The rule of three: ten clean hits are not yet enough to follow silently.
        let ten = score(&candidate, 10, 10, 1000, at).unwrap();
        assert!((ten.confidence - 0.7).abs() < 0.001);

        let fifty = score(&candidate, 50, 50, 1000, at).unwrap();
        assert!((fifty.confidence - 0.94).abs() < 0.001);
        assert!(fifty.confidence > ten.confidence);

        let full = score(&candidate, 200, 200, 1000, at).unwrap();
        assert!(full.confidence > fifty.confidence);
        assert_eq!(full.status, Status::Candidate, "inference proposes, it does not decide");
        assert_eq!(full.origin, Origin::Inferred);
        assert_eq!(full.evidence.unwrap().sampled, 1000);
    }

    #[test]
    fn partial_containment_reads_as_the_weaker_evidence_it_is() {
        let candidate = Candidate {
            source: FieldRef::new("shop", "orders", "userId"),
            target: FieldRef::id_of("shop", "users"),
            ids: ids(50),
        };
        let partial = score(&candidate, 50, 45, 1000, Utc::now()).unwrap();
        let clean = score(&candidate, 50, 50, 1000, Utc::now()).unwrap();
        assert!(partial.confidence < clean.confidence);
        assert!((partial.confidence - 0.9).abs() < 0.001);
    }

    #[test]
    fn only_a_clean_sweep_is_worth_a_bigger_round() {
        assert!(should_escalate(10, 10, 200));
        assert!(!should_escalate(10, 9, 200), "a miss has already said what it will say");
        assert!(!should_escalate(50, 50, 50), "nothing left to send");
        assert!(!should_escalate(200, 200, 400), "the cap is the last round");
    }

    #[test]
    fn a_field_keeps_only_its_strongest_target() {
        let source = FieldRef::new("shop", "orders", "userId");
        let weaker = Relation::candidate(
            source.clone(),
            FieldRef::id_of("shop", "user_profiles"),
            0.7,
            Evidence { probed: 10, hits: 10, sampled: 1000, sampled_at: Utc::now() },
        );
        let stronger = Relation::candidate(
            source.clone(),
            FieldRef::id_of("shop", "users"),
            0.97,
            Evidence { probed: 200, hits: 200, sampled: 1000, sampled_at: Utc::now() },
        );
        let other = Relation::candidate(
            FieldRef::new("shop", "orders", "items[].productId"),
            FieldRef::id_of("shop", "products"),
            0.94,
            Evidence { probed: 50, hits: 50, sampled: 1000, sampled_at: Utc::now() },
        );

        let kept = best_per_field(vec![weaker, stronger, other]);

        assert_eq!(kept.len(), 2, "one target per field, plus the other field");
        let users = kept.iter().find(|r| r.source.path == "userId").unwrap();
        assert_eq!(users.target.collection, "users");
    }
}
