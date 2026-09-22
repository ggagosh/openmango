//! Selective sync contracts use a disposable MongoDB 8 server; no user connection is touched.

use futures::{StreamExt, TryStreamExt};
use mongodb::bson::{Document, RawDocumentBuf, doc};
use mongodb::{Client, Collection, IndexModel};
use openmango::connection::CancellationToken;
use openmango::connection::ops::compare::{
    CompareMessage, CompareOptions, Side, compare_collections_async,
};
use openmango::connection::ops::compare_sync::{
    Operation, SyncItem, SyncSummary, operation_for, restore::RestoreHandle,
    sync_collections_async, undo_sync_async,
};
use std::sync::Arc;
use testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner};
use testcontainers_modules::mongo::Mongo;

async fn server(version: &str) -> (ContainerAsync<Mongo>, Client) {
    let container = Mongo::default().with_tag(version).start().await.unwrap();
    let client = Client::with_uri_str(format!(
        "mongodb://{}:{}",
        container.get_host().await.unwrap(),
        container.get_host_port_ipv4(27017).await.unwrap()
    ))
    .await
    .unwrap();
    client.database("admin").run_command(doc! {"ping":1}).await.unwrap();
    (container, client)
}

async fn plan(
    sides: &[Collection<Document>; 2],
    fields: &[&str],
    filter: Document,
    target: Side,
) -> Vec<SyncItem> {
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    let (result, messages) = tokio::join!(
        compare_collections_async(
            sides[0].clone_with_type(),
            sides[1].clone_with_type(),
            CompareOptions {
                fields: fields.iter().map(|f| (*f).into()).collect(),
                filter,
                ..Default::default()
            },
            CancellationToken::new(),
            sender
        ),
        receiver.collect::<Vec<_>>()
    );
    result.unwrap();
    messages
        .into_iter()
        .flat_map(|message| match message {
            CompareMessage::Progress { new_rows, .. } => new_rows,
            _ => vec![],
        })
        .enumerate()
        .filter_map(|(row_index, row)| {
            operation_for(row.kind, target).map(|operation| SyncItem { row_index, row, operation })
        })
        .collect()
}

async fn sync(
    sides: &[Collection<Document>; 2],
    fields: &[&str],
    target: Side,
    items: Vec<SyncItem>,
    restore: Arc<RestoreHandle>,
) -> SyncSummary {
    let (sender, _receiver) = futures::channel::mpsc::unbounded();
    sync_collections_async(
        sides.each_ref().map(Collection::clone_with_type),
        target,
        fields.iter().map(|f| (*f).into()).collect(),
        items,
        restore,
        CancellationToken::new(),
        sender,
    )
    .await
    .unwrap()
}

async fn undo(target: &Collection<Document>, restore: Arc<RestoreHandle>) -> SyncSummary {
    let (sender, _receiver) = futures::channel::mpsc::unbounded();
    undo_sync_async(target.clone_with_type(), restore, CancellationToken::new(), sender)
        .await
        .unwrap()
}

async fn raw_documents(collection: &Collection<Document>) -> Vec<Vec<u8>> {
    collection
        .clone_with_type::<RawDocumentBuf>()
        .find(doc! {})
        .sort(doc! {"_id":1})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.as_bytes().to_vec())
        .collect()
}

#[tokio::test]
async fn bulk_sync_both_directions_and_undo_preserve_bson() {
    let (_container, client) = server("8.2.3").await;
    for target in [Side::Left, Side::Right] {
        let db = client.database(if target == Side::Left { "left_target" } else { "right_target" });
        let sides = [db.collection::<Document>("left"), db.collection::<Document>("right")];
        sides[0].insert_many([
            doc! {"_id":1,"value":"left","long":i64::MAX, "nested":{"order":1,"b":2}, "date":mongodb::bson::DateTime::from_millis(123),"time":mongodb::bson::Timestamp { time: 12, increment: 3 }, "decimal": "1.234567890123456789".parse::<mongodb::bson::Decimal128>().unwrap(), "binary": mongodb::bson::Binary { subtype: mongodb::bson::spec::BinarySubtype::Generic, bytes: vec![0,1,255] }},
            doc! {"_id":2,"value":"only left"}, doc! {"_id":4,"value":1i64},
        ]).await.unwrap();
        sides[1]
            .insert_many([
                doc! {"_id":1,"value":"right"},
                doc! {"_id":3,"value":"only right"},
                doc! {"_id":4,"value":1.0},
            ])
            .await
            .unwrap();
        let index = if target == Side::Left { 0 } else { 1 };
        let original = raw_documents(&sides[index]).await;
        let items = plan(&sides, &["_id"], doc! {}, target).await;
        assert_eq!(items.len(), 4);
        let directory = tempfile::tempdir().unwrap();
        let restore = Arc::new(RestoreHandle::create(directory.path()).unwrap());
        let result = sync(&sides, &["_id"], target, items, restore.clone()).await;
        assert_eq!((result.written, result.failed, result.uncertain), (4, 0, 0));
        assert_eq!(raw_documents(&sides[index]).await, raw_documents(&sides[1 - index]).await);
        assert_eq!(restore.pending(), 4);
        assert_eq!(undo(&sides[index], restore.clone()).await.written, 4);
        assert_eq!(raw_documents(&sides[index]).await, original);
        assert_eq!(restore.pending(), 0);
    }
}

