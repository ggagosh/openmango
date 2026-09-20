//! Following a reference: decide where it points, confirm it, and go.

use std::time::Duration;

use gpui_kit::{App, AppContext as _, Entity};
use mongodb::Client;
use mongodb::bson::Document;

use crate::connection::ops::relations::{find_by_id_async, probe_id_async};
use crate::state::AppState;
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
