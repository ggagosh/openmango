use std::fs;
use std::sync::{Arc, Mutex, mpsc};

use mongodb::bson::{Bson, Document, doc};
use tempfile::TempDir;
use uuid::Uuid;

use super::backend::{BackendError, InMemoryMutationBackend, MutationBackend};
use super::engine::{OperationEngine, OperationError};
use super::model::{DocumentTarget, Mutation, OperationContext, OperationQuery, OperationStatus};

fn target(id: i32) -> DocumentTarget {
    DocumentTarget {
        connection_id: Uuid::from_u128(1),
        connection_name: "Local".into(),
        database: "app".into(),
        collection: "users".into(),
        id: Bson::Int32(id),
    }
}

fn document(id: i32, value: &str) -> Document {
    doc! { "_id": id, "value": value }
}

fn engine_with(
    directory: &TempDir,
    key: [u8; 32],
    backend: Arc<dyn MutationBackend>,
) -> Arc<OperationEngine> {
    Arc::new(OperationEngine::open(directory.path().join("history.sqlite3"), key, backend).unwrap())
}

#[test]
fn prepared_operation_persists_before_backend_mutation() {
    struct BlockingBackend {
        inner: Arc<InMemoryMutationBackend>,
        entered: mpsc::SyncSender<()>,
        resume: Mutex<mpsc::Receiver<()>>,
    }
    impl MutationBackend for BlockingBackend {
        fn current_document(
            &self,
            target: &DocumentTarget,
        ) -> Result<Option<Document>, BackendError> {
            self.inner.current_document(target)
        }

        fn replace_document_if_current(
            &self,
            target: &DocumentTarget,
            expected: &Document,
            replacement: &Document,
        ) -> Result<(), BackendError> {
            self.entered.send(()).unwrap();
            self.resume.lock().unwrap().recv().unwrap();
            self.inner.replace_document_if_current(target, expected, replacement)
        }
    }

    let directory = TempDir::new().unwrap();
    let memory = Arc::new(InMemoryMutationBackend::default());
    let target = target(1);
    memory.set_document(&target, document(1, "before"));
    let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
    let (resume_sender, resume_receiver) = mpsc::sync_channel(1);
    let engine = engine_with(
        &directory,
        [1; 32],
        Arc::new(BlockingBackend {
            inner: memory,
            entered: entered_sender,
            resume: Mutex::new(resume_receiver),
        }),
    );
    let worker_engine = engine.clone();
    let handle = std::thread::spawn(move || {
        worker_engine.execute(
            OperationContext::user(),
            Mutation::ReplaceDocument {
                target,
                replacement: document(1, "after"),
                editor_precondition: None,
            },
        )
    });

    entered_receiver.recv().unwrap();
    let page = engine.list(OperationQuery::default()).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].status, OperationStatus::Running);
    let details = engine.get(page.items[0].id).unwrap().unwrap();
    assert_eq!(details.events[0].event_type, "prepared");
    resume_sender.send(()).unwrap();
    assert!(handle.join().unwrap().is_ok());
}

#[test]
fn successful_replacement_becomes_completed() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(1);
    backend.set_document(&target, document(1, "before"));
    let engine = engine_with(&directory, [2; 32], backend.clone());

    let id = engine
        .execute(
            OperationContext::user(),
            Mutation::ReplaceDocument {
                target: target.clone(),
                replacement: document(1, "after"),
                editor_precondition: None,
            },
        )
        .unwrap();

    assert_eq!(engine.get(id).unwrap().unwrap().summary.status, OperationStatus::Completed);
    assert_eq!(backend.document(&target), Some(document(1, "after")));
}

#[test]
fn successful_insert_can_be_reverted_while_document_is_unchanged() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(10);
    let inserted = document(10, "inserted");
    let engine = engine_with(&directory, [17; 32], backend.clone());

    let inserted_operation = engine
        .execute(
            OperationContext::user(),
            Mutation::InsertDocument { target: target.clone(), document: inserted.clone() },
        )
        .unwrap();

    assert_eq!(backend.document(&target), Some(inserted));
    assert_eq!(
        engine.get(inserted_operation).unwrap().unwrap().summary.kind,
        super::model::OperationKind::InsertDocument
    );

    engine.revert(OperationContext::user(), inserted_operation).unwrap();

    assert_eq!(backend.document(&target), None);
}

