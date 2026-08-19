use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mongodb::Client;
use mongodb::bson::{Document, doc};
use openmango::history::{
    BatchQuery, EligibilityStatus, GroupingKind, HistoryConnection, HistoryService, OperationFamily,
};
use openmango::mcp::{McpBridge, McpConnection, McpServer, McpServerHandle};
use rmcp::ServiceExt as _;
use rmcp::model::{CallToolRequestParams, ClientInfo};
use rmcp::transport::{
    StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;

async fn replica_set() -> (testcontainers::ContainerAsync<Mongo>, Client) {
    let container = Mongo::repl_set().with_tag("7.0").start().await.unwrap();
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let client = Client::with_uri_str(format!(
        "mongodb://{host}:{port}/?directConnection=true&serverSelectionTimeoutMS=5000"
    ))
    .await
    .unwrap();
    (container, client)
}

async fn wait_for_items(service: &HistoryService, connection_id: uuid::Uuid, minimum: u64) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let page = service
                .list_batches(BatchQuery {
                    connection_id,
                    database: None,
                    collection: None,
                    offset: 0,
                    limit: 100,
                })
                .unwrap();
            if page.items.iter().map(|batch| batch.item_count).sum::<u64>() >= minimum {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("History did not record expected events");
}

#[tokio::test(flavor = "multi_thread")]
async fn replica_set_history_captures_all_clients_groups_and_restores_without_overwrite() {
    let (_container, client) = replica_set().await;
    let database = format!("history_{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let collection = client.database(&database).collection::<Document>("items");
    collection.insert_many((0..120).map(|id| doc! { "_id": id, "value": 0 })).await.unwrap();
    client
        .database(&database)
        .run_command(doc! {
            "collMod": "items",
            "changeStreamPreAndPostImages": { "enabled": true },
        })
        .await
        .unwrap();

    let directory = tempfile::tempdir().unwrap();
    let connection_id = uuid::Uuid::new_v4();
    let connection = HistoryConnection {
        id: connection_id,
        name: "Replica set".into(),
        client: client.clone(),
        databases: vec![database.clone()],
        max_age_days: 30,
        max_bytes: 64 * 1024 * 1024,
    };
    let eligibility = HistoryService::eligibility(&connection).await;
    assert_eq!(eligibility.status, EligibilityStatus::Eligible, "{eligibility:?}");
    let service = Arc::new(
        HistoryService::open(
            directory.path().join("history.sqlite3"),
            [11; 32],
            tokio::runtime::Handle::current(),
        )
        .unwrap(),
    );
    service.start(connection.clone());
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mcp_connection = McpConnection {
        id: connection_id,
        name: "Replica set".into(),
        environment: Some("Development".into()),
        protected: false,
        read_only: false,
        writable: true,
        connected: true,
        databases: vec![database.clone()],
    };
    let mcp_server = McpServer::new(McpBridge::fixed_with_clients_and_history(
        vec![mcp_connection],
        HashMap::from([(connection_id, client.clone())]),
        Some(service.clone()),
    ));
    let handle = McpServerHandle::start(mcp_server, "history-token".into()).await.unwrap();
    let mut headers = HashMap::new();
    headers.insert(
        axum::http::HeaderName::from_static("authorization"),
        axum::http::HeaderValue::from_static("Bearer history-token"),
    );
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://{}/mcp", handle.addr()))
            .custom_headers(headers),
    );
    let mcp = ClientInfo::default().serve(transport).await.unwrap();
    let arguments = serde_json::json!({
        "connection_id": connection_id.to_string(),
        "database": database,
        "collection": "items",
        "filter": {},
        "update": { "$set": { "value": 1 } },
        "many": true,
        "allow_all": true
    })
    .as_object()
    .unwrap()
    .clone();
    let result = mcp
        .call_tool(
            CallToolRequestParams::new("openmango_update_documents").with_arguments(arguments),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    // A direct driver write represents Forge/external-client observation.
    collection.delete_one(doc! { "_id": 119 }).await.unwrap();
    wait_for_items(&service, connection_id, 121).await;

    let mut session = client.start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    collection
        .update_one(doc! { "_id": 0 }, doc! { "$set": { "transaction": true } })
        .session(&mut session)
        .await
        .unwrap();
    collection
        .update_one(doc! { "_id": 1 }, doc! { "$set": { "transaction": true } })
        .session(&mut session)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    wait_for_items(&service, connection_id, 123).await;

    let page = service
        .list_batches(BatchQuery {
            connection_id,
            database: Some(database.clone()),
            collection: Some("items".into()),
            offset: 0,
            limit: 100,
        })
        .unwrap();
    assert!(page.items.len() < 120, "document events must be batched");
    assert!(page.items.iter().any(|batch| batch.grouping == GroupingKind::Attributed));
    assert!(page.items.iter().any(|batch| batch.grouping == GroupingKind::Observed));
    let transaction = page
        .items
        .iter()
        .find(|batch| batch.grouping == GroupingKind::Transaction)
        .expect("transaction batch missing");
    assert_eq!(transaction.item_count, 2);

    let update_batch = page
        .items
        .iter()
        .find(|batch| batch.family == OperationFamily::Update && batch.item_count >= 100)
        .unwrap();
    collection.update_one(doc! { "_id": 0 }, doc! { "$set": { "value": 99 } }).await.unwrap();
    service.revert_batch(update_batch.id).unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let progress = service.restore_progress(update_batch.id).unwrap();
            if progress.done {
                assert!(progress.conflicted >= 1);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        collection.find_one(doc! { "_id": 0 }).await.unwrap().unwrap().get_i32("value"),
        Ok(99),
        "restore must not overwrite a conflict"
    );
    mcp.cancel().await.unwrap();
    handle.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_document_updates_restore_newest_first_to_original() {
    let (_container, client) = replica_set().await;
    let database = format!("history_order_{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let collection = client.database(&database).collection::<Document>("items");
    collection.insert_one(doc! { "_id": 1, "value": "A" }).await.unwrap();
    client
        .database(&database)
        .run_command(doc! {
            "collMod": "items",
            "changeStreamPreAndPostImages": { "enabled": true },
        })
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let connection_id = uuid::Uuid::new_v4();
    let service = HistoryService::open(
        directory.path().join("history.sqlite3"),
        [23; 32],
        tokio::runtime::Handle::current(),
    )
    .unwrap();
    service.start(HistoryConnection {
        id: connection_id,
        name: "Replica set".into(),
        client: client.clone(),
        databases: vec![database.clone()],
        max_age_days: 30,
        max_bytes: 64 * 1024 * 1024,
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    collection.update_one(doc! {}, doc! { "$set": { "value": "B" } }).await.unwrap();
    collection.update_one(doc! {}, doc! { "$set": { "value": "C" } }).await.unwrap();
    wait_for_items(&service, connection_id, 2).await;
    let page = service
        .list_batches(BatchQuery {
            connection_id,
            database: Some(database.clone()),
            collection: Some("items".into()),
            offset: 0,
            limit: 10,
        })
        .unwrap();
    let batch = page.items.iter().find(|batch| batch.item_count == 2).unwrap();
    service.revert_batch(batch.id).unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if service.restore_progress(batch.id).unwrap().done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(collection.find_one(doc! {}).await.unwrap().unwrap().get_str("value"), Ok("A"));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_restore_reinserts_only_absent_documents_and_preserves_conflicts() {
    let (_container, client) = replica_set().await;
    let database = format!("history_delete_{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let collection = client.database(&database).collection::<Document>("items");
    collection
        .insert_many([
            doc! { "_id": 1, "value": "original-one" },
            doc! { "_id": 2, "value": "original-two" },
        ])
        .await
        .unwrap();
    client
        .database(&database)
        .run_command(doc! {
            "collMod": "items",
            "changeStreamPreAndPostImages": { "enabled": true },
        })
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let connection_id = uuid::Uuid::new_v4();
    let service = HistoryService::open(
        directory.path().join("history.sqlite3"),
        [29; 32],
        tokio::runtime::Handle::current(),
    )
    .unwrap();
    service.start(HistoryConnection {
        id: connection_id,
        name: "Replica set".into(),
        client: client.clone(),
        databases: vec![database.clone()],
        max_age_days: 30,
        max_bytes: 64 * 1024 * 1024,
    });
    tokio::time::sleep(Duration::from_millis(500)).await;

    collection.delete_many(doc! {}).await.unwrap();
    wait_for_items(&service, connection_id, 2).await;
    let page = service
        .list_batches(BatchQuery {
            connection_id,
            database: Some(database.clone()),
            collection: Some("items".into()),
            offset: 0,
            limit: 10,
        })
        .unwrap();
    let batch = page
        .items
        .iter()
        .find(|batch| batch.family == OperationFamily::Delete && batch.item_count == 2)
        .unwrap();

    collection.insert_one(doc! { "_id": 2, "value": "concurrent" }).await.unwrap();
    service.revert_batch(batch.id).unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let progress = service.restore_progress(batch.id).unwrap();
            if progress.done {
                assert_eq!(progress.restored, 1);
                assert_eq!(progress.conflicted, 1);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();

    assert_eq!(
        collection.find_one(doc! { "_id": 1 }).await.unwrap().unwrap().get_str("value"),
        Ok("original-one")
    );
    assert_eq!(
        collection.find_one(doc! { "_id": 2 }).await.unwrap().unwrap().get_str("value"),
        Ok("concurrent"),
        "delete restore must never overwrite a reinserted document"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn replica_set_history_resumes_after_restart_without_duplicates() {
    let (_container, client) = replica_set().await;
    let database = format!("history_resume_{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let collection = client.database(&database).collection::<Document>("items");
    collection.insert_one(doc! { "_id": 1, "value": 0 }).await.unwrap();
    client
        .database(&database)
        .run_command(doc! {
            "collMod": "items",
            "changeStreamPreAndPostImages": { "enabled": true },
        })
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.sqlite3");
    let connection_id = uuid::Uuid::new_v4();
    let connection = HistoryConnection {
        id: connection_id,
        name: "Replica set".into(),
        client: client.clone(),
        databases: vec![database.clone()],
        max_age_days: 30,
        max_bytes: 64 * 1024 * 1024,
    };
    let service =
        HistoryService::open(path.clone(), [19; 32], tokio::runtime::Handle::current()).unwrap();
    service.start(connection.clone());
    tokio::time::sleep(Duration::from_millis(500)).await;
    collection.update_one(doc! {}, doc! { "$set": { "value": 1 } }).await.unwrap();
    wait_for_items(&service, connection_id, 1).await;
    service.stop(connection_id);
    drop(service);

    let resumed = HistoryService::open(path, [19; 32], tokio::runtime::Handle::current()).unwrap();
    resumed.start(connection);
    tokio::time::sleep(Duration::from_millis(500)).await;
    collection.update_one(doc! {}, doc! { "$set": { "value": 2 } }).await.unwrap();
    wait_for_items(&resumed, connection_id, 2).await;
    let page = resumed
        .list_batches(BatchQuery {
            connection_id,
            database: None,
            collection: None,
            offset: 0,
            limit: 10,
        })
        .unwrap();
    assert_eq!(page.items.iter().map(|batch| batch.item_count).sum::<u64>(), 2);
}
