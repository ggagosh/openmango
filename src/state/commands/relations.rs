//! Following a reference: decide where it points, confirm it, and go.

use std::time::Duration;

use gpui_kit::{App, AppContext as _, Entity};
use mongodb::Client;
use mongodb::bson::{Bson, Document};

use std::collections::HashSet;

use chrono::Utc;
use futures::StreamExt as _;

use mongodb::bson::doc;
use uuid::Uuid;

use crate::connection::ops::documents::{AsyncFindOptions, find_documents_async};
use crate::connection::ops::indexes::list_indexes_async;
use crate::connection::ops::relations::{find_by_id_async, probe_id_async, probe_ids_async};
use crate::connection::ops::schema::sample_for_schema_async;
use crate::connection::ops::stats::collection_stats_async;
use crate::state::AppState;
use crate::state::StatusMessage;
use crate::state::relations::infer::{
    Candidate as InferCandidate, InferenceRun, Inferred, PROBE_ROUNDS, best_per_field, candidates,
    declared_relations, profile_reference_paths, score, should_escalate,
};
use crate::state::relations::lookup::{Anchor, Candidate, Intent, LookupState, ReferenceLookup};
use crate::state::relations::references::{GROUP_PREVIEW_LIMIT, GroupState, ReferenceGroup};
use crate::state::relations::resolve::{NAVIGATION_CONFIDENCE, Plan, Reference, plan};
use crate::state::relations::{FieldRef, Origin, Relation};
use crate::state::relations::{filter_text, mongo_path};

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

        // The driver's work belongs to the connection's Tokio runtime; gpui's executor has no
        // reactor for it to spawn onto.
        let runtime = state.read(cx).connection_manager().runtime_handle();
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
        let task = runtime.spawn(async move {
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
                let Ok(found) = task.await else {
                    return;
                };
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

        let runtime = state.read(cx).connection_manager().runtime_handle();
        state.update(cx, |state, cx| {
            state.set_status_message(Some(StatusMessage::info(format!(
                "Looking for relations in {database}.{collection}…"
            ))));
            cx.notify();
        });

        let task = runtime.spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move { infer(&client, &database, &collection, &collections).await }
        });

        cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
            let outcome = match task.await {
                Ok(outcome) => outcome,
                Err(error) => Err(crate::error::Error::Parse(format!(
                    "The relation search could not finish: {error}"
                ))),
            };
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

impl AppCommands {
    /// Work out what every collection in a database points at.
    ///
    /// Collections are read one at a time rather than all at once: a database of sixty is sixty
    /// samples and several hundred probes, and doing that in a burst is the kind of read a
    /// production server feels. One at a time is slower, stoppable, and unremarkable.
    pub fn infer_relations_for_database(state: Entity<AppState>, database: String, cx: &mut App) {
        if state.read(cx).inference_run().is_some() {
            return;
        }
        let Some((client, collections)) =
            state.read(cx).selected_connection_id().and_then(|connection_id| {
                let active = state.read(cx).active_connection_by_id(connection_id)?;
                let collections: Vec<String> = active
                    .collections
                    .get(&database)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|name| !name.starts_with("system."))
                    .collect();
                Some((active.client.clone(), collections))
            })
        else {
            return;
        };
        if collections.is_empty() {
            return;
        }

        let runtime = state.read(cx).connection_manager().runtime_handle();
        let run = InferenceRun::new(database.clone(), collections.len());
        let cancelled = run.cancel_flag();
        state.update(cx, |state, cx| {
            state.set_inference_run(Some(run));
            cx.notify();
        });

        cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
            let mut found = 0usize;
            let mut read = 0usize;
            for (index, collection) in collections.iter().enumerate() {
                if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        if let Some(run) = state.inference_run_mut() {
                            run.collection = collection.clone();
                            run.done = index;
                            run.found = found;
                        }
                        cx.notify();
                    });
                });

                let task = runtime.spawn({
                    let client = client.clone();
                    let database = database.clone();
                    let collection = collection.clone();
                    let collections = collections.clone();
                    async move { infer(&client, &database, &collection, &collections).await }
                });
                let Ok(Ok(inferred)) = task.await else {
                    read += 1;
                    continue;
                };
                read += 1;
                found += inferred.relations.len();

                cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        for relation in inferred.relations {
                            state.upsert_relation(relation);
                        }
                        cx.notify();
                    });
                });
            }

            cx.update(|cx| {
                state.update(cx, |state, cx| {
                    let stopped = state.inference_run().is_some_and(|run| run.is_cancelled());
                    state.set_inference_run(None);
                    state.set_status_message(Some(StatusMessage::info(if stopped {
                        format!(
                            "Stopped after {read} of {} collections. {found} relations found.",
                            collections.len()
                        )
                    } else {
                        match found {
                            0 => format!("No relations found in {database}."),
                            1 => format!("1 relation found in {database}."),
                            count => format!("{count} relations found in {database}."),
                        }
                    })));
                    cx.notify();
                });
            });
        })
        .detach();
    }

    /// Stop a relation search between collections.
    pub fn cancel_inference(state: &Entity<AppState>, cx: &mut App) {
        state.update(cx, |state, cx| {
            if let Some(run) = state.inference_run() {
                run.cancel();
                cx.notify();
            }
        });
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

/// A reference lookup runs once per incoming relation, so each one is capped on its own.
const REFERENCES_MAX_TIME: Duration = Duration::from_secs(5);

impl AppCommands {
    /// Open a tab answering what points at this document, and start filling it.
    pub fn find_references(state: Entity<AppState>, target: FieldRef, id: Bson, cx: &mut App) {
        let Some(tab_id) =
            state.update(cx, |state, cx| state.open_references_tab(target.clone(), id.clone(), cx))
        else {
            return;
        };
        Self::load_references(state, tab_id, cx);
    }

    /// Work out which fields point at the tab's document, then ask each of them.
    ///
    /// Only the graph is consulted for *which* fields: guessing here would mean scanning
    /// collections on the strength of a name, which is the one thing a reference lookup never
    /// does. A database nobody has inferred yet says so, and offers to infer.
    pub fn load_references(state: Entity<AppState>, tab_id: Uuid, cx: &mut App) {
        let runtime = state.read(cx).connection_manager().runtime_handle();
        let Some((client, id, groups, guard_scans)) = state.update(cx, |state, cx| {
            let tab = state.references_tab(tab_id)?;
            let target = tab.target.clone();
            let id = tab.id.clone();
            let connection_id = state.selected_connection_id()?;
            let client = state.active_connection_by_id(connection_id)?.client.clone();
            // An unindexed scan is not something to start unasked where it would be felt.
            let guard_scans = state
                .connection_by_id(connection_id)
                .map(|connection| {
                    connection.protected
                        || connection.environment
                            == Some(crate::models::ConnectionEnvironment::Production)
                })
                .unwrap_or(false);

            let groups: Vec<FieldRef> = state
                .relations()
                .referenced_by(&target.database, &target.collection, NAVIGATION_CONFIDENCE)
                .into_iter()
                .map(|relation| relation.source.clone())
                .collect();

            let tab = state.references_tab_mut(tab_id)?;
            tab.discovering = false;
            // One answer needs no choosing between; several are a summary until one is picked.
            let single = groups.len() == 1;
            tab.groups = groups
                .iter()
                .map(|source| ReferenceGroup {
                    source: source.clone(),
                    indexed: false,
                    expanded: single,
                    state: GroupState::Loading,
                })
                .collect();
            cx.notify();
            Some((client, id, groups, guard_scans))
        }) else {
            return;
        };

        for source in groups {
            let task = runtime.spawn({
                let client = client.clone();
                let source = source.clone();
                let id = id.clone();
                async move { load_group(&client, source, id, guard_scans).await }
            });
            cx.spawn({
                let state = state.clone();
                let source = source.clone();
                async move |cx: &mut gpui_kit::AsyncApp| {
                    let Ok((indexed, group_state)) = task.await else {
                        return;
                    };
                    cx.update(|cx| {
                        state.update(cx, |state, cx| {
                            if let Some(tab) = state.references_tab_mut(tab_id)
                                && let Some(group) = tab.group_mut(&source)
                            {
                                group.indexed = indexed;
                                group.state = group_state;
                                cx.notify();
                            }
                        });
                    });
                }
            })
            .detach();
        }
    }

    /// Run a group that was held back because its field has no index.
    pub fn run_held_reference_group(
        state: Entity<AppState>,
        tab_id: Uuid,
        source: FieldRef,
        cx: &mut App,
    ) {
        let runtime = state.read(cx).connection_manager().runtime_handle();
        let Some((client, id)) = state.update(cx, |state, cx| {
            let id = state.references_tab(tab_id)?.id.clone();
            let connection_id = state.selected_connection_id()?;
            let client = state.active_connection_by_id(connection_id)?.client.clone();
            let group = state.references_tab_mut(tab_id)?.group_mut(&source)?;
            group.state = GroupState::Loading;
            cx.notify();
            Some((client, id))
        }) else {
            return;
        };

        let task = runtime.spawn({
            let source = source.clone();
            async move { load_group(&client, source, id, false).await }
        });
        cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
            let Ok((indexed, group_state)) = task.await else {
                return;
            };
            cx.update(|cx| {
                state.update(cx, |state, cx| {
                    if let Some(tab) = state.references_tab_mut(tab_id)
                        && let Some(group) = tab.group_mut(&source)
                    {
                        group.indexed = indexed;
                        group.state = group_state;
                        cx.notify();
                    }
                });
            });
        })
        .detach();
    }
}

/// Ask one field what it points at, checking first whether asking is cheap.
///
/// One document over the limit is fetched rather than counted, so a full page reads as "20+"
/// without a `count` that would scan the collection to say the same thing.
async fn load_group(
    client: &Client,
    source: FieldRef,
    id: Bson,
    guard_scans: bool,
) -> (bool, GroupState) {
    let path = mongo_path(&source.path);
    let indexed = path_is_indexed(client, &source, &path).await;
    if guard_scans && !indexed {
        return (indexed, GroupState::Held);
    }

    let found = find_documents_async(
        client,
        &source.database,
        &source.collection,
        AsyncFindOptions {
            filter: doc! { path: id },
            sort: None,
            projection: None,
            skip: 0,
            limit: GROUP_PREVIEW_LIMIT as i64 + 1,
            max_time: REFERENCES_MAX_TIME,
        },
    )
    .await;

    match found {
        Ok(mut documents) => {
            let more = documents.len() > GROUP_PREVIEW_LIMIT;
            documents.truncate(GROUP_PREVIEW_LIMIT);
            (indexed, GroupState::Loaded { documents, more })
        }
        Err(error) => (indexed, GroupState::Failed(error.to_string())),
    }
}

/// Whether some index starts with this path, which is what decides if a lookup is a seek or a
/// scan. A compound index counts when the path is its first key.
async fn path_is_indexed(client: &Client, source: &FieldRef, path: &str) -> bool {
    let Ok(indexes) =
        list_indexes_async(client, &source.database, &source.collection, REFERENCES_MAX_TIME).await
    else {
        return false;
    };
    indexes.iter().any(|index| index.keys.keys().next().is_some_and(|first| first == path))
}
