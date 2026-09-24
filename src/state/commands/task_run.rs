//! What a task run needs to recover: its own connections, waits between attempts, and a watch
//! for runs that stall or run too long.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use futures::future::Either;
use gpui_kit::{AppContext as _, AsyncApp};
use mongodb::Client;
use uuid::Uuid;

use crate::connection::{CancellationToken, ConnectionManager};
use crate::error::Failure;
use crate::models::SavedConnection;

/// Attempts per step: the first try and four retries.
pub(super) const ATTEMPTS: u32 = 5;
/// Retrying stops once a run has spent this long waiting and retrying.
pub(super) const RETRY_BUDGET: Duration = Duration::from_secs(15 * 60);
/// A step with no progress for this long is stopped and retried like a timeout.
pub(super) const STALL: Duration = Duration::from_secs(10 * 60);
/// A run still going after this long stops as failed.
pub(super) const RUN_LIMIT: Duration = Duration::from_secs(24 * 60 * 60);

/// A random wait before retry `attempt` (1 for the first retry): full jitter, between zero and
/// the smaller of 60 seconds and 2 seconds × 2^attempt.
pub(super) fn backoff(attempt: u32) -> Duration {
    let cap = (2u64 << attempt.min(5)).min(60) * 1000;
    Duration::from_millis(rand::random_range(0..=cap))
}

/// Waits `duration`, or less if the run is cancelled. Time is counted in the executor's timer
/// steps, so tests can move it forward.
pub(super) async fn pause(cx: &mut AsyncApp, duration: Duration, run: &CancellationToken) {
    let mut waited = Duration::ZERO;
    while waited < duration && !run.is_cancelled() {
        let step = (duration - waited).min(Duration::from_millis(250));
        cx.background_executor().timer(step).await;
        waited += step;
    }
}

/// The connections a run opened for itself. Each has its own SSH tunnel, so a run never
/// disturbs the connections open in the sidebar, and a retry can start from fresh ones.
pub(super) struct RunConnections {
    manager: Arc<ConnectionManager>,
    /// Saved connection id, tunnel key, client, and the address the BSON tools use.
    open: Vec<(Uuid, Uuid, Client, Option<String>)>,
}

impl RunConnections {
    /// Blocking: connects to each saved connection and checks it answers.
    pub(super) fn open(
        manager: Arc<ConnectionManager>,
        saved: &[SavedConnection],
    ) -> Result<Self, Failure> {
        let mut connections = Self { manager, open: Vec::new() };
        for connection in saved {
            // The tunnel is keyed by a fresh id, never the sidebar's, so neither stops the other.
            let key = Uuid::new_v4();
            let (client, meta) =
                connections.manager.connect_managed(key, connection).map_err(Failure::from)?;
            let tool_uri =
                connections.manager.effective_uri_for_active_connection(connection, &meta).ok();
            connections.open.push((connection.id, key, client, tool_uri));
        }
        Ok(connections)
    }

    pub(super) fn client(&self, id: Uuid) -> Option<Client> {
        self.open.iter().find(|(connection, ..)| *connection == id).map(|(_, _, c, _)| c.clone())
    }

    /// The clients, for a transfer to use instead of the sidebar's.
    pub(super) fn task_clients(
        &self,
    ) -> std::collections::HashMap<Uuid, crate::state::app_state::TaskClient> {
        self.open
            .iter()
            .map(|(id, _, client, tool_uri)| {
                let own = crate::state::app_state::TaskClient {
                    client: client.clone(),
                    tool_uri: tool_uri.clone(),
                };
                (*id, own)
            })
            .collect()
    }
}

impl Drop for RunConnections {
    fn drop(&mut self) {
        for (_, key, ..) in self.open.drain(..) {
            self.manager.disconnect(key);
        }
    }
}

/// Opens a run's connections again, off the UI thread.
pub(super) struct Reconnect {
    pub manager: Arc<ConnectionManager>,
    pub saved: Vec<SavedConnection>,
}