#[test]
fn changed_insert_blocks_revert() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(13);
    let engine = engine_with(&directory, [20; 32], backend.clone());
    let inserted = engine
        .execute(
            OperationContext::user(),
            Mutation::InsertDocument { target: target.clone(), document: document(13, "inserted") },
        )
        .unwrap();
    backend.set_document(&target, document(13, "changed"));

    let error = engine.revert(OperationContext::user(), inserted).unwrap_err();

    assert!(matches!(error, OperationError::Conflict { .. }));
    assert_eq!(backend.document(&target), Some(document(13, "changed")));
}

#[test]
fn insert_fails_before_prepare_when_id_already_exists() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(11);
    backend.set_document(&target, document(11, "existing"));
    let engine = engine_with(&directory, [18; 32], backend.clone());

    let error = engine
        .execute(
            OperationContext::user(),
            Mutation::InsertDocument { target: target.clone(), document: document(11, "new") },
        )
        .unwrap_err();

    assert_eq!(error, OperationError::TargetExists);
    assert_eq!(backend.document(&target), Some(document(11, "existing")));
    assert!(engine.list(OperationQuery::default()).unwrap().items.is_empty());
}

#[test]
fn concurrent_insert_blocks_tracked_insert() {
    struct RacingInsertBackend {
        inner: Arc<InMemoryMutationBackend>,
        concurrent: Document,
    }
    impl MutationBackend for RacingInsertBackend {
        fn current_document(
            &self,
            target: &DocumentTarget,
        ) -> Result<Option<Document>, BackendError> {
            self.inner.current_document(target)
        }

        fn replace_document_if_current(
            &self,
            target: &DocumentTarget,
            expected: &Document,
            replacement: &Document,
        ) -> Result<(), BackendError> {
            self.inner.replace_document_if_current(target, expected, replacement)
        }

        fn insert_document_if_absent(
            &self,
            target: &DocumentTarget,
            document: &Document,
        ) -> Result<(), BackendError> {
            self.inner.set_document(target, self.concurrent.clone());
            self.inner.insert_document_if_absent(target, document)
        }
    }

    let directory = TempDir::new().unwrap();
    let memory = Arc::new(InMemoryMutationBackend::default());
    let target = target(12);
    let concurrent = document(12, "concurrent");
    let engine = engine_with(
        &directory,
        [19; 32],
        Arc::new(RacingInsertBackend { inner: memory.clone(), concurrent: concurrent.clone() }),
    );

    let error = engine
        .execute(
            OperationContext::user(),
            Mutation::InsertDocument { target: target.clone(), document: document(12, "new") },
        )
        .unwrap_err();

    assert!(matches!(error, OperationError::Conflict { .. }));
    assert_eq!(memory.document(&target), Some(concurrent));
}

#[test]
fn successful_delete_can_be_restored_while_id_remains_absent() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(7);
    let before = doc! { "_id": 7, "name": "recover me" };
    backend.set_document(&target, before.clone());
    let engine = engine_with(&directory, [13; 32], backend.clone());

    let deleted = engine
        .execute(
            OperationContext::user(),
            Mutation::DeleteDocument {
                target: target.clone(),
                editor_precondition: Some(before.clone()),
            },
        )
        .unwrap();

    assert_eq!(backend.document(&target), None);
    let deleted_summary = engine.get(deleted).unwrap().unwrap().summary;
    assert_eq!(deleted_summary.kind, super::model::OperationKind::DeleteDocument);
    assert_eq!(deleted_summary.preview.unwrap().changes[0].field, "name");

    let restored = engine.revert(OperationContext::user(), deleted).unwrap();

    assert_eq!(backend.document(&target), Some(before));
    assert_eq!(engine.get(restored).unwrap().unwrap().summary.status, OperationStatus::Completed);
    assert!(!engine.get(deleted).unwrap().unwrap().summary.can_revert());
}