#[tokio::test]
async fn dotted_compound_key_keeps_target_id_and_undo_skips_subsequent_edits() {
    let (_container, client) = server("8.2.3").await;
    let db = client.database("custom_key");
    let sides = [db.collection::<Document>("left"), db.collection::<Document>("right")];
    sides[0]
        .insert_many([
            doc! {"_id":1,"tenant":{"id":"a"},"sku":7,"value":"new"},
            doc! {"_id":2,"tenant":{"id":"b"},"sku":8},
        ])
        .await
        .unwrap();
    sides[1].insert_one(doc! {"_id":101,"tenant":{"id":"a"},"sku":7,"value":"old"}).await.unwrap();
    let fields = ["tenant.id", "sku"];
    let items = plan(&sides, &fields, doc! {}, Side::Right).await;
    let directory = tempfile::tempdir().unwrap();
    let restore = Arc::new(RestoreHandle::create(directory.path()).unwrap());
    assert_eq!(sync(&sides, &fields, Side::Right, items, restore.clone()).await.written, 2);
    let replaced = sides[1].find_one(doc! {"_id":101}).await.unwrap().unwrap();
    assert_eq!(replaced.get_str("value").unwrap(), "new");
    sides[1].update_one(doc! {"_id":101}, doc! {"$set":{"value":"later edit"}}).await.unwrap();
    let result = undo(&sides[1], restore.clone()).await;
    assert_eq!((result.written, result.skipped), (1, 1));
    assert_eq!(restore.pending(), 1);
    assert_eq!(
        sides[1].find_one(doc! {"_id":101}).await.unwrap().unwrap().get_str("value").unwrap(),
        "later edit"
    );
    assert!(sides[1].find_one(doc! {"_id":2}).await.unwrap().is_none());
}

#[tokio::test]
async fn revalidation_protects_filtered_out_documents_stale_sides_and_new_duplicates() {
    let (_container, client) = server("8.2.3").await;
    let db = client.database("stale");
    let sides = [db.collection::<Document>("left"), db.collection::<Document>("right")];
    sides[0]
        .insert_many([
            doc! {"_id":1,"sku":1,"active":true,"v":"left"},
            doc! {"_id":2,"sku":2,"active":true,"v":"left"},
            doc! {"_id":3,"sku":3,"active":true},
            doc! {"_id":4,"sku":4,"active":false},
            doc! {"_id":5,"sku":5,"active":true},
            doc! {"_id":6,"sku":6,"active":true},
        ])
        .await
        .unwrap();
    sides[1]
        .insert_many([
            doc! {"_id":101,"sku":1,"active":true,"v":"right"},
            doc! {"_id":102,"sku":2,"active":true,"v":"right"},
            doc! {"_id":103,"sku":3,"active":false},
            doc! {"_id":104,"sku":4,"active":true},
        ])
        .await
        .unwrap();
    let items = plan(&sides, &["sku"], doc! {"active":true}, Side::Right).await;
    sides[0].update_one(doc! {"_id":1}, doc! {"$set":{"v":"changed"}}).await.unwrap();
    sides[1].update_one(doc! {"_id":102}, doc! {"$set":{"v":"changed"}}).await.unwrap();
    sides[0].insert_one(doc! {"_id":55,"sku":5,"active":true}).await.unwrap();
    sides[0].update_one(doc! {"_id":6}, doc! {"$set":{"sku":60}}).await.unwrap();
    let original = raw_documents(&sides[1]).await;
    let directory = tempfile::tempdir().unwrap();
    let restore = Arc::new(RestoreHandle::create(directory.path()).unwrap());
    let result = sync(&sides, &["sku"], Side::Right, items, restore.clone()).await;
    assert_eq!((result.written, result.skipped, result.failed), (0, 6, 0));
    assert_eq!(restore.pending(), 0);
    assert_eq!(raw_documents(&sides[1]).await, original);
}

