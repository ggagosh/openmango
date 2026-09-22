use std::sync::Arc;

use futures::StreamExt;
use gpui_kit::{App, AppContext as _, Entity, Window};
use uuid::Uuid;

use super::AppCommands;
use crate::components::{WriteConfirmation, WriteRequest, request_connection_write};
use crate::connection::ops::compare_sync::{
    Operation, SyncProgress, SyncSummary, restore::RestoreHandle, sync_collections_async,
    undo_sync_async,
};
use crate::connection::{CancellationToken, ops::compare::Side};
use crate::error::{Error, Result};
use crate::models::ConnectionWriteIdentity;
use crate::state::compare::CompareConfig;
use crate::state::compare_sync::SyncPlan;
use crate::state::{AppEvent, AppState, SessionKey};

enum Work {
    Sync(SyncPlan),
    Undo { run: u64, config: CompareConfig, target: Side, restore: Arc<RestoreHandle> },
}

impl Work {
    fn config(&self) -> &CompareConfig {
        match self {
            Self::Sync(p) => &p.config,
            Self::Undo { config, .. } => config,
        }
    }
    fn target(&self) -> Side {
        match self {
            Self::Sync(p) => p.target,
            Self::Undo { target, .. } => *target,
        }
    }
    fn run(&self) -> u64 {
        match self {
            Self::Sync(p) => p.run,
            Self::Undo { run, .. } => *run,
        }
    }
    fn undo(&self) -> bool {
        matches!(self, Self::Undo { .. })
    }
    fn matches(&self, app: &AppState, id: Uuid) -> bool {
        let Some(tab) = app.compare_tab(id) else {
            return false;
        };
        match self {
            Self::Sync(plan) => plan.matches(tab),
            Self::Undo { run, config, target, restore } => {
                *run == tab.run
                    && config == tab.results_config()
                    && tab.sync.target == Some(*target)
                    && !tab.running
                    && !tab.sync.running
                    && tab
                        .sync
                        .restore
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, restore))
            }
        }
    }
}

fn report(state: &Entity<AppState>, id: Uuid, message: String, cx: &mut App) {
    state.update(cx, |app, cx| {
        if let Some(tab) = app.compare_tab_mut(id) {
            tab.sync.error = Some(message);
        }
        cx.notify();
    });
}

impl AppCommands {
    pub fn review_compare_sync(
        state: Entity<AppState>,
        id: Uuid,
        undo: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let app = state.read(cx);
        if let Some(reason) = app.compare_sync_disabled_reason(id, undo) {
            report(&state, id, reason, cx);
            return;
        }
        let Some(tab) = app.compare_tab(id) else {
            return;
        };
        let work = if undo {
            let Some(restore) = tab.sync.restore.as_ref().filter(|r| r.pending() > 0) else {
                return;
            };
            Work::Undo {
                run: tab.run,
                config: tab.results_config().clone(),
                target: tab.sync.target.unwrap(),
                restore: restore.clone(),
            }
        } else {
            let Some(plan) = SyncPlan::from_tab(tab) else {
                return;
            };
            Work::Sync(plan)
        };
        let index = if work.target() == Side::Left { 0 } else { 1 };
        let endpoint = &work.config().sides[index];
        let connection_id = endpoint.connection_id.unwrap();
        let identities: Vec<_> = work
            .config()
            .sides
            .iter()
            .enumerate()
            .filter(|(i, _)| !undo || *i == index)
            .filter_map(|(_, side)| {
                side.connection_id.and_then(|id| {
                    app.connection_by_id(id).map(|c| (id, ConnectionWriteIdentity::from(c)))
                })
            })
            .collect();
        if identities.len() != if undo { 1 } else { 2 } {
            return;
        }
        let name =
            app.connection_by_id(connection_id).map(|c| c.name.as_str()).unwrap_or("Connection");
        let target_label = format!("{name} · {}", endpoint.namespace());
        let message = match &work {
            Work::Sync(plan) => {
                let count = |op| plan.items.iter().filter(|item| item.operation == op).count();
                format!(
                    "Insert {}, replace {}, delete {} in {target_label}.\n\nChanged documents and ambiguous keys are skipped. Replacements keep the target _id. Undo is available until this tab closes or you compare again.{}",
                    count(Operation::Insert),
                    count(Operation::Replace),
                    count(Operation::Delete),
                    if tab.summary.as_ref().is_some_and(|s| s.truncated) {
                        " Only the stored differences are included; this comparison reached its result limit."
                    } else {
                        ""
                    }
                )
            }
            Work::Undo { restore, .. } => format!(
                "Undo up to {} writes in {target_label}.\n\nDocuments changed since sync are skipped. {} writes have uncertain acknowledgements and will only be undone if their intended result is still present.",
                restore.pending(),
                restore.uncertain()
            ),
        };
        request_connection_write(
            state.clone(),
            WriteRequest::new(
                connection_id,
                target_label,
                if undo { "Undo sync" } else { "Sync collections" },
                Some(WriteConfirmation {
                    title: if undo { "Undo this sync?" } else { "Sync selected differences?" }
                        .into(),
                    message,
                    confirm_label: if undo { "Undo sync" } else { "Sync selected" }.into(),
                    destructive: true,
                }),
            ),
            window,
            cx,
            move |_, cx| {
                let app = state.read(cx);
                if !work.matches(app, id)
                    || identities.iter().any(|(id, snapshot)| {
                        app.connection_by_id(*id).is_none_or(|c| !snapshot.matches(c))
                    })
                {
                    report(&state, id, "The comparison, selection, or connection changed. Review the operation again.".into(), cx);
                    return;
                }
                if let Some(reason) = app.compare_sync_disabled_reason(id, work.undo()) {
                    report(&state, id, reason, cx);
                    return;
                }
                Self::apply_compare_sync(state.clone(), id, work, cx);
            },
        );
    }

