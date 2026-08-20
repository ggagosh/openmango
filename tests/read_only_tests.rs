//! Integration coverage for application-level read-only and AI confirmation policy.

mod common;

use std::time::Duration;

use common::MongoTestContainer;
use mongodb::bson::{Document, doc};
use openmango::ai::safety::SafetyTier;
use openmango::ai::tools::aggregate::{AggregateArgs, AggregateTool};
use openmango::ai::tools::create_index::{CreateIndexArgs, CreateIndexTool};
use openmango::ai::tools::insert::{InsertArgs, InsertDocumentsTool};
use openmango::ai::tools::replace::{ReplaceArgs, ReplaceDocumentsTool};
use openmango::ai::tools::{MongoContext, StreamEvent};
use openmango::models::{ConnectionWriteIdentity, SavedConnection};
use rig::tool::Tool;

fn write_identity(read_only: bool) -> ConnectionWriteIdentity {
    let mut connection =
        SavedConnection::new("Integration test".into(), "mongodb://localhost".into());
    connection.read_only = read_only;
    ConnectionWriteIdentity::from(&connection)
}

#[tokio::test]
async fn read_only_ai_replacement_is_rejected_without_mutating_data() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "ai_read_only_update");
    collection
        .insert_one(doc! { "_id": "one", "status": "before" })
        .await
        .expect("Failed to seed collection");

    let tool = ReplaceDocumentsTool::new(MongoContext {
        client: mongo.client.clone(),
        database: mongo.db_name("test_db"),
        collection: Some("ai_read_only_update".to_string()),
        write_identity: write_identity(true),
        read_only: true,
        event_tx: None,
    });
    let error = tool
        .call(ReplaceArgs {
            collection: None,
            filter: r#"{"_id":"one"}"#.to_string(),
            replacement: r#"{"status":"after"}"#.to_string(),
            many: Some(false),
        })
        .await
        .expect_err("Read-only AI replacement must be rejected");

    assert!(error.to_string().contains("read-only"));
    let stored = collection.find_one(doc! { "_id": "one" }).await.unwrap().unwrap();
    assert_eq!(stored.get_str("status").unwrap(), "before");
}

#[tokio::test]
async fn read_only_ai_index_creation_is_rejected() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "ai_read_only_index");
    collection.insert_one(doc! { "email": "ada@example.com" }).await.unwrap();
    let tool = CreateIndexTool::new(MongoContext {
        client: mongo.client.clone(),
        database: mongo.db_name("test_db"),
        collection: Some("ai_read_only_index".to_string()),
        write_identity: write_identity(true),
        read_only: true,
        event_tx: None,
    });

    let error = tool
        .call(CreateIndexArgs {
            collection: None,
            keys: r#"{"email":1}"#.to_string(),
            unique: Some(true),
            name: Some("email_unique".to_string()),
        })
        .await
        .expect_err("Read-only AI index creation must be rejected");

    assert!(error.to_string().contains("read-only"));
    assert!(!collection.list_index_names().await.unwrap().contains(&"email_unique".to_string()));
}

#[tokio::test]
async fn ai_write_requires_confirmation_but_not_history() {
    let mongo = MongoTestContainer::start().await;
    let collection = mongo.collection::<Document>("test_db", "ai_confirmed_insert");
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let tool = InsertDocumentsTool::new(MongoContext {
        client: mongo.client.clone(),
        database: mongo.db_name("test_db"),
        collection: Some("ai_confirmed_insert".to_string()),
        write_identity: write_identity(false),
        read_only: false,
        event_tx: Some(event_tx),
    });
    let call = tokio::spawn(async move {
        tool.call(InsertArgs { collection: None, documents: r#"[{"_id":"one"}]"#.to_string() })
            .await
    });
    let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("AI insert did not request confirmation")
        .expect("AI confirmation channel closed");
    match event {
        StreamEvent::ConfirmationRequired { response_tx, .. } => response_tx.respond(true),
        other => panic!("Unexpected AI event: {other:?}"),
    }
    call.await.expect("AI insert task panicked").expect("Confirmed insert failed");
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 1);
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
        write_identity: write_identity(true),
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
        write_identity: write_identity(false),
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