impl Reconnect {
    pub(super) async fn open(&self, cx: &mut AsyncApp) -> Result<RunConnections, Failure> {
        let (manager, saved) = (self.manager.clone(), self.saved.clone());
        cx.background_spawn(async move { RunConnections::open(manager, &saved) }).await
    }
}

/// What a message from a collection-by-collection engine says about its collection.
pub(super) enum Event<S> {
    Done(S),
    /// The attempt was stopped before the collection finished.
    Cancelled,
    Failed(Failure),
    Other,
}

/// A progress message from an engine that works one collection at a time.
pub(super) trait Step {
    type Summary;
    fn event(&self) -> (usize, Event<Self::Summary>);
}

impl Step for crate::connection::ops::compare_database::PairMessage {
    type Summary = crate::connection::ops::compare::CompareSummary;
    fn event(&self) -> (usize, Event<Self::Summary>) {
        use crate::connection::ops::compare_database::PairMessage;
        match self {
            PairMessage::Done(i, summary) if summary.cancelled => (*i, Event::Cancelled),
            PairMessage::Done(i, summary) => (*i, Event::Done(summary.clone())),
            PairMessage::Failed(i, failure) => (*i, Event::Failed(failure.clone())),
            PairMessage::Started(i) | PairMessage::Progress(i, _) => (*i, Event::Other),
        }
    }
}

impl Step for crate::connection::ops::compare_database::PairSyncMessage {
    type Summary = crate::connection::ops::compare_sync::SyncSummary;
    fn event(&self) -> (usize, Event<Self::Summary>) {
        use crate::connection::ops::compare_database::PairSyncMessage;
        match self {
            PairSyncMessage::Done(i, summary) if summary.cancelled => (*i, Event::Cancelled),
            PairSyncMessage::Done(i, summary) => (*i, Event::Done(summary.clone())),
            PairSyncMessage::Failed(i, failure) => (*i, Event::Failed(failure.clone())),
            PairSyncMessage::Started(i, _) | PairSyncMessage::Progress(i, _) => (*i, Event::Other),
        }
    }
}

/// What `run_steps` ended with, per collection index.
pub(super) struct Finished<S> {
    pub done: std::collections::HashMap<usize, S>,
    pub failed: std::collections::HashMap<usize, Failure>,
    /// The whole attempt failed, not one collection, and trying again won't help.
    pub error: Option<Failure>,
}

impl<S> Default for Finished<S> {
    fn default() -> Self {
        Self { done: Default::default(), failed: Default::default(), error: None }
    }
}