#[tokio::test]
async fn unordered_partial_errors_keep_successful_rows_and_their_undo() {
    let (_container, client) = server("8.2.3").await;
    let db = client.database("partial");
    let sides = [db.collection::<Document>("left"), db.collection::<Document>("right")];
    sides[0]
        .insert_many([
            doc! {"_id":1,"sku":"one","email":"taken"},
            doc! {"_id":2,"sku":"two","email":"free"},
            doc! {"_id":3,"sku":"three","email":"third"},
        ])
        .await
        .unwrap();
    sides[1]
        .insert_many([
            doc! {"_id":50,"sku":"existing","email":"taken"},
            doc! {"_id":3,"sku":"different key","email":"collision"},
        ])
        .await
        .unwrap();
    sides[1]
        .create_index(
            IndexModel::builder()
                .keys(doc! {"email":1})
                .options(mongodb::options::IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .unwrap();
    let mut items = plan(&sides, &["sku"], doc! {}, Side::Right).await;
    items.retain(|item| item.operation == Operation::Insert);
    let original = raw_documents(&sides[1]).await;
    let directory = tempfile::tempdir().unwrap();
    let restore = Arc::new(RestoreHandle::create(directory.path()).unwrap());
    let result = sync(&sides, &["sku"], Side::Right, items, restore.clone()).await;
    assert_eq!((result.written, result.failed, result.uncertain), (1, 2, 0));
    assert_eq!(restore.pending(), 1);
    assert_eq!(undo(&sides[1], restore).await.written, 1);
    assert_eq!(raw_documents(&sides[1]).await, original);
}

#[tokio::test]
async fn undo_restores_a_deleted_identity_reused_by_an_insert_in_the_same_sync() {
    let (_container, client) = server("8.2.3").await;
    let db = client.database("reused_id");
    let sides = [db.collection::<Document>("left"), db.collection::<Document>("right")];
    sides[0].insert_one(doc! {"_id":1i64,"sku":"z-new"}).await.unwrap();
    sides[1].insert_one(doc! {"_id":1,"sku":"a-old"}).await.unwrap();
    let before = raw_documents(&sides[1]).await;
    let items = plan(&sides, &["sku"], doc! {}, Side::Right).await;
    let directory = tempfile::tempdir().unwrap();
    let restore = Arc::new(RestoreHandle::create(directory.path()).unwrap());
    let result = sync(&sides, &["sku"], Side::Right, items, restore.clone()).await;
    assert_eq!(result.written, 2);
    assert_eq!(restore.pending(), 2);
    assert_eq!(undo(&sides[1], restore.clone()).await.written, 2);
    assert_eq!(restore.pending(), 0);
    assert_eq!(raw_documents(&sides[1]).await, before);
}

#[tokio::test]
async fn cancel_between_bulk_batches_retains_a_working_undo() {
    let (_container, client) = server("8.2.3").await;
    let db = client.database("cancel");
    db.create_collection("right").await.unwrap();
    let sides = [db.collection::<Document>("left"), db.collection::<Document>("right")];
    sides[0].insert_many((0..2100).map(|id| doc! {"_id":id,"value":id})).await.unwrap();
    let items = plan(&sides, &["_id"], doc! {}, Side::Right).await;
    let directory = tempfile::tempdir().unwrap();
    let restore = Arc::new(RestoreHandle::create(directory.path()).unwrap());
    let cancel = CancellationToken::new();
    cancel.cancel();
    let (sender, _receiver) = futures::channel::mpsc::unbounded();
    let result = sync_collections_async(
        sides.each_ref().map(Collection::clone_with_type),
        Side::Right,
        vec!["_id".into()],
        items.clone(),
        restore.clone(),
        cancel,
        sender,
    )
    .await
    .unwrap();
    assert!(result.cancelled);
    assert_eq!(result.written, 0);
    let cancel = CancellationToken::new();
    let (sender, mut receiver) = futures::channel::mpsc::unbounded();
    let (result, ()) = tokio::join!(
        sync_collections_async(
            sides.each_ref().map(Collection::clone_with_type),
            Side::Right,
            vec!["_id".into()],
            items,
            restore.clone(),
            cancel.clone(),
            sender
        ),
        async {
            while let Some(progress) = receiver.next().await {
                if progress.summary.written >= 1000 {
                    cancel.cancel();
                }
            }
        }
    );
    let result = result.unwrap();
    assert!(result.cancelled);
    assert_eq!(result.written, 1000);
    assert_eq!(restore.pending(), 1000);
    assert_eq!(undo(&sides[1], restore).await.written, 1000);
    assert_eq!(sides[1].count_documents(doc! {}).await.unwrap(), 0);
}

#[tokio::test]
async fn large_documents_use_native_bulk_and_undo_without_truncation() {
    let (_container, client) = server("8.2.3").await;
    let db = client.database("large");
    let sides = [db.collection::<Document>("left"), db.collection::<Document>("right")];
    for (side, collection) in sides.iter().enumerate() {
        collection.insert_many((0..4).map(|id| doc! {"_id":id,"payload":if side == 0 { "a".repeat(9*1024*1024) } else { "b".repeat(9*1024*1024) }})).await.unwrap();
    }
    let original = raw_documents(&sides[1]).await;
    let items = plan(&sides, &["_id"], doc! {}, Side::Right).await;
    let directory = tempfile::tempdir().unwrap();
    let restore = Arc::new(RestoreHandle::create(directory.path()).unwrap());
    let result = sync(&sides, &["_id"], Side::Right, items, restore.clone()).await;
    assert_eq!((result.written, result.failed, result.uncertain), (4, 0, 0));
    assert_eq!(raw_documents(&sides[0]).await, raw_documents(&sides[1]).await);
    assert_eq!(undo(&sides[1], restore).await.written, 4);
    assert_eq!(raw_documents(&sides[1]).await, original);
}

#[tokio::test]
async fn old_servers_and_views_are_rejected_before_any_write() {
    for version in ["7.0", "8.2.3"] {
        let (_container, client) = server(version).await;
        let db = client.database("rejected");
        db.create_collection("source").await.unwrap();
        if version == "7.0" {
            db.create_collection("target").await.unwrap();
        } else {
            db.run_command(doc! {"create":"target","viewOn":"source","pipeline":[]}).await.unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let restore = Arc::new(RestoreHandle::create(directory.path()).unwrap());
        let (sender, _receiver) = futures::channel::mpsc::unbounded();
        let error = sync_collections_async(
            [db.collection("source"), db.collection("target")],
            Side::Right,
            vec!["_id".into()],
            vec![],
            restore.clone(),
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(if version == "7.0" { "8.0" } else { "regular collections" }),
            "{error}"
        );
        assert_eq!(restore.pending(), 0);
        if version == "7.0" {
            let (_new_container, new_client) = server("8.2.3").await;
            let sides = [
                db.collection::<Document>("source"),
                new_client.database("new_target").collection::<Document>("target"),
            ];
            sides[0].insert_one(doc! {"_id":1,"value":"old server source"}).await.unwrap();
            sides[1].insert_one(doc! {"_id":1,"value":"before"}).await.unwrap();
            let items = plan(&sides, &["_id"], doc! {}, Side::Right).await;
            assert_eq!(
                sync(&sides, &["_id"], Side::Right, items, restore.clone()).await.written,
                1
            );
            assert_eq!(undo(&sides[1], restore.clone()).await.written, 1);
        }
    }
}

#[tokio::test]
#[ignore = "manual performance measurement on a disposable MongoDB 8 container"]
async fn compare_sync_bulk_benchmark() {
    let (_container, client) = server("8.2.3").await;
    let db = client.database("benchmark");
    let sides = [db.collection::<Document>("left"), db.collection::<Document>("right")];
    for start in (0..100_000).step_by(1000) {
        sides[0]
            .insert_many(
                (start..start + 1000)
                    .map(|id| doc! {"_id":id,"value":"source","payload":"x".repeat(256)}),
            )
            .await
            .unwrap();
        sides[1]
            .insert_many(
                (start..start + 1000)
                    .map(|id| doc! {"_id":id,"value":"target","payload":"x".repeat(256)}),
            )
            .await
            .unwrap();
    }
    let items = plan(&sides, &["_id"], doc! {}, Side::Right).await;
    let directory = tempfile::tempdir().unwrap();
    let restore = Arc::new(RestoreHandle::create(directory.path()).unwrap());
    let started = std::time::Instant::now();
    let result = sync(&sides, &["_id"], Side::Right, items, restore.clone()).await;
    assert_eq!(result.written, 100_000);
    println!(
        "sync 100,000 replacements (including encrypted durable undo): {:?}",
        started.elapsed()
    );
    let started = std::time::Instant::now();
    assert_eq!(undo(&sides[1], restore).await.written, 100_000);
    println!("undo 100,000 replacements: {:?}", started.elapsed());
    assert_eq!(sides[1].count_documents(doc! {"value":"target"}).await.unwrap(), 100_000);
}
