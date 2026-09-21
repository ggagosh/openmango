//! Data-path benchmark: times the code the app runs to page, open, render, export, and import,
//! against a throwaway MongoDB container. Prints a Markdown table.
//!
//! cargo test --release --test bench_tests -- --ignored --nocapture

mod common;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use common::MongoTestContainer;
use mongodb::IndexModel;
use mongodb::bson::{DateTime, Document, doc, oid::ObjectId};
use openmango::bson::{DocumentKey, document_to_json_string};
use openmango::connection::{
    CancellationToken, ConnectionManager, FindDocumentsOptions, JsonExportOptions,
    JsonImportOptions,
};
use openmango::state::SessionDocument;
use openmango::views::documents::tree::lazy_tree::{
    build_visible_rows, collect_all_expandable_nodes,
};

const DOCS: u64 = 1_000_000;
const PAGE: i64 = 50;
const FAT_ITEMS: usize = 45_000;
const RUNS: usize = 9;

fn median_ms(mut run: impl FnMut()) -> f64 {
    let mut samples: Vec<f64> = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    samples[RUNS / 2]
}

fn order(i: u64) -> Document {
    const STATUS: [&str; 5] = ["pending", "paid", "shipped", "delivered", "cancelled"];
    doc! {
        "_id": ObjectId::new(),
        "user_id": (i % 50_000) as i64,
        "email": format!("user{}@example.com", i % 50_000),
        "status": STATUS[(i % 5) as usize],
        "amount": (i % 100_000) as f64 / 100.0,
        "created_at": DateTime::from_millis(1_700_000_000_000 + i as i64 * 1000),
        "tags": ["web", "promo", "returning"],
        "address": { "city": "Tbilisi", "zip": format!("{:04}", i % 10_000), "country": "GE" },
    }
}

fn fat_document() -> Document {
    let items: Vec<Document> = (0..FAT_ITEMS)
        .map(|i| {
            doc! {
                "sku": format!("SKU-{i:06}"),
                "qty": (i % 9) as i32,
                "price": i as f64 / 7.0,
                "note": "x".repeat(200),
                "attrs": { "color": "green", "size": "M" },
            }
        })
        .collect();
    doc! { "_id": ObjectId::new(), "kind": "fat", "items": items }
}

fn page(filter: Option<Document>, sort: Option<Document>, skip: u64) -> FindDocumentsOptions {
    FindDocumentsOptions {
        filter,
        sort,
        projection: None,
        skip,
        limit: PAGE,
        max_time: Duration::from_secs(120),
        cancellation: CancellationToken::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "benchmark; run with --release --ignored --nocapture"]
async fn bench_data_path() {
    let mongo = MongoTestContainer::start().await;
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || run(mongo, handle)).await.expect("bench panicked");
}

