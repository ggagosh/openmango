use std::sync::Arc;

use futures::StreamExt;
use gpui_kit::{App, AppContext as _, Entity, Window};
use uuid::Uuid;

use super::AppCommands;
use crate::components::{WriteConfirmation, WriteRequest, request_connection_write};
use crate::connection::ops::compare::MAX_ROWS;
use crate::connection::ops::compare_database::{
    DatabaseSync, PairSyncMessage, SyncMode, sync_pairs_async, undo_pairs_async,
};
use crate::connection::ops::compare_sync::{
    Operation, SyncProgress, SyncSummary, restore::RestoreHandle, sync_collections_async,
    undo_sync_async,
};
use crate::connection::{CancellationToken, ops::compare::Side};
use crate::error::{Error, Result};
use crate::helpers::format_number;
use crate::models::ConnectionWriteIdentity;
use crate::state::compare::CompareConfig;
use crate::state::compare_sync::{DatabaseSyncPlan, SyncPlan};
use crate::state::{AppEvent, AppState, SessionKey};

enum Work {
    Sync(SyncPlan),
    /// One path of the selected difference, guarded by the documents on screen.
    Field(SyncPlan),
    Undo {
        run: u64,
        config: CompareConfig,
        target: Side,
        restore: Arc<RestoreHandle>,
    },
}

