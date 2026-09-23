//! Read-only compare contract against MongoDB. No sync writes are implemented in this slice.

mod common;

use common::MongoTestContainer;
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::{
    Binary, Bson, DateTime, Document, RawDocumentBuf, Timestamp, doc, oid::ObjectId,
    spec::BinarySubtype,
};
use mongodb::options::Collation;
use mongodb::{Collection, IndexModel};
use openmango::bson::compare::{cmp_key_value, key_value};
use openmango::connection::CancellationToken;
use openmango::connection::ops::compare::{
    CompareMessage, CompareOptions, CompareSummary, DiffKind, DiffRow, Side, SortPlan,
    compare_collections_async,
};

async fn compare(
    left: Collection<RawDocumentBuf>,
    right: Collection<RawDocumentBuf>,
    options: CompareOptions,
) -> (CompareSummary, Vec<DiffRow>, SortPlan) {
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    let (result, messages) = tokio::join!(
        compare_collections_async(left, right, options, CancellationToken::new(), sender),
        receiver.collect::<Vec<_>>(),
    );
    let mut rows = Vec::new();
    let mut plan = None;
    for message in messages {
        match message {
            CompareMessage::Progress { new_rows, .. } => rows.extend(new_rows),
            CompareMessage::Prepared { sort, .. } => plan = Some(sort),
            CompareMessage::Failed(error) => panic!("{error}"),
            CompareMessage::Done(_) => {}
        }
    }
    (result.unwrap(), rows, plan.unwrap())
}

#[tokio::test]
async fn id_comparison_classifies_every_bucket_and_hashes_full_documents() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.database("compare");
    let left = database.collection::<Document>("left");
    let right = database.collection::<Document>("right");
    left.insert_many([
        doc! {"_id": 1, "value": "same"},
        doc! {"_id": 2, "value": "before"},
        doc! {"_id": 3, "value": 1},
        doc! {"_id": 4, "value": "left only"},
        doc! {"_id": 5, "a": 1, "b": 2},
    ])
    .await
    .unwrap();
    right
        .insert_many([
            doc! {"_id": 1, "value": "same"},
            doc! {"_id": 2, "value": "after"},
            doc! {"_id": 3, "value": 1.0},
            doc! {"_id": 5, "b": 2, "a": 1},
            doc! {"_id": 6, "value": "right only"},
        ])
        .await
        .unwrap();
    let (summary, rows, plan) =
        compare(left.clone_with_type(), right.clone_with_type(), CompareOptions::default()).await;
    assert!(plan.left_covered && plan.right_covered);
    assert_eq!(summary.counts.identical, 1);
    assert_eq!(summary.counts.different, 1);
    assert_eq!(summary.counts.minor, 2);
    assert_eq!(summary.counts.only_left, 1);
    assert_eq!(summary.counts.only_right, 1);
    assert_eq!(summary.counts.left_read, 5);
    assert_eq!(summary.counts.right_read, 5);
    assert_eq!(summary.skipped, Some([0, 0]));
    assert_eq!(rows.len(), 5);
    let changed = rows.iter().find(|r| r.kind == DiffKind::Different).unwrap();
    assert_eq!(&*changed.paths, "value");
    assert_ne!(changed.left_hash, changed.right_hash);
    assert_eq!(changed.id_on(Side::Left, true), Some(&Bson::Int32(2)));
}

