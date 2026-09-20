//! The relation graph: which field points at which collection.
//!
//! MongoDB declares no foreign keys, so this is the one place that records them. Navigation,
//! "find references", `$lookup` generation, the integrity report and the agent tools all read
//! the same graph, and every one of them is a plain query over [`RelationGraph`].
//!
//! Relations are keyed by **database name, not connection**: "`orders.userId` points at `users`"
//! is a fact about the application's schema, and the dev, staging and production copies of one
//! app share it. Learning it locally means it is already there against production, without
//! running inference there. A wrong entry from two unrelated apps sharing a database name heals
//! itself, because every jump is still confirmed by a probe.

pub mod export;
pub mod infer;
pub mod layout;
pub mod lookup;
pub mod references;
pub mod resolve;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The format written to `relations.json`. Bumped only for changes old builds cannot read.
pub const RELATION_MODEL_VERSION: u32 = 1;

/// One field of one collection. `path` marks every array level it crosses, so
/// `items[].productId` is distinguishable from `items.productId`; that count is how many
/// `$unwind` stages a generated join needs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FieldRef {
    pub database: String,
    pub collection: String,
    pub path: String,
}

impl FieldRef {
    pub fn new(
        database: impl Into<String>,
        collection: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self { database: database.into(), collection: collection.into(), path: path.into() }
    }

    /// The `_id` of a collection: what almost every reference points at.
    pub fn id_of(database: impl Into<String>, collection: impl Into<String>) -> Self {
        Self::new(database, collection, "_id")
    }

    pub fn namespace(&self) -> String {
        format!("{}.{}", self.database, self.collection)
    }

    pub fn is_in(&self, database: &str, collection: &str) -> bool {
        self.database == database && self.collection == collection
    }

    /// The path as MongoDB addresses it, with the array markers dropped:
    /// `sections[].blocks[].assetId` → `sections.blocks.assetId`.
    pub fn mongo_path(&self) -> String {
        mongo_path(&self.path)
    }

    /// How many `$unwind` stages a `$lookup` through this path needs.
    pub fn array_depth(&self) -> usize {
        array_depth(&self.path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// An ordinary pointer: the value is the target's key.
    Reference,
    /// The target collection depends on a discriminator field (Mongoose `refPath`).
    Polymorphic,
    /// Containment: the target lives inside the source document.
    Embedded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    OneToOne,
    ManyToOne,
    OneToMany,
    ManyToMany,
}

/// Where a relation came from, ordered by how much it should be trusted. Re-inference may
/// never overwrite something a human or the data itself asserted, so this ordering is the rule
/// [`RelationGraph::upsert`] enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Sampled and scored, never confirmed against the target.
    Inferred,
    /// A click-time probe found the value in the target collection.
    Probe,
    /// Read out of application code (Mongoose `ref`, Prisma, `$jsonSchema`).
    CodeImport,
    /// The document says so itself: a DBRef names its collection.
    DbRef,
    /// Someone accepted, rejected or edited it.
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Found by inference, not reviewed. Navigable only above a confidence floor.
    Candidate,
    Accepted,
    /// Reviewed and wrong. Never navigable, and kept so re-inference cannot resurrect it.
    Rejected,
}

/// What the confidence is based on. Sampling estimates, it does not prove, so the sample size
/// and its age travel with the number and are shown next to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// Values probed against the target's key index.
    pub probed: u32,
    /// How many of them were found.
    pub hits: u32,
    /// Documents sampled to find the candidate in the first place.
    pub sampled: u64,
    pub sampled_at: DateTime<Utc>,
}

impl Evidence {
    /// The lower bound on containment that `hits`-of-`probed` supports, at 95% confidence.
    ///
    /// All k found gives at least 1 − 3/k by the rule of three, so 20 hits means at least 85%
    /// and 100 hits at least 97%. A miss is fatal for a foreign key, so any miss scores 0.
    pub fn containment_lower_bound(&self) -> f32 {
        if self.probed == 0 || self.hits < self.probed {
            return 0.0;
        }
        (1.0 - 3.0 / self.probed as f32).max(0.0)
    }
}

/// One edge: a field that points at a collection's key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    pub source: FieldRef,
    pub target: FieldRef,
    pub kind: RelationKind,
    pub cardinality: Cardinality,
    pub origin: Origin,
    pub status: Status,
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
}

impl Relation {
    /// A relation the data itself asserts or a person confirmed: certain, and navigable at once.
    pub fn asserted(source: FieldRef, target: FieldRef, origin: Origin) -> Self {
        Self {
            cardinality: cardinality_for(&source),
            source,
            target,
            kind: RelationKind::Reference,
            origin,
            status: Status::Accepted,
            confidence: 1.0,
            evidence: None,
        }
    }

    /// A relation found by inference, waiting for review.
    pub fn candidate(
        source: FieldRef,
        target: FieldRef,
        confidence: f32,
        evidence: Evidence,
    ) -> Self {
        Self {
            cardinality: cardinality_for(&source),
            source,
            target,
            kind: RelationKind::Reference,
            origin: Origin::Inferred,
            status: Status::Candidate,
            confidence: confidence.clamp(0.0, 1.0),
            evidence: Some(evidence),
        }
    }

