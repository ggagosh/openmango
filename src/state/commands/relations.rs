//! Following a reference: decide where it points, confirm it, and go.

use std::time::Duration;

use gpui_kit::{App, AppContext as _, Entity};
use mongodb::Client;
use mongodb::bson::{Bson, Document};

use std::collections::HashSet;

use chrono::Utc;
use futures::StreamExt as _;

use crate::connection::ops::relations::{find_by_id_async, probe_id_async, probe_ids_async};
use crate::connection::ops::schema::sample_for_schema_async;
use crate::connection::ops::stats::collection_stats_async;
use crate::state::AppState;
use crate::state::StatusMessage;
use crate::state::relations::infer::{
    Candidate as InferCandidate, Inferred, PROBE_ROUNDS, best_per_field, candidates,
    declared_relations, profile_reference_paths, score, should_escalate,
};
use crate::state::relations::lookup::{Anchor, Candidate, Intent, LookupState, ReferenceLookup};
use crate::state::relations::resolve::{Plan, Reference, plan};
use crate::state::relations::{FieldRef, Origin, Relation};

use super::AppCommands;

/// Every query a click makes is bounded. A reference lookup is a `_id` seek, so this only ever
/// fires when something is badly wrong, and the user gets an answer instead of a hang.
const LOOKUP_MAX_TIME: Duration = Duration::from_secs(2);

/// Documents fetched to fill an ambiguous chooser. There are rarely more than two, and reading
/// a dozen full documents to pick one is not worth it.
const MAX_CANDIDATE_PREVIEWS: usize = 5;

/// What the background half of a lookup produced.
struct Found {
    candidates: Vec<Candidate>,
    searched: usize,
    more: usize,
    /// True when the collections came from a name-ranked search rather than a stored relation.
    from_search: bool,
    error: Option<String>,
}

impl AppCommands {
    /// Follow the reference the user clicked at `anchor`.
    ///
    /// The value is always confirmed against the server before anything moves: a name heuristic
    /// alone produces wrong jumps often enough to cost trust, and a stored relation can go stale.
    pub fn follow_reference(
        state: Entity<AppState>,
        anchor: Anchor,
        reference: Reference,
        intent: Intent,
        cx: &mut App,
    ) {
        let session = anchor.session.clone();
        let source = FieldRef::new(&session.database, &session.collection, &anchor.path);

        let Some((client, decision)) = state.update(cx, |state, cx| {
            let active = state.active_connection_by_id(session.connection_id)?;
            let client = active.client.clone();
            let collections =
                active.collections.get(&session.database).cloned().unwrap_or_default();
            let decision = plan(state.relations(), &source, &reference, &collections);
            state.set_reference_lookup(Some(ReferenceLookup::probing(
                anchor.clone(),
                source.clone(),
                reference.clone(),
                intent,
            )));
            cx.notify();
            Some((client, decision))
        }) else {
            return;
        };

        let database = session.database.clone();
        let id = reference.id().clone();
        let task = cx.background_spawn(async move {
            // A known target is asked directly. An unknown one is searched first, cheaply,
            // against each candidate's `_id` index, and only the hits are read in full.
            let (targets, from_search, searched, more) = match decision {
                Plan::Target { target, .. } => (vec![target], false, 1, 0),
                Plan::Search { candidates, more } => {
                    let hits =
                        probe_id_async(&client, &database, &candidates, &id, LOOKUP_MAX_TIME).await;
                    let targets: Vec<FieldRef> = hits
                        .iter()
                        .take(MAX_CANDIDATE_PREVIEWS)
                        .map(|collection| FieldRef::id_of(&database, collection))
                        .collect();
                    (targets, true, candidates.len(), more)
                }
            };
            let mut found = fetch_targets(&client, &targets, &id, from_search, searched).await;
            found.more = more;
            found
        });

        cx.spawn({
            let state = state.clone();
            let anchor = anchor.clone();
            async move |cx: &mut gpui_kit::AsyncApp| {
                let found = task.await;
                cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        apply(state, &anchor, found, cx);
                    });
                });
            }
        })
        .detach();
    }

    /// Put the peek away.
    pub fn dismiss_reference_lookup(state: &Entity<AppState>, cx: &mut App) {
        state.update(cx, |state, cx| {
            if state.reference_lookup().is_some() {
                state.set_reference_lookup(None);
                cx.notify();
            }
        });
    }

    /// Go to a collection the ambiguous chooser offered, remembering the pick when asked.
    pub fn choose_reference_target(state: Entity<AppState>, target: FieldRef, cx: &mut App) {
        state.update(cx, |state, cx| {
            let Some(lookup) = state.reference_lookup().cloned() else {
                return;
            };
            if lookup.remember {
                state.upsert_relation(Relation::asserted(
                    lookup.source.clone(),
                    target.clone(),
                    Origin::User,
                ));
            }
            state.set_reference_lookup(None);
            open_target(state, &lookup.reference, &target, lookup.intent, cx);
        });
    }
}