#[tokio::test]
async fn custom_keys_ignore_ids_skip_ineligible_documents_and_group_duplicates() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.database("compare");
    let left = database.collection::<Document>("left");
    let right = database.collection::<Document>("right");
    left.insert_many([
        doc! {"_id": 1, "sku": "same", "value": 1},
        doc! {"_id": 2, "sku": "duplicate"},
        doc! {"_id": 3, "sku": "duplicate"},
        doc! {"_id": 4},
        doc! {"_id": 5, "sku": ["array"]},
        doc! {"_id": 6, "sku": null},
        doc! {"_id": 7, "sku": "left only"},
    ])
    .await
    .unwrap();
    right
        .insert_many([
            doc! {"_id": 101, "sku": "same", "value": 1},
            doc! {"_id": 102, "sku": "duplicate"},
            doc! {"_id": 106, "sku": null},
        ])
        .await
        .unwrap();
    let (summary, rows, _) = compare(
        left.clone_with_type(),
        right.clone_with_type(),
        CompareOptions { fields: vec!["sku".into()], ..Default::default() },
    )
    .await;
    assert_eq!(summary.skipped, Some([2, 0]));
    assert_eq!(summary.counts.identical, 2);
    assert_eq!(summary.counts.multiple_matches, 1);
    assert_eq!(summary.counts.only_left, 1);
    assert_eq!(rows.len(), 2);
    let duplicate = rows.iter().find(|r| r.kind == DiffKind::MultipleMatches).unwrap();
    assert_eq!((duplicate.left_count, duplicate.right_count), (2, 1));
    assert!(duplicate.id_on(Side::Left, false).is_none());
    let only_left = rows.iter().find(|r| r.kind == DiffKind::OnlyLeft).unwrap();
    assert_eq!(only_left.id_on(Side::Left, false), Some(&Bson::Int32(7)));
    assert!(only_left.id_on(Side::Right, false).is_none());
}

#[tokio::test]
async fn compound_key_uses_shared_index_order_and_preserves_filters() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.database("compare");
    let left = database.collection::<Document>("left");
    let right = database.collection::<Document>("right");
    for (collection, offset) in [(&left, 0), (&right, 100)] {
        collection
            .insert_many([
                doc! {"_id": offset + 1, "a": "x", "b": 1, "active": true},
                doc! {"_id": offset + 2, "a": "y", "b": 0, "active": true},
                doc! {"_id": offset + 3, "a": "x", "b": 1, "active": false},
            ])
            .await
            .unwrap();
        collection
            .create_index(IndexModel::builder().keys(doc! {"b": 1, "a": 1}).build())
            .await
            .unwrap();
    }
    let (summary, rows, plan) = compare(
        left.clone_with_type(),
        right.clone_with_type(),
        CompareOptions {
            fields: vec!["a".into(), "b".into()],
            filter: doc! {"active": true},
            ..Default::default()
        },
    )
    .await;
    assert_eq!(plan.fields, ["b", "a"]);
    assert!(plan.left_covered && plan.right_covered);
    assert_eq!(summary.counts.identical, 2);
    assert!(rows.is_empty());
}

#[tokio::test]
async fn server_sort_matches_client_for_every_supported_key_bracket() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("compare", "ordering");
    let keys = vec![
        Bson::MaxKey,
        Bson::Null,
        Bson::MinKey,
        Bson::Double(f64::NAN),
        Bson::Double(f64::NEG_INFINITY),
        Bson::Int64(i64::MIN),
        Bson::Double(-1.5),
        Bson::Int32(-1),
        Bson::Int32(0),
        Bson::Double(-0.0),
        Bson::Int32(1),
        Bson::Double(1.0),
        Bson::Int64(9_007_199_254_740_993),
        Bson::Double(9_007_199_254_740_992.0),
        Bson::Int64(i64::MAX),
        Bson::Double(9_223_372_036_854_775_808.0),
        Bson::Double(f64::INFINITY),
        Bson::String("Z".into()),
        Bson::String("é".into()),
        Bson::Document(doc! {}),
        Bson::Document(doc! {"z": 1}),
        Bson::Document(doc! {"a": "x"}),
        Bson::Document(doc! {"a": [1, 2]}),
        Bson::Document(doc! {"a": [1, 3]}),
        Bson::Binary(Binary { subtype: BinarySubtype::Generic, bytes: vec![255] }),
        Bson::Binary(Binary { subtype: BinarySubtype::UserDefined(128), bytes: vec![0] }),
        Bson::Binary(Binary { subtype: BinarySubtype::Generic, bytes: vec![0, 0] }),
        Bson::ObjectId(ObjectId::from_bytes([0; 12])),
        Bson::Boolean(false),
        Bson::Boolean(true),
        Bson::DateTime(DateTime::from_millis(0)),
        Bson::Timestamp(Timestamp { time: 0, increment: 1 }),
    ];
    collection.insert_many(keys.into_iter().map(|key| doc! {"key": key})).await.unwrap();
    let raw = collection.clone_with_type::<RawDocumentBuf>();
    let docs: Vec<_> = raw
        .find(doc! {})
        .sort(doc! {"key": 1})
        .collation(Collation::builder().locale("simple").build())
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    for pair in docs.windows(2) {
        let a = key_value(&pair[0], "key").unwrap().unwrap();
        let b = key_value(&pair[1], "key").unwrap().unwrap();
        assert!(cmp_key_value(a, b).unwrap().is_le(), "server ordered {a:?} before {b:?}");
    }
}