/// Runs `pending` collections through `start` until each is done. A collection that fails for a
/// reason that can pass, stalls, or isn't reached is tried again after a wait, with fresh
/// connections: at most `ATTEMPTS` times and within `RETRY_BUDGET`. Every message goes to `on`
/// after it has been read here.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_steps<M, Fut>(
    cx: &mut AsyncApp,
    watch: &mut Watch,
    reconnect: &Reconnect,
    connections: &mut RunConnections,
    mut pending: Vec<usize>,
    mut log: impl FnMut(&mut AsyncApp, String),
    mut start: impl FnMut(&RunConnections, Vec<usize>, CancellationToken) -> (UnboundedReceiver<M>, Fut),
    mut on: impl FnMut(&mut AsyncApp, M),
) -> Finished<M::Summary>
where
    M: Step,
    Fut: std::future::Future<Output = Result<(), Failure>>,
{
    let mut finished = Finished::default();
    for attempt in 1..=ATTEMPTS {
        let token = CancellationToken::new();
        let (mut receiver, work) = start(connections, pending.clone(), token.clone());
        let mut last_failure: Option<Failure> = None;
        watch
            .drain(cx, &token, &mut receiver, STALL, |cx, message| {
                let (index, event) = message.event();
                match event {
                    Event::Done(summary) => {
                        finished.done.insert(index, summary);
                    }
                    Event::Failed(failure) if !failure.transient => {
                        finished.failed.insert(index, failure);
                    }
                    Event::Failed(failure) => last_failure = Some(failure),
                    Event::Cancelled | Event::Other => {}
                }
                on(cx, message);
            })
            .await;
        match work.await {
            Ok(()) => {}
            Err(failure) if failure.transient => last_failure = Some(failure),
            Err(failure) => {
                finished.error = Some(failure);
                return finished;
            }
        }
        pending.retain(|index| {
            !finished.done.contains_key(index) && !finished.failed.contains_key(index)
        });
        if pending.is_empty() || watch.over() {
            return finished;
        }
        let reason = if watch.stalled {
            format!("No progress for {} minutes", STALL.as_secs() / 60)
        } else {
            last_failure.map(|failure| failure.message).unwrap_or_else(|| "It stopped early".into())
        };
        if attempt == ATTEMPTS || !watch.may_retry() {
            for index in &pending {
                finished.failed.insert(index.to_owned(), Failure::temporary(reason.clone()));
            }
            return finished;
        }
        let wait = backoff(attempt);
        log(
            cx,
            format!(
                "{reason}. Trying {} collection{} again (attempt {} of {ATTEMPTS}) after {} s.",
                pending.len(),
                if pending.len() == 1 { "" } else { "s" },
                attempt + 1,
                wait.as_secs()
            ),
        );
        pause(cx, wait, &watch.run).await;
        if watch.over() {
            return finished;
        }
        match reconnect.open(cx).await {
            Ok(fresh) => *connections = fresh,
            Err(failure) => log(cx, format!("Couldn't reconnect yet: {failure}")),
        }
    }
    finished
}

/// Runs a quick step, such as listing collections, until it succeeds or fails for good. A failure
/// that can pass is tried again after a wait, with fresh connections.
pub(super) async fn retry_once<T, Fut>(
    cx: &mut AsyncApp,
    watch: &mut Watch,
    reconnect: &Reconnect,
    connections: &mut RunConnections,
    mut log: impl FnMut(&mut AsyncApp, String),
    mut attempt: impl FnMut(&RunConnections) -> Fut,
) -> Result<T, Failure>
where
    Fut: std::future::Future<Output = Result<T, Failure>>,
{
    let mut tries = 1;
    loop {
        let failure = match attempt(connections).await {
            Ok(value) => return Ok(value),
            Err(failure) => failure,
        };
        if !failure.transient || tries == ATTEMPTS || watch.over() || !watch.may_retry() {
            return Err(failure);
        }
        let wait = backoff(tries);
        log(
            cx,
            format!(
                "{failure}. Trying again (attempt {} of {ATTEMPTS}) after {} s.",
                tries + 1,
                wait.as_secs()
            ),
        );
        pause(cx, wait, &watch.run).await;
        if watch.over() {
            return Err(failure);
        }
        match reconnect.open(cx).await {
            Ok(fresh) => *connections = fresh,
            Err(failure) => log(cx, format!("Couldn't reconnect yet: {failure}")),
        }
        tries += 1;
    }
}

/// Opens a run's own connections, waiting and trying again while the server can't be reached.
pub(super) async fn open_connections(
    cx: &mut AsyncApp,
    watch: &mut Watch,
    reconnect: &Reconnect,
    mut log: impl FnMut(&mut AsyncApp, String),
) -> Result<RunConnections, Failure> {
    let mut tries = 1;
    loop {
        let failure = match reconnect.open(cx).await {
            Ok(connections) => return Ok(connections),
            Err(failure) => failure,
        };
        if !failure.transient || tries == ATTEMPTS || watch.over() || !watch.may_retry() {
            return Err(failure);
        }
        let wait = backoff(tries);
        log(
            cx,
            format!(
                "Couldn't connect: {failure}. Trying again (attempt {} of {ATTEMPTS}) after {} s.",
                tries + 1,
                wait.as_secs()
            ),
        );
        pause(cx, wait, &watch.run).await;
        tries += 1;
    }
}