/// Fetch the document from each target that has it. The query doubles as the existence check,
/// so a target that comes back empty is a broken reference rather than a failure.
async fn fetch_targets(
    client: &Client,
    targets: &[FieldRef],
    id: &mongodb::bson::Bson,
    from_search: bool,
    searched: usize,
) -> Found {
    let mut candidates = Vec::new();
    let mut error = None;
    for target in targets {
        match find_by_id_async(client, &target.database, &target.collection, id, LOOKUP_MAX_TIME)
            .await
        {
            Ok(Some(document)) => candidates.push(Candidate { target: target.clone(), document }),
            Ok(None) => {}
            Err(failure) => error = Some(failure.to_string()),
        }
    }
    Found {
        candidates,
        searched: if from_search { searched } else { targets.len() },
        more: 0,
        from_search,
        error,
    }
}

/// Turn the answer into what the user sees, and move when the click asked to move.
fn apply(
    state: &mut AppState,
    anchor: &Anchor,
    found: Found,
    cx: &mut gpui_kit::Context<AppState>,
) {
    // A second click while this one was in flight owns the popover now.
    let Some(mut lookup) = state.reference_lookup().cloned().filter(|open| &open.anchor == anchor)
    else {
        return;
    };
    lookup.searched = found.from_search;

    lookup.state = match (found.candidates.len(), found.error) {
        (0, Some(message)) => LookupState::Failed(message),
        (0, None) => LookupState::Missing { searched: found.searched, more: found.more },
        (1, _) => LookupState::Found(found.candidates.into_iter().next().expect("one candidate")),
        (_, _) => LookupState::Ambiguous(found.candidates),
    };

    // A single answer found by searching is worth keeping: the next click on this field jumps
    // straight there. A probe hit on an ObjectId is near-proof, so this needs no confirmation.
    if let (LookupState::Found(candidate), true) = (&lookup.state, lookup.searched) {
        let relation =
            Relation::asserted(lookup.source.clone(), candidate.target.clone(), Origin::Probe);
        state.upsert_relation(relation);
    }

    match (&lookup.state, lookup.intent.is_peek()) {
        (LookupState::Found(candidate), false) => {
            let target = candidate.target.clone();
            let reference = lookup.reference.clone();
            let intent = lookup.intent;
            state.set_reference_lookup(None);
            open_target(state, &reference, &target, intent, cx);
        }
        // Everything else — a peek, an empty result, several answers, a failure — stays on
        // screen for the user to read and decide.
        _ => {
            state.set_reference_lookup(Some(lookup));
            cx.notify();
        }
    }
}

/// Show the target, filtered to the one document, with the filter visible and editable.
fn open_target(
    state: &mut AppState,
    reference: &Reference,
    target: &FieldRef,
    intent: Intent,
    cx: &mut gpui_kit::Context<AppState>,
) {
    let filter = mongodb::bson::doc! { "_id": reference.id().clone() };
    let raw = filter_text(&filter);
    let database = target.database.clone();
    let collection = target.collection.clone();

    if intent == Intent::OpenInNewTab {
        state.open_collection_in_new_tab(database, collection, raw, Some(filter), cx);
    } else {
        state.navigate_to_collection(database, collection, raw, Some(filter), cx);
    }
}

/// The filter as the user will see it in the filter bar: `{_id: ObjectId("…")}`, not Extended
/// JSON. The same rendering the workspace uses, so a navigated filter and a typed one match.
fn filter_text(filter: &Document) -> String {
    crate::bson::format_relaxed_json_compact(
        &mongodb::bson::Bson::Document(filter.clone()).into_relaxed_extjson(),
    )
}

/// A sample big enough to meet the rare fields, small enough not to be a scan.
///
/// The budget is bytes rather than documents, because a thousand 40 KB documents is 40 MB of
/// reads and a thousand 200-byte ones is nothing. Sized from `collStats`' average, which is
/// free, and clamped so a wrong average cannot turn into a huge read.
const SAMPLE_BYTE_BUDGET: u64 = 5 * 1024 * 1024;
const MIN_SAMPLE: u64 = 200;
const MAX_SAMPLE: u64 = 2_000;
const DEFAULT_SAMPLE: u64 = 1_000;

/// Inference reads in bulk, so it is gentler than a click: fewer workers, and every query is
/// still capped.
const INFER_CONCURRENCY: usize = 4;
const INFER_MAX_TIME: Duration = Duration::from_secs(10);