    fn apply_compare_sync(state: Entity<AppState>, id: Uuid, work: Work, cx: &mut App) {
        let config = work.config().clone();
        let index = if work.target() == Side::Left { 0 } else { 1 };
        let endpoint = &config.sides[index];
        let key = SessionKey::new(
            endpoint.connection_id.unwrap(),
            &endpoint.database,
            &endpoint.collection,
        );
        // Identity and plan checks precede this gate; views are refused before a production grant is spent.
        if !Self::ensure_collection_writable(&state, &key, cx) {
            return;
        }
        let app = state.read(cx);
        let Some(target_client) = app.active_connection_client(key.connection_id) else {
            return;
        };
        let clients = config.sides.each_ref().map(|endpoint| {
            endpoint
                .connection_id
                .and_then(|id| app.active_connection_client(id))
                .unwrap_or_else(|| target_client.clone())
        });
        let runtime = app.connection_manager().runtime_handle();
        let directory = app.compare_restore_dir();
        let run = work.run();
        let undo = work.undo();
        let cancellation = CancellationToken::new();
        state.update(cx, |app, cx| {
            let tab = app.compare_tab_mut(id).unwrap();
            tab.sync.running = true;
            tab.sync.completed = true;
            tab.sync.undoing = undo;
            tab.sync.error = None;
            tab.sync.summary = SyncSummary::default();
            tab.sync.cancellation = Some(cancellation.clone());
            tab.detail_cache.clear();
            tab.detail_generation = tab.detail_generation.wrapping_add(1);
            cx.notify();
        });
        let (sender, mut receiver) = futures::channel::mpsc::unbounded::<SyncProgress>();
        let (restore_sender, restore_receiver) = futures::channel::oneshot::channel();
        let task = runtime.spawn(async move {
            let sides = [0, 1].map(|i| {
                clients[i]
                    .database(&config.sides[i].database)
                    .collection(&config.sides[i].collection)
            });
            match work {
                Work::Sync(plan) => {
                    let restore = Arc::new(
                        tokio::task::spawn_blocking(move || RestoreHandle::create(&directory))
                            .await
                            .map_err(|e| Error::Parse(e.to_string()))??,
                    );
                    let _ = restore_sender.send(restore.clone());
                    sync_collections_async(
                        sides,
                        plan.target,
                        config.fields,
                        plan.items,
                        restore,
                        cancellation,
                        sender,
                    )
                    .await
                }
                Work::Undo { restore, .. } => {
                    let _ = restore_sender.send(restore.clone());
                    undo_sync_async(sides[index].clone(), restore, cancellation, sender).await
                }
            }
        });
        cx.spawn(async move |cx| {
            if let Ok(restore) = restore_receiver.await {
                cx.update(|cx| {
                    state.update(cx, |app, cx| {
                        if let Some(tab) = app.compare_tab_mut(id).filter(|t| t.run == run) {
                            tab.sync.restore = Some(restore);
                        }
                        cx.notify();
                    })
                });
            }
            while let Some(progress) = receiver.next().await {
                cx.update(|cx| {
                    state.update(cx, |app, cx| {
                        if let Some(tab) = app.compare_tab_mut(id).filter(|t| t.run == run) {
                            tab.sync.summary = progress.summary;
                            tab.sync.outcomes.extend(progress.outcomes);
                        }
                        cx.notify();
                    })
                });
            }
            let result: Result<SyncSummary> =
                task.await.map_err(|e| Error::Parse(format!("Sync stopped: {e}"))).and_then(|r| r);
            cx.update(|cx| {
                let selected = state.update(cx, |app, cx| {
                    let tab = app.compare_tab_mut(id).filter(|t| t.run == run);
                    let selected = tab.as_ref().and_then(|t| t.selected);
                    if let Some(tab) = tab {
                        tab.sync.running = false;
                        tab.sync.cancellation = None;
                        match result {
                            Ok(summary) => tab.sync.summary = summary,
                            Err(error) => tab.sync.error = Some(error.to_string()),
                        }
                    }
                    cx.emit(AppEvent::CompareChanged { compare_id: id });
                    cx.notify();
                    selected
                });
                // The documents just changed under the open detail; fetch them again.
                if let Some(row) = selected {
                    Self::select_compare_row(state.clone(), id, row, cx);
                }
            });
        })
        .detach();
    }

    pub fn cancel_compare_sync(state: &Entity<AppState>, id: Uuid, cx: &App) {
        if let Some(token) =
            state.read(cx).compare_tab(id).and_then(|t| t.sync.cancellation.as_ref())
        {
            token.cancel();
        }
    }
}