/// Watches one attempt: it stops the attempt when the run is cancelled, when nothing has arrived
/// for `STALL`, or when the run has gone on past `RUN_LIMIT`.
pub(super) struct Watch {
    pub run: CancellationToken,
    pub started: Instant,
    pub stalled: bool,
    pub expired: bool,
    /// When the run first had to retry; retrying stops `RETRY_BUDGET` after it.
    pub retrying_since: Option<Instant>,
    /// Retrying also stops at this time: a scheduled run's next run is due.
    pub until: Option<Instant>,
}

impl Watch {
    pub(super) fn new(run: CancellationToken, until: Option<Instant>) -> Self {
        Self {
            run,
            started: Instant::now(),
            stalled: false,
            expired: false,
            retrying_since: None,
            until,
        }
    }

    /// Whether the run may retry once more. The budget starts at its first retry.
    pub(super) fn may_retry(&mut self) -> bool {
        self.until.is_none_or(|until| Instant::now() < until)
            && self.retrying_since.get_or_insert_with(Instant::now).elapsed() < RETRY_BUDGET
    }

    /// Reads the attempt's messages until its sender is gone, handing each to `on`.
    pub(super) async fn drain<T>(
        &mut self,
        cx: &mut AsyncApp,
        attempt: &CancellationToken,
        receiver: &mut UnboundedReceiver<T>,
        stall: Duration,
        mut on: impl FnMut(&mut AsyncApp, T),
    ) {
        self.stalled = false;
        let mut last = Instant::now();
        loop {
            let tick = cx.background_executor().timer(Duration::from_secs(1));
            match futures::future::select(receiver.next(), tick).await {
                Either::Left((Some(message), _)) => {
                    last = Instant::now();
                    on(cx, message);
                }
                Either::Left((None, _)) => return,
                Either::Right(_) => {
                    if self.run.is_cancelled() {
                        attempt.cancel();
                    }
                    if self.started.elapsed() > RUN_LIMIT {
                        self.expired = true;
                        attempt.cancel();
                    }
                    if !self.stalled && last.elapsed() > stall {
                        self.stalled = true;
                        attempt.cancel();
                    }
                }
            }
        }
    }

    /// The run should end: cancelled, or past its limit.
    pub(super) fn over(&self) -> bool {
        self.run.is_cancelled() || self.expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    fn a_step_with_no_progress_is_stopped_as_stalled(cx: &mut gpui_kit::TestAppContext) {
        let (sender, mut receiver) = futures::channel::mpsc::unbounded::<()>();
        let attempt = CancellationToken::new();
        let watched = attempt.clone();
        let stalled = std::rc::Rc::new(std::cell::Cell::new(None));
        let result = stalled.clone();
        cx.spawn(async move |mut cx| {
            let mut watch = Watch::new(CancellationToken::new(), None);
            watch.drain(&mut cx, &watched, &mut receiver, Duration::ZERO, |_, _| {}).await;
            result.set(Some(watch.stalled));
        })
        .detach();
        // A tick finds no progress, so the attempt is stopped...
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert!(attempt.is_cancelled());
        // ...and once the engine stops sending, the watch says why.
        drop(sender);
        cx.run_until_parked();
        assert_eq!(stalled.get(), Some(true));
    }

    #[test]
    fn a_server_that_cant_be_reached_is_a_failure_that_can_pass() {
        let closed = SavedConnection::new(
            "Closed".into(),
            "mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=200".into(),
        );
        let Err(failure) = RunConnections::open(Arc::new(ConnectionManager::new()), &[closed])
        else {
            panic!("nothing listens on port 1");
        };
        assert!(failure.transient, "{failure}");
        assert!(!failure.sign_in);
    }

    #[test]
    fn waits_grow_with_each_retry_and_never_pass_a_minute() {
        for attempt in 1..=ATTEMPTS {
            let cap = Duration::from_secs((2u64 << attempt).min(60));
            for _ in 0..50 {
                assert!(backoff(attempt) <= cap);
            }
        }
        assert!(backoff(10) <= Duration::from_secs(60));
    }
}