impl AppCommands {
    /// Work out what the fields of a collection point at, and store what the data confirms.
    ///
    /// Explicitly triggered. Inference reads a sample of the collection and probes its
    /// neighbours, which is more than a click should ever do on its own.
    pub fn infer_relations(
        state: Entity<AppState>,
        database: String,
        collection: String,
        cx: &mut App,
    ) {
        let Some((client, collections)) =
            state.read(cx).selected_connection_id().and_then(|connection_id| {
                let active = state.read(cx).active_connection_by_id(connection_id)?;
                Some((
                    active.client.clone(),
                    active.collections.get(&database).cloned().unwrap_or_default(),
                ))
            })
        else {
            return;
        };

        state.update(cx, |state, cx| {
            state.set_status_message(Some(StatusMessage::info(format!(
                "Looking for relations in {database}.{collection}…"
            ))));
            cx.notify();
        });

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move { infer(&client, &database, &collection, &collections).await }
        });

        cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
            let outcome = task.await;
            cx.update(|cx| {
                state.update(cx, |state, cx| {
                    let message = match outcome {
                        Err(error) => {
                            StatusMessage::error(format!("Couldn't look for relations: {error}"))
                        }
                        Ok(inferred) => {
                            let found = inferred.relations.len();
                            for relation in inferred.relations {
                                state.upsert_relation(relation);
                            }
                            match found {
                                0 => StatusMessage::info(format!(
                                    "No relations found in {database}.{collection}."
                                )),
                                1 => StatusMessage::info("1 relation found.".to_string()),
                                count => StatusMessage::info(format!("{count} relations found.")),
                            }
                        }
                    };
                    state.set_status_message(Some(message));
                    cx.notify();
                });
            });
        })
        .detach();
    }
}

/// Sample the collection, pair every reference-shaped field with the collections worth asking,
/// and keep what the data confirms.
async fn infer(
    client: &Client,
    database: &str,
    collection: &str,
    collections: &[String],
) -> crate::error::Result<Inferred> {
    let sample_size = sample_size_for(client, database, collection).await;
    let (documents, _) =
        sample_for_schema_async(client, database, collection, sample_size, INFER_MAX_TIME).await?;
    if documents.is_empty() {
        return Ok(Inferred::default());
    }
    let sampled = documents.len() as u64;

    let profiles = profile_reference_paths(&documents);
    let candidates = candidates(database, collection, &profiles, collections);
    let confirmed: Vec<Relation> = futures::stream::iter(
        candidates
            .into_iter()
            .map(|candidate| {
                let client = client.clone();
                async move { confirm(&client, candidate, sampled).await }
            })
            .collect::<Vec<_>>(),
    )
    .buffer_unordered(INFER_CONCURRENCY)
    .collect::<Vec<Option<Relation>>>()
    .await
    .into_iter()
    .flatten()
    .collect();

    // What the documents assert outright comes first and is never displaced: a DBRef names its
    // collection, which beats anything a probe can conclude.
    let mut relations = declared_relations(database, collection, &documents);
    let declared_paths: HashSet<String> =
        relations.iter().map(|relation| relation.source.path.clone()).collect();
    relations.extend(
        best_per_field(confirmed)
            .into_iter()
            .filter(|relation| !declared_paths.contains(&relation.source.path)),
    );
    let placed: HashSet<&String> = relations.iter().map(|relation| &relation.source.path).collect();
    let unresolved = profiles
        .iter()
        .map(|profile| profile.path.clone())
        .filter(|path| !placed.contains(path))
        .collect();

    Ok(Inferred { relations, unresolved })
}

/// Probe one candidate, sending more ids only while every one of them keeps landing.
async fn confirm(client: &Client, candidate: InferCandidate, sampled: u64) -> Option<Relation> {
    let mut best = None;
    for round in 0..PROBE_ROUNDS.len() {
        let ids = candidate.round(round);
        if ids.is_empty() {
            break;
        }
        let hits = probe_ids_async(
            client,
            &candidate.target.database,
            &candidate.target.collection,
            ids,
            INFER_MAX_TIME,
        )
        .await
        .ok()?;

        best = score(&candidate, ids.len() as u32, hits as u32, sampled, Utc::now());
        best.as_ref()?;
        if !should_escalate(ids.len(), hits, candidate.ids.len()) {
            break;
        }
    }
    best
}

/// Documents to sample, from the collection's average document size.
///
/// A collection too new or too small to report an average gets the default; being wrong there
/// costs one modest sample, and the clamp keeps a wrong answer from becoming a big read.
async fn sample_size_for(client: &Client, database: &str, collection: &str) -> u64 {
    let average = collection_stats_async(client, database, collection, INFER_MAX_TIME)
        .await
        .ok()
        .and_then(|stats| {
            let storage = stats.get_document("storageStats").ok()?;
            number(storage.get("avgObjSize")?)
        })
        .filter(|size| *size > 0.0);

    match average {
        Some(average) => {
            ((SAMPLE_BYTE_BUDGET as f64 / average) as u64).clamp(MIN_SAMPLE, MAX_SAMPLE)
        }
        None => DEFAULT_SAMPLE,
    }
}

/// `$collStats` reports sizes as whichever integer type fits, so read them all.
fn number(value: &Bson) -> Option<f64> {
    match value {
        Bson::Double(size) => Some(*size),
        Bson::Int32(size) => Some(*size as f64),
        Bson::Int64(size) => Some(*size as f64),
        _ => None,
    }
}