    /// What identifies this edge. A field may legitimately point at more than one collection
    /// (polymorphic, or rival candidates before review), so the target is part of the identity.
    pub fn identity(&self) -> (&FieldRef, &FieldRef, RelationKind) {
        (&self.source, &self.target, self.kind)
    }

    pub fn is_navigable(&self, min_confidence: f32) -> bool {
        match self.status {
            Status::Rejected => false,
            Status::Accepted => true,
            Status::Candidate => self.confidence >= min_confidence,
        }
    }
}

/// An array-valued source points at many targets; a scalar one points at a single target.
///
/// ponytail: the other half — one-to-one from a unique index on the source, and many-to-many
/// from a collection that holds exactly two references — needs the index list, which the
/// inference pass has and this constructor does not. It refines the value then.
fn cardinality_for(source: &FieldRef) -> Cardinality {
    if source.array_depth() > 0 { Cardinality::OneToMany } else { Cardinality::ManyToOne }
}

/// What [`RelationGraph::upsert`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upsert {
    Added,
    /// Replaced an entry of the same or lower trust.
    Updated,
    /// A more trusted entry is already there and stays. Re-inference cannot undo a decision.
    Refused,
}

/// One leg of a join. `forward` says which side of the relation the local field is on, which is
/// what decides `localField` and `foreignField` in the generated `$lookup`.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinStep {
    pub relation: Relation,
    pub forward: bool,
}

impl JoinStep {
    /// The collection this step arrives at.
    pub fn target(&self) -> &FieldRef {
        if self.forward { &self.relation.target } else { &self.relation.source }
    }

    /// The field on the collection the step starts from.
    pub fn local(&self) -> &FieldRef {
        if self.forward { &self.relation.source } else { &self.relation.target }
    }
}

/// A collection, as the join search addresses it.
type Namespace = (String, String);

/// How the breadth-first search reached each collection: the step that got there, and where it
/// came from. Walking it back from the goal gives the chain.
type Trail = HashMap<Namespace, (JoinStep, Namespace)>;

/// Every relation known for every database, and the queries the features ask of it.
///
/// ponytail: a plain `Vec` scanned linearly. The feature doc's four hash indexes are sized for
/// 5,000 edges, where a scan is still microseconds and happens once per click. Add an index
/// keyed by source path the day a profile says a scan shows up.
#[derive(Debug, Clone, Default)]
pub struct RelationGraph {
    relations: Vec<Relation>,
    /// When each database was last read in full. Its absence is what "nobody has looked yet"
    /// means, which an empty graph cannot say: a database read and found to have no relations
    /// is also empty.
    inferred: BTreeMap<String, DateTime<Utc>>,
}