#[test]
fn concurrent_change_blocks_delete() {
    struct RacingDeleteBackend {
        inner: Arc<InMemoryMutationBackend>,
        concurrent: Document,
    }
    impl MutationBackend for RacingDeleteBackend {
        fn current_document(
            &self,
            target: &DocumentTarget,
        ) -> Result<Option<Document>, BackendError> {
            self.inner.current_document(target)
        }

        fn replace_document_if_current(
            &self,
            target: &DocumentTarget,
            expected: &Document,
            replacement: &Document,
        ) -> Result<(), BackendError> {
            self.inner.replace_document_if_current(target, expected, replacement)
        }

        fn delete_document_if_current(
            &self,
            target: &DocumentTarget,
            expected: &Document,
        ) -> Result<(), BackendError> {
            self.inner.set_document(target, self.concurrent.clone());
            self.inner.delete_document_if_current(target, expected)
        }
    }

    let directory = TempDir::new().unwrap();
    let memory = Arc::new(InMemoryMutationBackend::default());
    let target = target(8);
    let before = document(8, "before");
    let concurrent = document(8, "concurrent");
    memory.set_document(&target, before.clone());
    let engine = engine_with(
        &directory,
        [14; 32],
        Arc::new(RacingDeleteBackend { inner: memory.clone(), concurrent: concurrent.clone() }),
    );

    let error = engine
        .execute(
            OperationContext::user(),
            Mutation::DeleteDocument { target: target.clone(), editor_precondition: Some(before) },
        )
        .unwrap_err();

    assert!(matches!(error, OperationError::Conflict { .. }));
    assert_eq!(memory.document(&target), Some(concurrent));
}

#[test]
fn restore_delete_conflicts_when_id_was_reused() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(9);
    backend.set_document(&target, document(9, "before"));
    let engine = engine_with(&directory, [15; 32], backend.clone());
    let deleted = engine
        .execute(
            OperationContext::user(),
            Mutation::DeleteDocument { target: target.clone(), editor_precondition: None },
        )
        .unwrap();
    backend.set_document(&target, document(9, "reused"));

    let error = engine.revert(OperationContext::user(), deleted).unwrap_err();

    assert!(matches!(error, OperationError::Conflict { .. }));
    assert_eq!(backend.document(&target), Some(document(9, "reused")));
}

#[test]
fn operation_listing_previews_the_document_and_changed_fields() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(42);
    backend.set_document(&target, doc! { "_id": 42, "name": "before" });
    let engine = engine_with(&directory, [12; 32], backend);
    engine
        .execute(
            OperationContext::user(),
            Mutation::ReplaceDocument {
                target,
                replacement: doc! { "_id": 42, "name": "after", "active": true },
                editor_precondition: None,
            },
        )
        .unwrap();

    let operation = engine.list(OperationQuery::default()).unwrap().items.remove(0);
    let preview = operation.preview.unwrap();

    assert_eq!(preview.document_id, "42");
    assert_eq!(preview.total_changes, 2);
    assert_eq!(preview.changes[0].field, "name");
    assert_eq!(preview.changes[0].before.as_deref(), Some("before"));
    assert_eq!(preview.changes[0].after.as_deref(), Some("after"));
    assert_eq!(preview.changes[1].field, "active");
    assert_eq!(preview.changes[1].before, None);
    assert_eq!(preview.changes[1].after.as_deref(), Some("true"));
}

#[test]
fn encrypted_store_never_contains_plaintext_document_values() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(1);
    let before_secret = "plaintext-before-4df18";
    let after_secret = "plaintext-after-8ca27";
    backend.set_document(&target, document(1, before_secret));
    let engine = engine_with(&directory, [3; 32], backend);
    engine
        .execute(
            OperationContext::user(),
            Mutation::ReplaceDocument {
                target,
                replacement: document(1, after_secret),
                editor_precondition: None,
            },
        )
        .unwrap();
    let path = engine.store_path().to_path_buf();

    let mut bytes = fs::read(&path).unwrap();
    for suffix in ["-wal", "-shm"] {
        let candidate = format!("{}{suffix}", path.display());
        if let Ok(extra) = fs::read(candidate) {
            bytes.extend(extra);
        }
    }
    let raw = String::from_utf8_lossy(&bytes);
    assert!(!raw.contains(before_secret));
    assert!(!raw.contains(after_secret));
}