fn run(mongo: MongoTestContainer, handle: tokio::runtime::Handle) {
    let manager = ConnectionManager::new();
    let client = mongo.client.clone();
    let db = mongo.db_name("bench");
    let orders = mongo.collection::<Document>("bench", "orders");
    let mut rows: Vec<(String, String)> = Vec::new();

    // Seed (not measured).
    for start in (0..DOCS).step_by(10_000) {
        let batch: Vec<Document> = (start..start + 10_000).map(order).collect();
        handle.block_on(async { orders.insert_many(batch).await }).expect("seed");
    }
    let index = IndexModel::builder().keys(doc! { "status": 1, "created_at": -1 }).build();
    handle.block_on(async { orders.create_index(index).await }).expect("index");

    let mut find = |label: &str, opts: &dyn Fn() -> FindDocumentsOptions| {
        let ms = median_ms(|| {
            let (docs, _) = manager.find_documents(&client, &db, "orders", opts()).expect("find");
            assert_eq!(docs.len(), PAGE as usize);
        });
        rows.push((label.to_string(), format!("{ms:.1} ms")));
    };
    find("First page of 50, 1M-document collection", &|| page(None, None, 0));
    find("Page 18,000 of 50 (skip 900,000)", &|| page(None, None, 900_000));
    find("Filtered + sorted page, indexed (200k matches)", &|| {
        page(Some(doc! { "status": "paid" }), Some(doc! { "created_at": -1 }), 0)
    });

    // Where the first-page time goes: the page query counts exactly before it finds.
    let ms = median_ms(|| {
        let total = manager.count_documents(&client, &db, "orders", doc! {}).expect("count");
        assert_eq!(total, DOCS);
    });
    rows.push(("Exact count alone, no filter".to_string(), format!("{ms:.1} ms")));
    let ms = median_ms(|| {
        manager.estimated_document_count(&client, &db, "orders").expect("estimate");
    });
    rows.push(("Estimated count alone (collection metadata)".to_string(), format!("{ms:.1} ms")));

    // One fat document: fetch, fully expand, render as JSON.
    let fat = fat_document();
    let fat_id = fat.get("_id").cloned().expect("_id");
    let fat_mb = mongodb::bson::to_vec(&fat).expect("bson").len() as f64 / 1_048_576.0;
    let fat_coll = mongo.collection::<Document>("bench", "fat");
    handle.block_on(async { fat_coll.insert_one(fat).await }).expect("insert fat");

    let mut fetched = Vec::new();
    let ms = median_ms(|| {
        (fetched, _) = manager
            .find_documents(&client, &db, "fat", page(Some(doc! { "_id": &fat_id }), None, 0))
            .expect("find fat");
    });
    rows.push((format!("Fetch one {fat_mb:.1} MB document"), format!("{ms:.1} ms")));

    let session: Vec<SessionDocument> = fetched
        .into_iter()
        .enumerate()
        .map(|(i, doc)| SessionDocument { key: DocumentKey::from_document(&doc, i), doc })
        .collect();
    let mut expanded = HashSet::new();
    let mut row_count = 0;
    let ms = median_ms(|| {
        expanded = collect_all_expandable_nodes(&session);
        row_count = build_visible_rows(&session, &expanded).len();
    });
    rows.push((
        format!("Expand all: build {row_count} tree rows from that document"),
        format!("{ms:.1} ms"),
    ));
    let ms = median_ms(|| {
        std::hint::black_box(document_to_json_string(&session[0].doc));
    });
    rows.push(("Render that document as JSON text".to_string(), format!("{ms:.1} ms")));

    // Export, then import, the whole collection as JSON Lines (single run each).
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("orders.jsonl");
    let start = Instant::now();
    let exported = manager
        .export_collection_json_with_options(
            &client,
            &db,
            "orders",
            &path,
            JsonExportOptions::default(),
        )
        .expect("export");
    let secs = start.elapsed().as_secs_f64();
    let file_mb = std::fs::metadata(&path).expect("file").len() as f64 / 1_048_576.0;
    assert_eq!(exported, DOCS);
    rows.push((
        format!("Export 1M documents to JSON Lines ({file_mb:.0} MB)"),
        format!("{secs:.1} s ({:.0} docs/s)", DOCS as f64 / secs),
    ));

    let start = Instant::now();
    let imported = manager
        .import_collection_json_with_options(
            &client,
            &db,
            "orders_copy",
            &path,
            JsonImportOptions { batch_size: 1000, ..Default::default() },
        )
        .expect("import");
    let secs = start.elapsed().as_secs_f64();
    assert_eq!(imported, DOCS);
    rows.push((
        "Import that file into a new collection".to_string(),
        format!("{secs:.1} s ({:.0} docs/s)", DOCS as f64 / secs),
    ));

    println!("\n| Operation | Result |\n| --- | --- |");
    for (label, result) in rows {
        println!("| {label} | {result} |");
    }
    println!("\nMedian of {RUNS} runs unless a single run is noted.");
}