#[tokio::test]
async fn default_collation_is_overridden_and_simple_views_can_be_read() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.database("compare");
    database
        .create_collection("left")
        .collation(Collation::builder().locale("en").build())
        .await
        .unwrap();
    let left = database.collection::<Document>("left");
    let right = database.collection::<Document>("right");
    for (collection, offset) in [(&left, 0), (&right, 100)] {
        collection
            .insert_many([
                doc! {"_id": offset + 1, "key": "Z"},
                doc! {"_id": offset + 2, "key": "a"},
            ])
            .await
            .unwrap();
    }
    let options = CompareOptions { fields: vec!["key".into()], ..Default::default() };
    let (summary, _, plan) =
        compare(left.clone_with_type(), right.clone_with_type(), options.clone()).await;
    assert_eq!(summary.counts.identical, 2);
    assert!(!plan.left_covered);
    database.create_collection("view").view_on("right").pipeline(vec![]).await.unwrap();
    let (summary, _, _) =
        compare(database.collection("view"), right.clone_with_type(), options.clone()).await;
    assert_eq!(summary.counts.identical, 2);
    database
        .create_collection("projected")
        .view_on("right")
        .pipeline(vec![doc! {"$project": {"_id": 0}}])
        .await
        .unwrap();
    let (summary, _, _) =
        compare(database.collection("projected"), right.clone_with_type(), options.clone()).await;
    assert_eq!(summary.counts.identical, 2);
    right.update_one(doc! {"key": "Z"}, doc! {"$set": {"value": 1}}).await.unwrap();
    let (_, rows, _) =
        compare(database.collection("projected"), left.clone_with_type(), options).await;
    let different = rows.iter().find(|row| row.kind == DiffKind::Different).unwrap();
    assert_eq!(different.left_count, 1);
    assert!(different.left_id.is_none());
}

#[tokio::test]
async fn skipped_counts_apply_the_same_simple_collation_as_the_scan_filter() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.database("compare");
    for name in ["left", "right"] {
        database
            .create_collection(name)
            .collation(
                Collation::builder()
                    .locale("en")
                    .strength(mongodb::options::CollationStrength::Primary)
                    .build(),
            )
            .await
            .unwrap();
        database
            .collection::<Document>(name)
            .insert_many([
                doc! {"_id": 1, "tenant": "A", "sku": "x"},
                doc! {"_id": 2, "tenant": "A"},
                doc! {"_id": 3, "tenant": "a"},
            ])
            .await
            .unwrap();
    }
    let (summary, _, _) = compare(
        database.collection("left"),
        database.collection("right"),
        CompareOptions {
            fields: vec!["sku".into()],
            filter: doc! {"tenant": "A"},
            ..Default::default()
        },
    )
    .await;
    assert_eq!(summary.counts.identical, 1);
    assert_eq!(summary.skipped, Some([1, 1]));
}

#[tokio::test]
async fn dotted_array_keys_fail_even_when_the_eligibility_filter_would_hide_them() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.database("compare");
    database
        .collection::<Document>("left")
        .insert_one(doc! {"customer": [{"email": ["x"]}]})
        .await
        .unwrap();
    database
        .collection::<Document>("right")
        .insert_one(doc! {"customer": {"email": "x"}})
        .await
        .unwrap();
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    let error = compare_collections_async(
        database.collection("left"),
        database.collection("right"),
        CompareOptions { fields: vec!["customer.email".into()], ..Default::default() },
        CancellationToken::new(),
        sender,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("passes through an array"));
    assert!(receiver.any(|m| async move { matches!(m, CompareMessage::Failed(_)) }).await);
}