#[test]
fn concurrent_change_blocks_original_update() {
    struct RacingBackend {
        inner: Arc<InMemoryMutationBackend>,
        concurrent: Document,
    }
    impl MutationBackend for RacingBackend {
        fn current_document(
            &self,
            target: &DocumentTarget,
        ) -> Result<Option<Document>, BackendError> {
            self.inner.current_document(target)
        }

        fn replace_document_if_current(
            &self,
            target: &DocumentTarget,
            expected: &Document,
            replacement: &Document,
        ) -> Result<(), BackendError> {
            self.inner.set_document(target, self.concurrent.clone());
            self.inner.replace_document_if_current(target, expected, replacement)
        }
    }

    let directory = TempDir::new().unwrap();
    let memory = Arc::new(InMemoryMutationBackend::default());
    let target = target(1);
    memory.set_document(&target, document(1, "before"));
    let engine = engine_with(
        &directory,
        [4; 32],
        Arc::new(RacingBackend { inner: memory.clone(), concurrent: document(1, "concurrent") }),
    );

    let error = engine
        .execute(
            OperationContext::user(),
            Mutation::ReplaceDocument {
                target: target.clone(),
                replacement: document(1, "after"),
                editor_precondition: None,
            },
        )
        .unwrap_err();

    assert!(matches!(error, OperationError::Conflict { .. }));
    assert_eq!(memory.document(&target), Some(document(1, "concurrent")));
}

#[test]
fn concurrent_change_after_update_blocks_revert() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(1);
    backend.set_document(&target, document(1, "before"));
    let engine = engine_with(&directory, [5; 32], backend.clone());
    let original = engine
        .execute(
            OperationContext::user(),
            Mutation::ReplaceDocument {
                target: target.clone(),
                replacement: document(1, "after"),
                editor_precondition: None,
            },
        )
        .unwrap();
    backend.set_document(&target, document(1, "concurrent"));

    let error = engine.revert(OperationContext::user(), original).unwrap_err();

    assert!(matches!(error, OperationError::Conflict { .. }));
    assert_eq!(backend.document(&target), Some(document(1, "concurrent")));
    assert_eq!(engine.get(original).unwrap().unwrap().summary.status, OperationStatus::Completed);
}

#[test]
fn successful_revert_restores_before_image_and_creates_linked_operation() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(1);
    backend.set_document(&target, document(1, "before"));
    let engine = engine_with(&directory, [6; 32], backend.clone());
    let original = engine
        .execute(
            OperationContext::user(),
            Mutation::ReplaceDocument {
                target: target.clone(),
                replacement: document(1, "after"),
                editor_precondition: None,
            },
        )
        .unwrap();

    let reverted = engine.revert(OperationContext::user(), original).unwrap();
    let reverted = engine.get(reverted).unwrap().unwrap().summary;

    assert_eq!(backend.document(&target), Some(document(1, "before")));
    assert_eq!(reverted.status, OperationStatus::Completed);
    assert_eq!(reverted.parent_operation_id, Some(original));
    assert_eq!(reverted.reverts_operation_id, Some(original));
    let original = engine.get(original).unwrap().unwrap().summary;
    assert_eq!(original.status, OperationStatus::Completed);
    assert!(!original.can_revert());
}

#[test]
fn stale_editor_precondition_blocks_tracked_save_before_prepare() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(1);
    backend.set_document(&target, document(1, "server-newer"));
    let engine = engine_with(&directory, [10; 32], backend.clone());

    let error = engine
        .execute(
            OperationContext::user(),
            Mutation::ReplaceDocument {
                target: target.clone(),
                replacement: document(1, "after"),
                editor_precondition: Some(document(1, "editor-baseline")),
            },
        )
        .unwrap_err();

    assert_eq!(error, OperationError::PreconditionConflict);
    assert_eq!(backend.document(&target), Some(document(1, "server-newer")));
    assert!(engine.list(OperationQuery::default()).unwrap().items.is_empty());
}

