//! Integration coverage for application-level read-only and AI confirmation policy.

mod common;

use std::time::Duration;

use common::MongoTestContainer;
use mongodb::bson::{Document, doc};
use openmango::ai::safety::SafetyTier;
use openmango::ai::tools::aggregate::{AggregateArgs, AggregateTool};
use openmango::ai::tools::insert::{InsertArgs, InsertDocumentsTool};
use openmango::ai::tools::update::{UpdateArgs, UpdateDocumentsTool};
use openmango::ai::tools::{MongoContext, StreamEvent};
use rig::tool::Tool;

#[tokio::test]
async fn read_only_ai_update_is_rejected_without_mutating_data() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "ai_read_only_update");
    collection
        .insert_one(doc! { "_id": "one", "status": "before" })
        .await
        .expect("Failed to seed collection");

    let tool = UpdateDocumentsTool::new(MongoContext {
        client: mongo.client.clone(),
        database: mongo.db_name("test_db"),
        collection: Some("ai_read_only_update".to_string()),
        read_only: true,
        event_tx: None,
    });
    let error = tool
        .call(UpdateArgs {
            collection: None,
            filter: r#"{"_id":"one"}"#.to_string(),
            update: r#"{"$set":{"status":"after"}}"#.to_string(),
            many: Some(false),
        })
        .await
        .expect_err("Read-only AI update must be rejected");

    assert!(error.to_string().contains("read-only"));
    let stored = collection.find_one(doc! { "_id": "one" }).await.unwrap().unwrap();
    assert_eq!(stored.get_str("status").unwrap(), "before");
}

#[tokio::test]
async fn ai_write_fails_closed_without_confirmation_channel() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "ai_missing_confirmation");

    let tool = InsertDocumentsTool::new(MongoContext {
        client: mongo.client.clone(),
        database: mongo.db_name("test_db"),
        collection: Some("ai_missing_confirmation".to_string()),
        read_only: false,
        event_tx: None,
    });
    let error = tool
        .call(InsertArgs { collection: None, documents: r#"[{"_id":"one"}]"#.to_string() })
        .await
        .expect_err("AI write without a confirmation channel must fail closed");

    assert!(error.to_string().contains("confirmation"));
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 0);
}

#[tokio::test]
async fn ai_update_one_confirmation_reports_one_affected_document() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "ai_update_one_preview");
    collection
        .insert_many(vec![doc! { "group": "a" }, doc! { "group": "a" }])
        .await
        .expect("Failed to seed collection");

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let tool = UpdateDocumentsTool::new(MongoContext {
        client: mongo.client.clone(),
        database: mongo.db_name("test_db"),
        collection: Some("ai_update_one_preview".to_string()),
        read_only: false,
        event_tx: Some(event_tx),
    });
    let call = tokio::spawn(async move {
        tool.call(UpdateArgs {
            collection: None,
            filter: r#"{"group":"a"}"#.to_string(),
            update: r#"{"$set":{"updated":true}}"#.to_string(),
            many: Some(false),
        })
        .await
    });

    let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("AI update did not request confirmation")
        .expect("AI confirmation channel closed");
    match event {
        StreamEvent::ConfirmationRequired { preview, response_tx, .. } => {
            assert_eq!(preview.affected_count, 1);
            response_tx.respond(false);
        }
        other => panic!("Unexpected AI event: {other:?}"),
    }

    call.await.expect("AI update task panicked").expect_err("Rejected update must not run");
    assert_eq!(collection.count_documents(doc! { "updated": true }).await.unwrap(), 0);
}

#[tokio::test]
async fn read_only_ai_output_stage_is_rejected_without_creating_target() {
    let mongo = MongoTestContainer::start().await;
    let source = mongo.collection::<Document>("test_db", "ai_read_only_aggregate");
    source.insert_one(doc! { "value": 1 }).await.expect("Failed to seed source");

    let tool = AggregateTool::new(MongoContext {
        client: mongo.client.clone(),
        database: mongo.db_name("test_db"),
        collection: Some("ai_read_only_aggregate".to_string()),
        read_only: true,
        event_tx: None,
    });
    let error = tool
        .call(AggregateArgs {
            collection: None,
            pipeline: r#"[{"$out":"ai_read_only_output"}]"#.to_string(),
        })
        .await
        .expect_err("Read-only AI output stage must be rejected");

    assert!(error.to_string().contains("read-only"));
    let names = mongo.database("test_db").list_collection_names().await.unwrap();
    assert!(!names.iter().any(|name| name == "ai_read_only_output"));
}

#[tokio::test]
async fn writable_ai_output_stage_requires_confirmation_before_execution() {
    let mongo = MongoTestContainer::start().await;
    let source = mongo.collection::<Document>("test_db", "ai_confirmed_aggregate");
    source
        .insert_many(vec![doc! { "value": 1 }, doc! { "value": 2 }])
        .await
        .expect("Failed to seed source");

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let tool = AggregateTool::new(MongoContext {
        client: mongo.client.clone(),
        database: mongo.db_name("test_db"),
        collection: Some("ai_confirmed_aggregate".to_string()),
        read_only: false,
        event_tx: Some(event_tx),
    });
    let call = tokio::spawn(async move {
        tool.call(AggregateArgs {
            collection: None,
            pipeline: r#"[{"$limit":2},{"$out":"ai_confirmed_output"}]"#.to_string(),
        })
        .await
    });

    let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("AI aggregation did not request confirmation")
        .expect("AI confirmation channel closed");
    match event {
        StreamEvent::ConfirmationRequired { tool_name, tier, preview, response_tx, .. } => {
            assert_eq!(tool_name, "aggregate");
            assert_eq!(tier, SafetyTier::AlwaysConfirm);
            assert!(preview.collection.contains("ai_confirmed_output"));
            response_tx.respond(true);
        }
        other => panic!("Unexpected AI event: {other:?}"),
    }

    call.await.expect("AI aggregation task panicked").expect("Confirmed aggregation failed");
    let output = mongo.collection::<Document>("test_db", "ai_confirmed_output");
    assert_eq!(output.count_documents(doc! {}).await.unwrap(), 2);
}