#[tokio::test]
async fn cancellation_returns_promptly_with_a_terminal_summary() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.database("compare");
    for name in ["left", "right"] {
        database
            .collection::<Document>(name)
            .insert_many((0..10_000).map(|id| doc! {"_id": id, "value": "x".repeat(1024)}))
            .await
            .unwrap();
    }
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    let scan = compare_collections_async(
        database.collection("left"),
        database.collection("right"),
        CompareOptions::default(),
        cancellation,
        sender,
    );
    let stop = async move {
        let mut receiver = receiver;
        while let Some(message) = receiver.next().await {
            if matches!(message, CompareMessage::Prepared { .. }) {
                cancel.cancel();
            }
            if let CompareMessage::Done(summary) = message {
                return summary;
            }
        }
        panic!("missing terminal summary");
    };
    let (result, terminal) =
        tokio::time::timeout(std::time::Duration::from_secs(5), async { tokio::join!(scan, stop) })
            .await
            .unwrap();
    let summary = result.unwrap();
    assert!(summary.cancelled && terminal.cancelled);
    assert_eq!(summary.counts, terminal.counts);
    assert_eq!(summary.skipped, None);
}

/// Seed with scripts/compare-bench-seed.js, then run explicitly with --ignored --nocapture.
/// /usr/bin/time -l records peak RSS on macOS; this prints throughput without retaining rows.
#[tokio::test]
#[ignore = "manual million-document benchmark; requires OPENMANGO_COMPARE_BENCH_URI and seeded data"]
async fn compare_million_document_benchmark() {
    let uri = std::env::var("OPENMANGO_COMPARE_BENCH_URI").expect("set the benchmark server URI");
    let client = mongodb::Client::with_uri_str(uri).await.unwrap();
    let database = client.database("openmango_compare_bench");
    for (left, right, field) in
        [("left", "right", "_id"), ("left", "right_sku", "sku"), ("left", "right_sku", "unindexed")]
    {
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        let (result, ()) = tokio::join!(
            compare_collections_async(
                database.collection(left),
                database.collection(right),
                CompareOptions { fields: vec![field.into()], ..Default::default() },
                CancellationToken::new(),
                sender
            ),
            receiver.for_each(|_| async {}),
        );
        let summary = result.unwrap();
        assert_eq!(summary.counts.identical, 990_000);
        assert_eq!(summary.counts.different, 10_000);
        println!(
            "{field}: {:.0} documents/s, elapsed {:?}, counts {:?}",
            (summary.counts.left_read + summary.counts.right_read) as f64
                / summary.elapsed.as_secs_f64(),
            summary.elapsed,
            summary.counts
        );
    }
}

#[tokio::test]
async fn database_listing_pairs_collections_by_name_with_kinds_and_sizes() {
    use openmango::connection::ops::compare_database::{
        CollectionKind, PairKind, list_side, pair_collections,
    };
    let mongo = MongoTestContainer::start().await;
    let left = mongo.database("compare_db_left");
    let right = mongo.database("compare_db_right");
    left.collection::<Document>("orders")
        .insert_many([doc! {"_id": 1}, doc! {"_id": 2}])
        .await
        .unwrap();
    right.collection::<Document>("orders").insert_one(doc! {"_id": 1}).await.unwrap();
    left.collection::<Document>("audit").insert_one(doc! {"_id": 1}).await.unwrap();
    for database in [&left, &right] {
        database
            .create_collection("metrics")
            .timeseries(
                mongodb::options::TimeseriesOptions::builder().time_field("at".to_string()).build(),
            )
            .await
            .unwrap();
    }
    // A view adds `system.views`, and time-series adds `system.buckets.*`: neither may be listed.
    left.create_collection("recent")
        .view_on("orders".to_string())
        .pipeline(Vec::new())
        .await
        .unwrap();
    right.collection::<Document>("recent").insert_one(doc! {"_id": 1}).await.unwrap();

    let timeout = std::time::Duration::from_secs(10);
    let left = list_side(&mongo.client, left.name(), timeout).await.unwrap();
    let right = list_side(&mongo.client, right.name(), timeout).await.unwrap();
    assert!(left.iter().chain(&right).all(|(name, _)| !name.starts_with("system.")));
    let pairs = pair_collections(left, right);
    let kinds: Vec<_> = pairs.iter().map(|pair| (pair.name.as_str(), pair.kind())).collect();
    assert_eq!(
        kinds,
        [
            ("audit", PairKind::LeftOnly),
            ("metrics", PairKind::NotComparable(CollectionKind::Timeseries)),
            ("orders", PairKind::Both),
            ("recent", PairKind::NotComparable(CollectionKind::View)),
        ]
    );
    let orders = pairs[2].sides.each_ref().map(|side| side.clone().unwrap());
    assert_eq!(orders.each_ref().map(|side| side.estimated), [Some(2), Some(1)]);
    assert!(orders.iter().all(|side| side.bytes.is_some_and(|bytes| bytes > 0)));
}