#[test]
fn stale_editor_precondition_blocks_tracked_delete_before_prepare() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(1);
    backend.set_document(&target, document(1, "server-newer"));
    let engine = engine_with(&directory, [16; 32], backend.clone());

    let error = engine
        .execute(
            OperationContext::user(),
            Mutation::DeleteDocument {
                target: target.clone(),
                editor_precondition: Some(document(1, "editor-baseline")),
            },
        )
        .unwrap_err();

    assert_eq!(error, OperationError::PreconditionConflict);
    assert_eq!(backend.document(&target), Some(document(1, "server-newer")));
    assert!(engine.list(OperationQuery::default()).unwrap().items.is_empty());
}

#[test]
fn reconciliation_recognizes_unapplied_applied_conflicting_and_missing_states() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let engine = engine_with(&directory, [7; 32], backend.clone());
    let unapplied_target = target(1);
    let applied_target = target(2);
    let ambiguous_target = target(3);
    let missing_target = target(4);
    let deleted_target = target(5);
    let unapplied_insert_target = target(6);
    let applied_insert_target = target(7);
    let conflicting_insert_target = target(8);
    let unapplied = engine.prepare_for_test(
        unapplied_target.clone(),
        document(1, "before"),
        document(1, "after"),
    );
    let applied = engine.prepare_for_test(
        applied_target.clone(),
        document(2, "before"),
        document(2, "after"),
    );
    let ambiguous = engine.prepare_for_test(
        ambiguous_target.clone(),
        document(3, "before"),
        document(3, "after"),
    );
    let missing =
        engine.prepare_for_test(missing_target, document(4, "before"), document(4, "after"));
    let deleted = engine.prepare_delete_for_test(deleted_target, document(5, "before"));
    let unapplied_insert =
        engine.prepare_insert_for_test(unapplied_insert_target.clone(), document(6, "inserted"));
    let applied_insert =
        engine.prepare_insert_for_test(applied_insert_target.clone(), document(7, "inserted"));
    let conflicting_insert =
        engine.prepare_insert_for_test(conflicting_insert_target.clone(), document(8, "inserted"));
    backend.set_document(&unapplied_target, document(1, "before"));
    backend.set_document(&applied_target, document(2, "after"));
    backend.set_document(&ambiguous_target, document(3, "other"));
    backend.set_document(&applied_insert_target, document(7, "inserted"));
    backend.set_document(&conflicting_insert_target, document(8, "other"));

    let report = engine.reconcile().unwrap();

    assert_eq!(report.not_applied, 2);
    assert_eq!(report.completed, 3);
    assert_eq!(report.conflicted, 3);
    assert_eq!(engine.get(unapplied).unwrap().unwrap().summary.status, OperationStatus::Failed);
    assert_eq!(engine.get(applied).unwrap().unwrap().summary.status, OperationStatus::Completed);
    assert_eq!(engine.get(ambiguous).unwrap().unwrap().summary.status, OperationStatus::Conflict);
    assert_eq!(engine.get(missing).unwrap().unwrap().summary.status, OperationStatus::Conflict);
    assert_eq!(engine.get(deleted).unwrap().unwrap().summary.status, OperationStatus::Completed);
    assert_eq!(
        engine.get(unapplied_insert).unwrap().unwrap().summary.status,
        OperationStatus::Failed
    );
    assert_eq!(
        engine.get(applied_insert).unwrap().unwrap().summary.status,
        OperationStatus::Completed
    );
    assert_eq!(
        engine.get(conflicting_insert).unwrap().unwrap().summary.status,
        OperationStatus::Conflict
    );
}

#[test]
fn history_disabled_selects_the_existing_untracked_path() {
    assert!(super::tracked_engine(false, None).unwrap().is_none());
    assert!(matches!(super::tracked_engine(true, None), Err(OperationError::Unavailable)));
}