impl RelationGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn relations(&self) -> &[Relation] {
        &self.relations
    }

    pub fn is_empty(&self) -> bool {
        self.relations.is_empty()
    }

    /// When `database` was last read from end to end, if it ever was.
    pub fn inferred_at(&self, database: &str) -> Option<DateTime<Utc>> {
        self.inferred.get(database).copied()
    }

    pub fn mark_inferred(&mut self, database: &str, at: DateTime<Utc>) {
        self.inferred.insert(database.to_string(), at);
    }

    /// Where a field points. Drives Cmd+click and the peek popover.
    pub fn outgoing(
        &self,
        database: &str,
        collection: &str,
        path: &str,
        min_confidence: f32,
    ) -> Vec<&Relation> {
        let mut found: Vec<&Relation> = self
            .relations
            .iter()
            .filter(|relation| {
                relation.source.is_in(database, collection)
                    && relation.source.path == path
                    && relation.is_navigable(min_confidence)
            })
            .collect();
        found.sort_by(|a, b| rank(b).partial_cmp(&rank(a)).unwrap_or(std::cmp::Ordering::Equal));
        found
    }

    /// What points at a collection. Drives the References view.
    pub fn referenced_by(
        &self,
        database: &str,
        collection: &str,
        min_confidence: f32,
    ) -> Vec<&Relation> {
        let mut found: Vec<&Relation> = self
            .relations
            .iter()
            .filter(|relation| {
                relation.target.is_in(database, collection) && relation.is_navigable(min_confidence)
            })
            .collect();
        found.sort_by(|a, b| {
            (&a.source.collection, &a.source.path).cmp(&(&b.source.collection, &b.source.path))
        });
        found
    }

    /// Every relation in a database, grouped by what it points at and ordered for reading:
    /// the most-referenced collection first, then by source.
    ///
    /// This is the review order — "what points at users" is the question people arrive with,
    /// and the collections nothing points at are the ones worth seeing last.
    pub fn by_target(&self, database: &str) -> Vec<(String, Vec<&Relation>)> {
        let mut groups: HashMap<&str, Vec<&Relation>> = HashMap::new();
        for relation in &self.relations {
            if relation.source.database == database {
                groups.entry(relation.target.collection.as_str()).or_default().push(relation);
            }
        }
        let mut grouped: Vec<(String, Vec<&Relation>)> = groups
            .into_iter()
            .map(|(target, mut relations)| {
                relations.sort_by(|a, b| {
                    (&a.source.collection, &a.source.path)
                        .cmp(&(&b.source.collection, &b.source.path))
                });
                (target.to_string(), relations)
            })
            .collect();
        grouped.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
        grouped
    }

    /// Every relation leaving a collection, by the field it leaves from.
    pub fn from_collection(&self, database: &str, collection: &str) -> Vec<&Relation> {
        let mut found: Vec<&Relation> = self
            .relations
            .iter()
            .filter(|relation| {
                relation.source.is_in(database, collection) && relation.status != Status::Rejected
            })
            .collect();
        found.sort_by(|a, b| a.source.path.cmp(&b.source.path));
        found
    }

    /// Every relation arriving at a collection, by where it comes from.
    pub fn into_collection(&self, database: &str, collection: &str) -> Vec<&Relation> {
        let mut found: Vec<&Relation> = self
            .relations
            .iter()
            .filter(|relation| {
                relation.target.is_in(database, collection) && relation.status != Status::Rejected
            })
            .collect();
        found.sort_by(|a, b| {
            (&a.source.collection, &a.source.path).cmp(&(&b.source.collection, &b.source.path))
        });
        found
    }

    /// Accepted candidates for a source path other than `keep`. This is drift: inference now
    /// believes something a reviewed decision contradicts.
    pub fn rivals_of(&self, keep: &Relation) -> Vec<&Relation> {
        self.relations
            .iter()
            .filter(|relation| {
                relation.source == keep.source
                    && relation.target != keep.target
                    && relation.status != Status::Rejected
            })
            .collect()
    }

    /// The shortest chain of relations joining two collections, or `None` when there is none.
    ///
    /// Relations are followed in both directions: joining `users` to `orders` runs the
    /// `orders.userId → users._id` edge backwards, which is an ordinary `$lookup` with the
    /// fields swapped.
    pub fn join_path(
        &self,
        from: (&str, &str),
        to: (&str, &str),
        min_confidence: f32,
    ) -> Option<Vec<JoinStep>> {
        let start: Namespace = (from.0.to_string(), from.1.to_string());
        let goal: Namespace = (to.0.to_string(), to.1.to_string());
        if start == goal {
            return Some(Vec::new());
        }

        // Breadth-first, so the first arrival is the shortest chain. `came_from` records the
        // step that reached each collection, and the chain is walked back from the goal.
        let mut seen: HashSet<Namespace> = HashSet::from([start.clone()]);
        let mut came_from: Trail = HashMap::new();
        let mut queue: VecDeque<Namespace> = VecDeque::from([start.clone()]);

        while let Some(here) = queue.pop_front() {
            for step in self.steps_from(&here, min_confidence) {
                let next = (step.target().database.clone(), step.target().collection.clone());
                if !seen.insert(next.clone()) {
                    continue;
                }
                came_from.insert(next.clone(), (step, here.clone()));
                if next == goal {
                    return Some(walk_back(&came_from, &start, goal));
                }
                queue.push_back(next);
            }
        }
        None
    }

    /// Every relation leaving a collection, in either direction.
    fn steps_from(&self, at: &Namespace, min_confidence: f32) -> Vec<JoinStep> {
        self.relations
            .iter()
            .filter(|relation| relation.is_navigable(min_confidence))
            .filter_map(|relation| {
                if relation.source.is_in(&at.0, &at.1) {
                    Some(JoinStep { relation: relation.clone(), forward: true })
                } else if relation.target.is_in(&at.0, &at.1) {
                    Some(JoinStep { relation: relation.clone(), forward: false })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Store a relation, keeping whichever of the two is more trusted.
    ///
    /// Idempotent on (source, target, kind): repeated inference over the same data refreshes
    /// the evidence instead of piling up duplicates.
    pub fn upsert(&mut self, relation: Relation) -> Upsert {
        match self.relations.iter_mut().find(|existing| existing.identity() == relation.identity())
        {
            Some(existing) if existing.origin > relation.origin => Upsert::Refused,
            Some(existing) => {
                *existing = relation;
                Upsert::Updated
            }
            None => {
                self.relations.push(relation);
                Upsert::Added
            }
        }
    }

    /// Record a review. The decision becomes the relation's origin, so re-inference leaves it
    /// alone from here on.
    pub fn set_status(&mut self, source: &FieldRef, target: &FieldRef, status: Status) -> bool {
        let Some(relation) = self
            .relations
            .iter_mut()
            .find(|relation| &relation.source == source && &relation.target == target)
        else {
            return false;
        };
        relation.status = status;
        // The decision outranks any later inference either way. Only accepting makes the
        // relation certain; a rejected one keeps the score it was rejected on, so the row still
        // says what the evidence had been.
        relation.origin = Origin::User;
        if status == Status::Accepted {
            relation.confidence = 1.0;
        }
        true
    }

    /// Forget every relation touching a database.
    ///
    /// Not wired to dropping a database: the model is shared across connections by database
    /// name, so dropping `shop` on dev would throw away what is still true of `shop` on
    /// production. Stale entries cost nothing — a jump probes, misses, and the user re-maps —
    /// so pruning is a deliberate act on the Relations page.
    pub fn remove_database(&mut self, database: &str) {
        self.relations.retain(|relation| {
            relation.source.database != database && relation.target.database != database
        });
        self.inferred.remove(database);
    }

    /// Follow a collection rename into both ends of every relation. Deliberate, for the same
    /// reason as [`RelationGraph::remove_database`].
    pub fn rename_collection(&mut self, database: &str, from: &str, to: &str) {
        for relation in &mut self.relations {
            for field in [&mut relation.source, &mut relation.target] {
                if field.database == database && field.collection == from {
                    field.collection = to.to_string();
                }
            }
        }
    }

    /// The on-disk form, sorted so the file only changes where the model did.
    pub fn to_model(&self) -> RelationModel {
        let mut relations = self.relations.clone();
        relations.sort_by(|a, b| (&a.source, &a.target).cmp(&(&b.source, &b.target)));
        RelationModel {
            version: RELATION_MODEL_VERSION,
            relations,
            inferred: self.inferred.clone(),
        }
    }

    pub fn from_model(model: RelationModel) -> Self {
        Self { relations: model.relations, inferred: model.inferred }
    }
}

/// Accepted beats candidate, then confidence, then trust. Decides which target a click takes
/// when a field has more than one.
fn rank(relation: &Relation) -> f32 {
    let accepted = if relation.status == Status::Accepted { 10.0 } else { 0.0 };
    accepted + relation.confidence + (relation.origin as u8 as f32) / 100.0
}

fn walk_back(came_from: &Trail, start: &Namespace, goal: Namespace) -> Vec<JoinStep> {
    let mut chain = Vec::new();
    let mut at = goal;
    while &at != start {
        let Some((step, previous)) = came_from.get(&at) else {
            break;
        };
        chain.push(step.clone());
        at = previous.clone();
    }
    chain.reverse();
    chain
}

/// `relations.json`. One flat list so a relation crossing databases is expressible, sorted for
/// readable diffs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationModel {
    pub version: u32,
    #[serde(default)]
    pub relations: Vec<Relation>,
    /// Database name to when it was last read in full. Added after version 1 shipped; a file
    /// without it loads as "never", which is the truth about it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inferred: BTreeMap<String, DateTime<Utc>>,
}

impl Default for RelationModel {
    fn default() -> Self {
        Self { version: RELATION_MODEL_VERSION, relations: Vec::new(), inferred: BTreeMap::new() }
    }
}

// =============================================================================================
// Field paths
// =============================================================================================

/// Turn a schema-profiler path into a relation path: `items.[*].productId` → `items[].productId`.
///
/// The profiler writes an array element as its own `.[*]` segment; relations mark the array on
/// the field that owns it, which keeps the field name and its array-ness in one token.
pub fn from_profile_path(path: &str) -> String {
    path.replace(".[*]", "[]")
}

/// The path as MongoDB addresses it: `sections[].blocks[].assetId` → `sections.blocks.assetId`.
/// `find` uses this directly, and `$lookup` needs one `$unwind` per marker removed.
pub fn mongo_path(path: &str) -> String {
    path.replace("[]", "")
}

/// How many array levels a path crosses.
pub fn array_depth(path: &str) -> usize {
    path.matches("[]").count()
}

/// The relation path for a concrete path through a document: `items.0.productId` becomes
/// `items[].productId`.
///
/// The index is dropped because a relation belongs to the field, not to one element of it —
/// every `items[].productId` in the collection points at the same collection.
pub fn path_from_segments(segments: &[crate::bson::PathSegment]) -> String {
    let mut path = String::new();
    for segment in segments {
        match segment {
            crate::bson::PathSegment::Key(key) => {
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(key);
            }
            crate::bson::PathSegment::Index(_) => path.push_str("[]"),
        }
    }
    path
}

/// A filter as the user will see it in the filter bar: `{ _id: ObjectId("…") }`, not Extended
/// JSON. The same rendering the workspace uses, so a navigated filter and a typed one match.
pub fn filter_text(filter: &mongodb::bson::Document) -> String {
    crate::bson::format_relaxed_json_compact(
        &mongodb::bson::Bson::Document(filter.clone()).into_relaxed_extjson(),
    )
}

/// Whether a path is a document's own `_id`.
///
/// Exactly `_id`, not a nested one: `users.addresses[]._id` belongs to an embedded document,
/// which is a different thing to ask about.
pub fn is_document_id(segments: &[crate::bson::PathSegment]) -> bool {
    matches!(segments, [crate::bson::PathSegment::Key(key)] if key == "_id")
}

/// The field name a path ends in, without its array marker: `items[].productId` → `productId`.
pub fn leaf_name(path: &str) -> &str {
    let leaf = path.rsplit('.').next().unwrap_or(path);
    leaf.strip_suffix("[]").unwrap_or(leaf)
}

// =============================================================================================
// Name heuristic
// =============================================================================================

/// Rank collections by how much their name looks like the target of `path`, best first.
///
/// This only orders the probe queue. A name never decides a jump on its own — `userId` pointing
/// at `users` still has to be confirmed by finding the value there — so a wrong guess here costs
/// one extra probe, not a wrong navigation.
pub fn rank_target_collections<'a>(path: &str, collections: &'a [String]) -> Vec<&'a str> {
    let stem = reference_stem(path);
    let mut ranked: Vec<(u32, &'a str)> = collections
        .iter()
        .map(|collection| (name_score(&stem, collection), collection.as_str()))
        .collect();
    // A stable sort, so ties keep the caller's order — the server's collection order.
    ranked.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    ranked.into_iter().map(|(_, collection)| collection).collect()
}

/// What a reference field is named after: `userId` → `user`, `owner_ids` → `owner`,
/// `items[].productId` → `product`.
pub fn reference_stem(path: &str) -> String {
    let leaf = leaf_name(path);
    let lowered = leaf.to_lowercase();
    let trimmed = lowered
        .strip_suffix("_ids")
        .or_else(|| lowered.strip_suffix("_id"))
        .or_else(|| lowered.strip_suffix("ids"))
        .or_else(|| lowered.strip_suffix("id"))
        .unwrap_or(&lowered);
    let trimmed = trimmed.trim_end_matches('_');
    // `_id` and `id` trim to nothing; those name no collection, so keep the original.
    if trimmed.is_empty() { normalize(&lowered) } else { normalize(trimmed) }
}

/// How much a collection's name looks like the target of `path`. Higher is closer.
///
/// Only ever breaks ties between collections the data has already confirmed; it never decides
/// a relation on its own.
pub fn target_name_score(path: &str, collection: &str) -> u32 {
    name_score(&reference_stem(path), collection)
}

fn name_score(stem: &str, collection: &str) -> u32 {
    let candidate = normalize(&collection.to_lowercase());
    if candidate == stem {
        return 100;
    }
    if singular(&candidate) == singular(stem) {
        return 90;
    }
    if candidate.ends_with(stem) || stem.ends_with(&candidate) {
        return 70;
    }
    // `usermodels` for `user`: named after the thing, with a suffix a codebase happens to use
    // everywhere. Worth more than merely containing the stem somewhere in the middle.
    if candidate.starts_with(stem) {
        return 60;
    }
    if candidate.contains(stem) || stem.contains(&candidate) {
        return 25;
    }
    0
}

/// Fold away the separators, so `user_profiles`, `userProfiles` and `userprofiles` compare equal.
fn normalize(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

/// ponytail: enough English plurals to match a collection name against a field name. Nothing
/// downstream trusts it — it only orders probes — so irregular plurals cost an extra probe.
fn singular(word: &str) -> String {
    for (suffix, replacement) in
        [("ies", "y"), ("ches", "ch"), ("shes", "sh"), ("sses", "ss"), ("xes", "x"), ("zes", "z")]
    {
        if let Some(stem) = word.strip_suffix(suffix) {
            return format!("{stem}{replacement}");
        }
    }
    match word.strip_suffix('s') {
        Some(stem) if !word.ends_with("ss") => stem.to_string(),
        _ => word.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(collection: &str, path: &str) -> FieldRef {
        FieldRef::new("shop", collection, path)
    }

    fn id(collection: &str) -> FieldRef {
        FieldRef::id_of("shop", collection)
    }

    fn evidence(probed: u32, hits: u32) -> Evidence {
        Evidence {
            probed,
            hits,
            sampled: 1000,
            sampled_at: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        }
    }

    /// `orders.userId → users`, `orders.items[].productId → products`.
    fn shop() -> RelationGraph {
        let mut graph = RelationGraph::new();
        graph.upsert(Relation::asserted(field("orders", "userId"), id("users"), Origin::Probe));
        graph.upsert(Relation::asserted(
            field("orders", "items[].productId"),
            id("products"),
            Origin::Probe,
        ));
        graph
    }

    #[test]
    fn when_a_database_was_read_survives_a_save_and_old_files_say_never() {
        let mut graph = RelationGraph::new();
        let at = chrono::Utc::now();
        graph.mark_inferred("shop", at);

        let text = serde_json::to_string(&graph.to_model()).unwrap();
        let loaded = RelationGraph::from_model(serde_json::from_str(&text).unwrap());
        assert_eq!(loaded.inferred_at("shop"), Some(at));
        assert_eq!(loaded.inferred_at("elsewhere"), None);

        // A file written before the field existed has nothing to say, and says nothing on save.
        let old: RelationModel = serde_json::from_str(r#"{"version":1,"relations":[]}"#).unwrap();
        assert_eq!(RelationGraph::from_model(old).inferred_at("shop"), None);
        assert!(
            !serde_json::to_string(&RelationGraph::new().to_model()).unwrap().contains("inferred")
        );

        // A dropped database is forgotten whole, the date with it.
        graph.remove_database("shop");
        assert_eq!(graph.inferred_at("shop"), None);
    }

    #[test]
    fn paths_mark_every_array_level() {
        assert_eq!(from_profile_path("items.[*].productId"), "items[].productId");
        assert_eq!(from_profile_path("tagIds.[*]"), "tagIds[]");
        assert_eq!(
            from_profile_path("sections.[*].blocks.[*].assetId"),
            "sections[].blocks[].assetId"
        );
        assert_eq!(from_profile_path("shipping.address.countryId"), "shipping.address.countryId");

        // find() addresses the dotted path; $lookup needs one $unwind per marker.
        assert_eq!(mongo_path("sections[].blocks[].assetId"), "sections.blocks.assetId");
        assert_eq!(array_depth("sections[].blocks[].assetId"), 2);
        assert_eq!(array_depth("shipping.address.countryId"), 0);

        assert_eq!(leaf_name("items[].productId"), "productId");
        assert_eq!(leaf_name("tagIds[]"), "tagIds");
        assert_eq!(leaf_name("userId"), "userId");
    }

    #[test]
    fn a_documents_concrete_path_becomes_the_field_it_belongs_to() {
        use crate::bson::PathSegment::{Index, Key};

        // The element index is dropped: every items[].productId points at the same collection,
        // so the relation belongs to the field rather than to one element of it.
        assert_eq!(
            path_from_segments(&[Key("items".into()), Index(3), Key("productId".into())]),
            "items[].productId"
        );
        assert_eq!(path_from_segments(&[Key("tagIds".into()), Index(0)]), "tagIds[]");
        assert_eq!(
            path_from_segments(&[Key("shipping".into()), Key("countryId".into())]),
            "shipping.countryId"
        );
        assert_eq!(path_from_segments(&[Key("_id".into())]), "_id");
        assert_eq!(path_from_segments(&[]), "");

        // The round trip a click makes: document path to relation path to Mongo path.
        let path = path_from_segments(&[
            Key("sections".into()),
            Index(1),
            Key("blocks".into()),
            Index(0),
            Key("assetId".into()),
        ]);
        assert_eq!(path, "sections[].blocks[].assetId");
        assert_eq!(array_depth(&path), 2);
        assert_eq!(mongo_path(&path), "sections.blocks.assetId");
    }

    #[test]
    fn only_a_documents_own_id_is_where_incoming_references_are_asked_about() {
        use crate::bson::PathSegment::{Index, Key};

        assert!(is_document_id(&[Key("_id".into())]));

        // An embedded document's id is a different thing to ask about.
        assert!(!is_document_id(&[Key("addresses".into()), Index(0), Key("_id".into())]));
        assert!(!is_document_id(&[Key("userId".into())]));
        assert!(!is_document_id(&[]));
    }

    #[test]
    fn cardinality_follows_the_shape_of_the_source() {
        let scalar = Relation::asserted(field("orders", "userId"), id("users"), Origin::Probe);
        assert_eq!(scalar.cardinality, Cardinality::ManyToOne);

        let array = Relation::asserted(field("posts", "tagIds[]"), id("tags"), Origin::Probe);
        assert_eq!(array.cardinality, Cardinality::OneToMany);
    }

    #[test]
    fn a_join_runs_relations_in_both_directions() {
        let graph = shop();

        // Forward: orders holds the pointer.
        let forward = graph.join_path(("shop", "orders"), ("shop", "users"), 0.5).unwrap();
        assert_eq!(forward.len(), 1);
        assert!(forward[0].forward);
        assert_eq!(forward[0].local().path, "userId");

        // Backward: users is pointed at, so the same edge joins the other way.
        let backward = graph.join_path(("shop", "users"), ("shop", "orders"), 0.5).unwrap();
        assert_eq!(backward.len(), 1);
        assert!(!backward[0].forward);
        assert_eq!(backward[0].local().path, "_id");
        assert_eq!(backward[0].target().collection, "orders");
    }

    #[test]
    fn a_join_chains_through_an_intermediate_collection() {
        let graph = shop();
        let chain = graph.join_path(("shop", "users"), ("shop", "products"), 0.5).unwrap();

        let hops: Vec<&str> = chain.iter().map(|step| step.target().collection.as_str()).collect();
        assert_eq!(hops, ["orders", "products"]);
        // The array marker survives the walk, so the generator knows it needs an $unwind.
        assert_eq!(chain[1].local().array_depth(), 1);
        assert_eq!(chain[1].local().mongo_path(), "items.productId");
    }

    #[test]
    fn a_join_with_no_route_is_a_miss_not_an_empty_path() {
        let graph = shop();
        assert!(graph.join_path(("shop", "orders"), ("shop", "invoices"), 0.5).is_none());
        // A collection joins to itself with no stages at all.
        assert_eq!(graph.join_path(("shop", "orders"), ("shop", "orders"), 0.5), Some(Vec::new()));
    }

    #[test]
    fn review_order_leads_with_the_most_referenced_collection() {
        let mut graph = shop();
        graph.upsert(Relation::asserted(field("invoices", "userId"), id("users"), Origin::Probe));

        let grouped = graph.by_target("shop");
        let shape: Vec<(String, usize)> =
            grouped.iter().map(|(target, rows)| (target.clone(), rows.len())).collect();
        assert_eq!(shape, [("users".to_string(), 2), ("products".to_string(), 1)]);
        // Within a group, by where the reference comes from.
        assert_eq!(grouped[0].1[0].source.collection, "invoices");
        assert_eq!(grouped[0].1[1].source.collection, "orders");

        assert!(graph.by_target("other").is_empty(), "scoped to one database");
    }

    #[test]
    fn a_rejected_relation_is_never_walked() {
        let mut graph = shop();
        graph.set_status(&field("orders", "userId"), &id("users"), Status::Rejected);

        assert!(graph.outgoing("shop", "orders", "userId", 0.0).is_empty());
        assert!(graph.referenced_by("shop", "users", 0.0).is_empty());
        assert!(graph.join_path(("shop", "orders"), ("shop", "users"), 0.0).is_none());
    }

    #[test]
    fn a_candidate_navigates_only_above_the_confidence_floor() {
        let mut graph = RelationGraph::new();
        graph.upsert(Relation::candidate(
            field("orders", "userId"),
            id("users"),
            0.6,
            evidence(20, 20),
        ));

        assert!(graph.outgoing("shop", "orders", "userId", 0.5).len() == 1);
        assert!(graph.outgoing("shop", "orders", "userId", 0.8).is_empty());
    }

    #[test]
    fn a_decision_outranks_later_inference() {
        let mut graph = RelationGraph::new();
        let source = field("orders", "userId");

        assert_eq!(
            graph.upsert(Relation::asserted(source.clone(), id("users"), Origin::User)),
            Upsert::Added
        );
        // Re-inference finds the same edge and must not downgrade the decision.
        assert_eq!(
            graph.upsert(Relation::candidate(source.clone(), id("users"), 0.7, evidence(20, 20))),
            Upsert::Refused
        );
        let kept = &graph.outgoing("shop", "orders", "userId", 0.0)[0];
        assert_eq!(kept.origin, Origin::User);
        assert_eq!(kept.confidence, 1.0);

        // A DBRef, being weaker than a decision, is refused too; a second decision is not.
        assert_eq!(
            graph.upsert(Relation::asserted(source.clone(), id("users"), Origin::DbRef)),
            Upsert::Refused
        );
        assert_eq!(
            graph.upsert(Relation::asserted(source, id("users"), Origin::User)),
            Upsert::Updated
        );
    }

    #[test]
    fn upsert_refreshes_rather_than_duplicating() {
        let mut graph = RelationGraph::new();
        let source = field("orders", "userId");
        graph.upsert(Relation::candidate(source.clone(), id("users"), 0.6, evidence(10, 10)));
        graph.upsert(Relation::candidate(source.clone(), id("users"), 0.9, evidence(200, 200)));

        assert_eq!(graph.relations().len(), 1, "the same edge twice is one edge");
        assert_eq!(graph.relations()[0].evidence.as_ref().unwrap().probed, 200);

        // A different target from the same field is a rival, not a replacement.
        graph.upsert(Relation::candidate(source, id("accounts"), 0.5, evidence(10, 10)));
        assert_eq!(graph.relations().len(), 2);
    }

    #[test]
    fn a_rival_target_for_a_decided_field_is_visible_as_drift() {
        let mut graph = RelationGraph::new();
        let source = field("orders", "userId");
        let decided = Relation::asserted(source.clone(), id("users"), Origin::User);
        graph.upsert(decided.clone());
        graph.upsert(Relation::candidate(source, id("accounts"), 0.9, evidence(50, 50)));

        let rivals = graph.rivals_of(&decided);
        assert_eq!(rivals.len(), 1);
        assert_eq!(rivals[0].target.collection, "accounts");
    }

    #[test]
    fn a_click_prefers_the_accepted_target_over_a_confident_guess() {
        let mut graph = RelationGraph::new();
        let source = field("orders", "userId");
        graph.upsert(Relation::candidate(source.clone(), id("accounts"), 0.99, evidence(50, 50)));
        graph.upsert(Relation::asserted(source.clone(), id("users"), Origin::User));

        let targets = graph.outgoing("shop", "orders", "userId", 0.0);
        assert_eq!(targets[0].target.collection, "users");
        assert_eq!(targets.len(), 2, "the guess is still offered, just second");
    }

    #[test]
    fn the_model_round_trips_and_sorts_stably() {
        let mut graph = RelationGraph::new();
        // Inserted out of order, and one edge carries evidence.
        graph.upsert(Relation::asserted(
            field("orders", "items[].productId"),
            id("products"),
            Origin::DbRef,
        ));
        graph.upsert(Relation::candidate(
            field("orders", "userId"),
            id("users"),
            0.85,
            evidence(50, 50),
        ));
        graph.upsert(Relation::asserted(field("invoices", "orderId"), id("orders"), Origin::User));

        let json = serde_json::to_string_pretty(&graph.to_model()).unwrap();
        let reloaded: RelationModel = serde_json::from_str(&json).unwrap();
        assert_eq!(reloaded.version, RELATION_MODEL_VERSION);

        let restored = RelationGraph::from_model(reloaded);
        assert_eq!(restored.relations(), graph.to_model().relations.as_slice());

        // Sorted by source then target, so the file only changes where the model did.
        let order: Vec<String> = restored
            .relations()
            .iter()
            .map(|relation| format!("{}:{}", relation.source.collection, relation.source.path))
            .collect();
        assert_eq!(order, ["invoices:orderId", "orders:items[].productId", "orders:userId"]);
        assert_eq!(serde_json::to_string_pretty(&restored.to_model()).unwrap(), json);
    }

    #[test]
    fn a_decision_survives_a_save_and_reload() {
        let mut graph = shop();
        graph.set_status(&field("orders", "userId"), &id("users"), Status::Rejected);

        let json = serde_json::to_string(&graph.to_model()).unwrap();
        let restored = RelationGraph::from_model(serde_json::from_str(&json).unwrap());

        let rejected = restored
            .relations()
            .iter()
            .find(|relation| relation.source.path == "userId")
            .expect("the rejection is kept, so re-inference cannot resurrect it");
        assert_eq!(rejected.status, Status::Rejected);
        assert_eq!(rejected.origin, Origin::User);
        assert_eq!(
            rejected.confidence, 1.0,
            "this one was certain before it was rejected, and the row still says so"
        );
    }

    #[test]
    fn rejecting_keeps_the_score_it_was_rejected_on() {
        let mut graph = RelationGraph::new();
        let source = field("orders", "userId");
        graph.upsert(Relation::candidate(source.clone(), id("accounts"), 0.6, evidence(10, 6)));

        graph.set_status(&source, &id("accounts"), Status::Rejected);
        let rejected = &graph.relations()[0];
        assert_eq!(rejected.status, Status::Rejected);
        assert_eq!(rejected.origin, Origin::User, "the decision outranks later inference");
        assert!(
            (rejected.confidence - 0.6).abs() < 0.001,
            "a rejected guess that was never certain must not read as certain"
        );
        assert!(!rejected.is_navigable(0.0), "rejected is rejected whatever the score");
    }

    #[test]
    fn a_rename_follows_both_ends_of_a_relation() {
        let mut graph = shop();
        graph.rename_collection("shop", "users", "members");

        assert_eq!(graph.outgoing("shop", "orders", "userId", 0.0)[0].target.collection, "members");
        assert_eq!(graph.referenced_by("shop", "members", 0.0).len(), 1);
        assert!(graph.referenced_by("shop", "users", 0.0).is_empty());
    }

    #[test]
    fn containment_needs_every_probed_value_to_land() {
        // The rule of three: 20 found gives at least 85%, 100 gives at least 97%.
        assert!((evidence(20, 20).containment_lower_bound() - 0.85).abs() < 0.001);
        assert!((evidence(100, 100).containment_lower_bound() - 0.97).abs() < 0.001);
        // One miss is fatal for a foreign key, however many landed.
        assert_eq!(evidence(100, 99).containment_lower_bound(), 0.0);
        assert_eq!(evidence(0, 0).containment_lower_bound(), 0.0);
    }

    #[test]
    fn a_reference_field_is_named_after_its_target() {
        assert_eq!(reference_stem("userId"), "user");
        assert_eq!(reference_stem("user_id"), "user");
        assert_eq!(reference_stem("items[].productId"), "product");
        assert_eq!(reference_stem("tagIds[]"), "tag");
        assert_eq!(reference_stem("owner_ids"), "owner");
        assert_eq!(reference_stem("shipping.address.countryId"), "country");
        // A bare id names no collection, so it keeps its own name rather than becoming empty.
        assert_eq!(reference_stem("_id"), "id");
    }

    #[test]
    fn collections_are_probed_best_name_first() {
        let collections: Vec<String> =
            ["audit_log", "users", "user_profiles", "orders"].map(String::from).to_vec();

        let ranked = rank_target_collections("userId", &collections);
        assert_eq!(ranked[0], "users", "the plural of the stem wins");
        assert_eq!(ranked[1], "user_profiles", "then a name containing it");
        assert!(ranked.contains(&"orders"), "everything stays probeable, just later");

        // Separators and case do not matter.
        assert_eq!(rank_target_collections("userProfileId", &collections)[0], "user_profiles");
    }
}