impl Work {
    fn config(&self) -> &CompareConfig {
        match self {
            Self::Sync(p) | Self::Field(p) => &p.config,
            Self::Undo { config, .. } => config,
        }
    }
    fn target(&self) -> Side {
        match self {
            Self::Sync(p) | Self::Field(p) => p.target,
            Self::Undo { target, .. } => *target,
        }
    }
    fn run(&self) -> u64 {
        match self {
            Self::Sync(p) | Self::Field(p) => p.run,
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
            Self::Field(plan) => {
                let item = &plan.items[0];
                plan.run == tab.run
                    && tab.selected == Some(item.row_index)
                    && tab.detail.as_ref().is_some_and(|detail| {
                        detail.hashes == [item.row.left_hash, item.row.right_hash]
                    })
                    && app
                        .compare_field_copy_disabled_reason(
                            id,
                            item.field.as_deref().unwrap_or_default(),
                            plan.target,
                        )
                        .is_none()
            }
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

enum DatabaseWork {
    Sync(DatabaseSyncPlan),
    Undo { run: u64, config: CompareConfig, logs: Vec<(usize, String, Arc<RestoreHandle>)> },
}

impl DatabaseWork {
    fn config(&self) -> &CompareConfig {
        match self {
            Self::Sync(plan) => &plan.config,
            Self::Undo { config, .. } => config,
        }
    }
    fn run(&self) -> u64 {
        match self {
            Self::Sync(plan) => plan.run,
            Self::Undo { run, .. } => *run,
        }
    }
    fn matches(&self, app: &AppState, id: Uuid) -> bool {
        let Some(tab) = app.compare_tab(id) else {
            return false;
        };
        match self {
            Self::Sync(plan) => plan.matches(tab),
            Self::Undo { run, config, logs } => {
                *run == tab.run
                    && config == tab.results_config()
                    && !tab.running
                    && !tab.sync.running
                    && logs.iter().all(|(_, _, log)| {
                        tab.sync.logs.iter().any(|(_, _, current)| Arc::ptr_eq(current, log))
                    })
            }
        }
    }
}

fn collection_count(count: usize) -> String {
    format!("{} collection{}", format_number(count as u64), if count == 1 { "" } else { "s" })
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
            Work::Sync(plan) | Work::Field(plan) => {
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
                    title: match (undo, tab.sync.field_copies) {
                        (true, true) => "Undo these field copies?",
                        (true, false) => "Undo this sync?",
                        _ => "Sync selected differences?",
                    }
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

    /// Copy one field of the selected difference into `target`. It writes at once, like an edit;
    /// production connections still confirm, and Undo covers every copy since the comparison.
    pub fn copy_compare_field(
        state: Entity<AppState>,
        id: Uuid,
        path: Vec<crate::bson::PathSegment>,
        target: Side,
        window: &mut Window,
        cx: &mut App,
    ) {
        let app = state.read(cx);
        if let Some(reason) = app.compare_field_copy_disabled_reason(id, &path, target) {
            report(&state, id, reason, cx);
            return;
        }
        let tab = app.compare_tab(id).unwrap();
        let (Some(row_index), Some(detail)) = (tab.selected, tab.detail.as_ref()) else {
            return;
        };
        let mut row = tab.rows[row_index].clone();
        [row.left_hash, row.right_hash] = detail.hashes;
        (row.left_count, row.right_count) = (1, 1);
        let config = tab.results_config().clone();
        let index = if target == Side::Left { 0 } else { 1 };
        let endpoint = &config.sides[index];
        let Some(connection_id) = endpoint.connection_id else {
            return;
        };
        let name =
            app.connection_by_id(connection_id).map(|c| c.name.as_str()).unwrap_or("Connection");
        let target_label = format!("{name} · {}", endpoint.namespace());
        let work = Work::Field(SyncPlan {
            run: tab.run,
            revision: tab.sync.revision,
            config: config.clone(),
            target,
            items: vec![crate::connection::ops::compare_sync::SyncItem {
                row_index,
                row,
                operation: Operation::Replace,
                field: Some(path),
            }],
        });
        request_connection_write(
            state.clone(),
            WriteRequest::new(connection_id, target_label, "Copy field", None),
            window,
            cx,
            move |_, cx| {
                if !work.matches(state.read(cx), id) {
                    report(&state, id, "The documents changed. Copy the field again.".into(), cx);
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
        let field = matches!(work, Work::Field(_));
        // Field copies after a finished sync or copy add to its undo log and its totals.
        let (existing, base) = app
            .compare_tab(id)
            .filter(|tab| field && tab.sync.completed && !tab.sync.undoing)
            .and_then(|tab| Some((tab.sync.restore.clone()?, tab.sync.summary.clone())))
            .map_or((None, SyncSummary::default()), |(restore, base)| (Some(restore), base));
        let cancellation = CancellationToken::new();
        state.update(cx, |app, cx| {
            let tab = app.compare_tab_mut(id).unwrap();
            tab.sync.field_copies = field && (existing.is_none() || tab.sync.field_copies);
            tab.sync.target = Some(work.target());
            tab.sync.running = true;
            tab.sync.completed = true;
            tab.sync.undoing = undo;
            tab.sync.error = None;
            tab.sync.summary = base.clone();
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
                Work::Sync(plan) | Work::Field(plan) => {
                    let restore = match existing {
                        Some(restore) => restore,
                        None => Arc::new(
                            tokio::task::spawn_blocking(move || RestoreHandle::create(&directory))
                                .await
                                .map_err(|e| Error::Parse(e.to_string()))??,
                        ),
                    };
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
        let with_base = move |summary: SyncSummary| {
            let mut total = base.clone();
            total.absorb(&summary);
            total
        };
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
                            tab.sync.summary = with_base(progress.summary);
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
                            Ok(summary) => tab.sync.summary = with_base(summary),
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

    /// Database scope: confirm, then sync the ticked collections or undo the last sync.
    pub fn review_database_sync(
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
            let logs: Vec<_> =
                tab.sync.logs.iter().filter(|(_, _, log)| log.pending() > 0).cloned().collect();
            if logs.is_empty() {
                return;
            }
            DatabaseWork::Undo { run: tab.run, config: tab.results_config().clone(), logs }
        } else {
            let Some(plan) = DatabaseSyncPlan::from_tab(tab) else {
                return;
            };
            DatabaseWork::Sync(plan)
        };
        let Some(target) = tab.sync.target else {
            return;
        };
        let index = if target == Side::Left { 0 } else { 1 };
        let config = work.config().clone();
        let Some(connection_id) = config.sides[index].connection_id else {
            return;
        };
        let identities: Vec<_> = config
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
        let target_label = format!("{name} · {}", config.sides[index].database);
        let (collections, message) = match &work {
            DatabaseWork::Sync(plan) => {
                let [insert, replace, delete] = plan.writes;
                let created = plan.pairs.iter().filter(|pair| pair.create).count();
                let mut counts = vec![format!(
                    "insert {}{}",
                    if plan.estimated { "~" } else { "" },
                    format_number(insert)
                )];
                if plan.mode != SyncMode::AddMissing {
                    counts.push(format!("replace {}", format_number(replace)));
                }
                if plan.mode == SyncMode::Mirror {
                    counts.push(format!("delete {}", format_number(delete)));
                }
                let collections = plan.pairs.len();
                let mut message = format!(
                    "{} {}: {} in {}.",
                    plan.mode.label(),
                    collection_count(collections),
                    counts.join(", "),
                    target_label
                );
                if created > 0 {
                    message.push_str(&format!(
                        " {} created with the source's options and indexes.",
                        collection_count(created)
                    ));
                }
                message.push_str("\n\nDocuments changed since the comparison are skipped, and minor differences are left as they are. Undo is available until this tab closes or you compare again.");
                (collections, message)
            }
            DatabaseWork::Undo { logs, .. } => {
                let pending: usize = logs.iter().map(|(_, _, log)| log.pending()).sum();
                (
                    logs.len(),
                    format!(
                        "Undo up to {} writes across {} in {target_label}.\n\nDocuments changed since the sync are skipped. Collections the sync created stay, empty.",
                        format_number(pending as u64),
                        collection_count(logs.len())
                    ),
                )
            }
        };
        request_connection_write(
            state.clone(),
            WriteRequest::new(
                connection_id,
                target_label,
                if undo { "Undo sync" } else { "Sync databases" },
                Some(WriteConfirmation {
                    title: if undo {
                        "Undo this sync?".into()
                    } else {
                        format!("Sync {}?", collection_count(collections))
                    },
                    message,
                    confirm_label: if undo {
                        "Undo sync".into()
                    } else {
                        format!("Sync {}", collection_count(collections))
                    },
                    destructive: true,
                }),
            )
            .for_writes(collections),
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
                if let Some(reason) = app.compare_sync_disabled_reason(id, undo) {
                    report(&state, id, reason, cx);
                    return;
                }
                Self::apply_database_sync(state.clone(), id, target, work, cx);
            },
        );
    }

    fn apply_database_sync(
        state: Entity<AppState>,
        id: Uuid,
        target: Side,
        work: DatabaseWork,
        cx: &mut App,
    ) {
        let config = work.config().clone();
        let index = if target == Side::Left { 0 } else { 1 };
        let Some(connection_id) = config.sides[index].connection_id else {
            return;
        };
        let names: Vec<&str> = match &work {
            DatabaseWork::Sync(plan) => plan.pairs.iter().map(|p| p.name.as_str()).collect(),
            DatabaseWork::Undo { logs, .. } => logs.iter().map(|(_, n, _)| n.as_str()).collect(),
        };
        // One production grant per collection, spent before anything is written.
        for name in names {
            let key = SessionKey::new(connection_id, &config.sides[index].database, name);
            if !Self::ensure_collection_writable(&state, &key, cx) {
                return;
            }
        }
        let app = state.read(cx);
        let clients = config
            .sides
            .each_ref()
            .map(|side| side.connection_id.and_then(|id| app.active_connection_client(id)));
        let [Some(left), Some(right)] = clients else {
            report(&state, id, "Reconnect both connections to sync".into(), cx);
            return;
        };
        let clients = [left, right];
        let runtime = app.connection_manager().runtime_handle();
        let directory = app.compare_restore_dir();
        let ignore = config.ignore_set();
        let run = work.run();
        let undo = matches!(work, DatabaseWork::Undo { .. });
        let cancellation = CancellationToken::new();
        state.update(cx, |app, cx| {
            let tab = app.compare_tab_mut(id).unwrap();
            tab.sync.running = true;
            tab.sync.completed = true;
            tab.sync.undoing = undo;
            tab.sync.error = None;
            tab.sync.pairs.clear();
            tab.sync.pair_current = None;
            tab.sync.cancellation = Some(cancellation.clone());
            cx.notify();
        });
        let (sender, mut receiver) = futures::channel::mpsc::unbounded::<PairSyncMessage>();
        let databases = config.sides.each_ref().map(|side| side.database.clone());
        let target_client = clients[index].clone();
        let target_database = databases[index].clone();
        let task = runtime.spawn(async move {
            match work {
                DatabaseWork::Sync(plan) => {
                    sync_pairs_async(
                        DatabaseSync {
                            clients,
                            databases,
                            target: plan.target,
                            mode: plan.mode,
                            pairs: plan.pairs,
                            ignore,
                            restore_dir: directory,
                            pass_rows: MAX_ROWS,
                        },
                        cancellation,
                        sender,
                    )
                    .await
                }
                DatabaseWork::Undo { logs, .. } => {
                    undo_pairs_async(target_client, target_database, logs, cancellation, sender)
                        .await;
                    Ok(())
                }
            }
        });
        cx.spawn(async move |cx| {
            while let Some(message) = receiver.next().await {
                cx.update(|cx| {
                    state.update(cx, |app, cx| {
                        if let Some(tab) = app.compare_tab_mut(id).filter(|t| t.run == run) {
                            tab.receive_pair_sync(message);
                        }
                        cx.notify();
                    })
                });
            }
            let result: Result<()> =
                task.await.map_err(|e| Error::Parse(format!("Sync stopped: {e}"))).and_then(|r| r);
            cx.update(|cx| {
                let scans = state.update(cx, |app, cx| {
                    let tab = app.compare_tab_mut(id).filter(|t| t.run == run)?;
                    tab.sync.running = false;
                    tab.sync.cancellation = None;
                    tab.sync.pair_current = None;
                    if let Err(error) = result {
                        tab.sync.error = Some(error.to_string());
                    }
                    // Show what the collections hold now.
                    let mut written: Vec<usize> = tab.sync.pairs.keys().copied().collect();
                    written.sort_unstable();
                    let scans = tab.recheck_pairs(&written);
                    cx.emit(AppEvent::CompareChanged { compare_id: id });
                    cx.notify();
                    Some(scans)
                });
                if let Some(scans) = scans {
                    Self::scan_database_pairs(state.clone(), id, run, scans, cx);
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