#[test]
fn store_migration_and_reopen_round_trip() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(1);
    backend.set_document(&target, document(1, "before"));
    let engine = engine_with(&directory, [8; 32], backend.clone());
    let id = engine
        .execute(
            OperationContext::user(),
            Mutation::ReplaceDocument {
                target,
                replacement: document(1, "after"),
                editor_precondition: None,
            },
        )
        .unwrap();
    drop(engine);

    let reopened = engine_with(&directory, [8; 32], backend);
    let details = reopened.get(id).unwrap().unwrap();
    assert_eq!(details.summary.status, OperationStatus::Completed);
    assert!(details.events.iter().any(|event| event.event_type == "prepared"));
}

#[test]
fn authenticated_target_rejects_tampered_query_metadata() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(1);
    backend.set_document(&target, document(1, "before"));
    let engine = engine_with(&directory, [12; 32], backend);
    let id = engine
        .execute(
            OperationContext::user(),
            Mutation::ReplaceDocument {
                target,
                replacement: document(1, "after"),
                editor_precondition: None,
            },
        )
        .unwrap();
    let connection = rusqlite::Connection::open(engine.store_path()).unwrap();
    connection
        .execute(
            "UPDATE operations SET connection_id = ?2 WHERE id = ?1",
            rusqlite::params![id.to_string(), Uuid::new_v4().to_string()],
        )
        .unwrap();

    assert!(matches!(engine.list(OperationQuery::default()), Err(OperationError::Internal)));
}

#[cfg(unix)]
#[test]
fn sqlite_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(1);
    backend.set_document(&target, document(1, "before"));
    let engine = engine_with(&directory, [11; 32], backend);
    engine
        .execute(
            OperationContext::user(),
            Mutation::ReplaceDocument {
                target,
                replacement: document(1, "after"),
                editor_precondition: None,
            },
        )
        .unwrap();

    let path = engine.store_path();
    for candidate in [
        path.to_path_buf(),
        std::path::PathBuf::from(format!("{}-wal", path.display())),
        std::path::PathBuf::from(format!("{}-shm", path.display())),
    ] {
        if candidate.exists() {
            assert_eq!(fs::metadata(candidate).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }
}

#[test]
fn operation_listing_filters_by_collection() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let users = target(1);
    let mut orders = target(2);
    orders.collection = "orders".into();
    backend.set_document(&users, document(1, "before"));
    backend.set_document(&orders, document(2, "before"));
    let engine = engine_with(&directory, [13; 32], backend);
    for (target, id) in [(users.clone(), 1), (orders.clone(), 2)] {
        engine
            .execute(
                OperationContext::user(),
                Mutation::ReplaceDocument {
                    target,
                    replacement: document(id, "after"),
                    editor_precondition: None,
                },
            )
            .unwrap();
    }

    let page = engine
        .list(OperationQuery::for_collection(
            users.connection_id,
            &users.database,
            &users.collection,
        ))
        .unwrap();

    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].collection, "users");
}

#[test]
fn operation_listing_is_paginated_and_newest_first() {
    let directory = TempDir::new().unwrap();
    let backend = Arc::new(InMemoryMutationBackend::default());
    let target = target(1);
    backend.set_document(&target, document(1, "zero"));
    let engine = engine_with(&directory, [9; 32], backend);
    let mut ids = Vec::new();
    for value in ["one", "two", "three"] {
        ids.push(
            engine
                .execute(
                    OperationContext::user(),
                    Mutation::ReplaceDocument {
                        target: target.clone(),
                        replacement: document(1, value),
                        editor_precondition: None,
                    },
                )
                .unwrap(),
        );
    }

    let first =
        engine.list(OperationQuery { offset: 0, limit: 2, ..OperationQuery::default() }).unwrap();
    let second =
        engine.list(OperationQuery { offset: 2, limit: 2, ..OperationQuery::default() }).unwrap();

    assert_eq!(first.items.iter().map(|item| item.id).collect::<Vec<_>>(), vec![ids[2], ids[1]]);
    assert_eq!(first.next_offset, Some(2));
    assert_eq!(second.items.iter().map(|item| item.id).collect::<Vec<_>>(), vec![ids[0]]);
    assert_eq!(second.next_offset, None);
    assert_eq!(first.total, 3);
}