#[tokio::test]
async fn database_scan_counts_each_collection_with_one_comparator_and_honours_skips() {
    use openmango::bson::compare::IgnoreSet;
    use openmango::connection::ops::compare_database::{
        PairMessage, PairScan, compare_pairs_async,
    };
    let mongo = MongoTestContainer::start().await;
    let left = mongo.database("scan_left");
    let right = mongo.database("scan_right");
    let seed = |database: &mongodb::Database, name: &str, documents: Vec<Document>| {
        let collection = database.collection::<Document>(name);
        async move { collection.insert_many(documents).await.unwrap() }
    };
    seed(&left, "same", vec![doc! {"_id": 1}, doc! {"_id": 2}]).await;
    seed(&right, "same", vec![doc! {"_id": 1}, doc! {"_id": 2}]).await;
    seed(&left, "changed", vec![doc! {"_id": 1, "v": "a"}, doc! {"_id": 2, "v": "b"}]).await;
    seed(
        &right,
        "changed",
        vec![doc! {"_id": 1, "v": "a"}, doc! {"_id": 2, "v": "c"}, doc! {"_id": 3}],
    )
    .await;
    seed(&left, "reordered", vec![doc! {"_id": 1, "a": 1, "b": 2}]).await;
    seed(&right, "reordered", vec![doc! {"_id": 1, "b": 2, "a": 1}]).await;
    seed(&left, "stamped", vec![doc! {"_id": 1, "v": 1, "updatedAt": 1}]).await;
    seed(&right, "stamped", vec![doc! {"_id": 1, "v": 1, "updatedAt": 2}]).await;
    seed(&left, "skipped", vec![doc! {"_id": 1}]).await;
    seed(&right, "skipped", vec![doc! {"_id": 2}]).await;

    let names = ["same", "changed", "reordered", "stamped", "skipped"];
    let scans: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(index, name)| PairScan {
            index,
            name: name.to_string(),
            cancellation: CancellationToken::new(),
        })
        .collect();
    scans[4].cancellation.cancel();
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    let ((), messages) = tokio::join!(
        compare_pairs_async(
            [mongo.client.clone(), mongo.client.clone()],
            [left.name().to_string(), right.name().to_string()],
            scans,
            IgnoreSet::new(&["updatedAt".to_string()]),
            sender,
        ),
        receiver.collect::<Vec<_>>(),
    );
    let mut done = std::collections::BTreeMap::new();
    for message in messages {
        match message {
            PairMessage::Done(index, summary) => {
                done.insert(names[index], summary.counts);
            }
            PairMessage::Failed(index, error) => panic!("{}: {error}", names[index]),
            PairMessage::Started(_) | PairMessage::Progress(..) => {}
        }
    }
    assert!(!done.contains_key("skipped"), "a skipped collection is never read");
    let counts = |name| {
        let c = done[name];
        (c.identical, c.different, c.minor, c.only_left, c.only_right)
    };
    assert_eq!(counts("same"), (2, 0, 0, 0, 0));
    assert_eq!(counts("changed"), (1, 1, 0, 0, 1));
    assert_eq!(counts("reordered"), (0, 0, 1, 0, 0));
    assert_eq!(counts("stamped"), (1, 0, 0, 0, 0), "ignored fields apply to every collection");
}
