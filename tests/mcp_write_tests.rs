mod common;

use std::collections::HashMap;

use common::MongoTestContainer;
use mongodb::bson::{Document, doc};
use openmango::mcp::{McpBridge, McpConnection, McpServer, McpServerHandle};
use rmcp::ServiceExt as _;
use rmcp::model::{CallToolRequestParams, ClientInfo};
use rmcp::transport::{
    StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
};

async fn mcp_client(
    mongo: &MongoTestContainer,
    connection_id: uuid::Uuid,
    writable: bool,
    read_only: bool,
) -> (rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>, McpServerHandle) {
    let connection = McpConnection {
        id: connection_id,
        name: "Test".into(),
        environment: Some("Development".into()),
        protected: false,
        read_only,
        writable,
        connected: true,
        databases: vec![],
    };
    let server = McpServer::new(McpBridge::fixed_with_clients(
        vec![connection],
        HashMap::from([(connection_id, mongo.client.clone())]),
    ));
    let handle = McpServerHandle::start(server, "test-token".into()).await.unwrap();
    let mut headers = HashMap::new();
    headers.insert(
        axum::http::HeaderName::from_static("authorization"),
        axum::http::HeaderValue::from_static("Bearer test-token"),
    );
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://{}/mcp", handle.addr()))
            .custom_headers(headers),
    );
    let client = ClientInfo::default().serve(transport).await.unwrap();
    (client, handle)
}

async fn call_success(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>,
    name: &'static str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let result = client
        .call_tool(
            CallToolRequestParams::new(name).with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    result.structured_content.unwrap()
}

fn assert_trace_id(content: &serde_json::Value) {
    uuid::Uuid::parse_str(content["openmango_trace_id"].as_str().unwrap()).unwrap();
}

#[tokio::test]
async fn direct_document_tools_round_trip_over_real_mcp_transport() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.db_name("mcp_write_round_trip");
    let collection = mongo.client.database(&database).collection::<Document>("items");
    let connection_id = uuid::Uuid::new_v4();
    let (client, handle) = mcp_client(&mongo, connection_id, true, false).await;
    let first_id = mongodb::bson::oid::ObjectId::new();
    let second_id = mongodb::bson::oid::ObjectId::new();

    let inserted = call_success(
        &client,
        "openmango_insert_documents",
        serde_json::json!({
            "connection_id": connection_id.to_string(),
            "database": database,
            "collection": "items",
            "documents": [
                { "_id": { "$oid": first_id.to_hex() }, "status": "new" },
                { "_id": { "$oid": second_id.to_hex() }, "status": "new" }
            ]
        }),
    )
    .await;
    assert_eq!(inserted["inserted_count"], 2);
    assert_eq!(inserted["inserted_ids"][0]["$oid"], first_id.to_hex());
    assert_trace_id(&inserted);

    let updated = call_success(
        &client,
        "openmango_update_documents",
        serde_json::json!({
            "connection_id": connection_id.to_string(),
            "database": database,
            "collection": "items",
            "filter": { "_id": { "$oid": first_id.to_hex() } },
            "update": [{ "$set": { "status": "updated" } }]
        }),
    )
    .await;
    assert_eq!(updated["matched_count"], 1);
    assert_eq!(updated["modified_count"], 1);
    assert_trace_id(&updated);

    let replaced = call_success(
        &client,
        "openmango_replace_document",
        serde_json::json!({
            "connection_id": connection_id.to_string(),
            "database": database,
            "collection": "items",
            "filter": { "_id": { "$oid": second_id.to_hex() } },
            "replacement": {
                "_id": { "$oid": second_id.to_hex() },
                "status": "replaced",
                "count": { "$numberLong": "42" }
            }
        }),
    )
    .await;
    assert_eq!(replaced["matched_count"], 1);
    assert_eq!(replaced["modified_count"], 1);
    assert_trace_id(&replaced);

    let deleted = call_success(
        &client,
        "openmango_delete_documents",
        serde_json::json!({
            "connection_id": connection_id.to_string(),
            "database": database,
            "collection": "items",
            "filter": { "_id": { "$oid": first_id.to_hex() } }
        }),
    )
    .await;
    assert_eq!(deleted["deleted_count"], 1);
    assert_trace_id(&deleted);

    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 1);
    let remaining = collection.find_one(doc! { "_id": second_id }).await.unwrap().unwrap();
    assert_eq!(remaining.get_str("status").unwrap(), "replaced");
    assert_eq!(remaining.get_i64("count").unwrap(), 42);

    client.cancel().await.unwrap();
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn direct_writes_reject_javascript_and_oversized_insert_batches() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.db_name("mcp_write_validation");
    let collection = mongo.client.database(&database).collection::<Document>("items");
    collection.insert_one(doc! { "_id": 1, "status": "unchanged" }).await.unwrap();
    let connection_id = uuid::Uuid::new_v4();
    let (client, handle) = mcp_client(&mongo, connection_id, true, false).await;

    for (name, arguments, expected) in [
        (
            "openmango_delete_documents",
            serde_json::json!({
                "connection_id": connection_id.to_string(),
                "database": database,
                "collection": "items",
                "filter": { "$where": "return true" }
            }),
            "$where is not allowed",
        ),
        (
            "openmango_update_documents",
            serde_json::json!({
                "connection_id": connection_id.to_string(),
                "database": database,
                "collection": "items",
                "filter": { "_id": 1 },
                "update": [{
                    "$set": {
                        "status": {
                            "$function": { "body": "return 'changed'", "args": [], "lang": "js" }
                        }
                    }
                }]
            }),
            "$function is not allowed",
        ),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new(name)
                    .with_arguments(arguments.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true), "{result:?}");
        assert!(format!("{result:?}").contains(expected), "{result:?}");
    }

    let documents: Vec<_> = (0..101).map(|index| serde_json::json!({ "_id": index })).collect();
    let result = client
        .call_tool(
            CallToolRequestParams::new("openmango_insert_documents").with_arguments(
                serde_json::json!({
                    "connection_id": connection_id.to_string(),
                    "database": database,
                    "collection": "items",
                    "documents": documents
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");
    assert!(format!("{result:?}").contains("documents must contain 1-100 items"), "{result:?}");

    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 1);
    assert_eq!(
        collection.find_one(doc! { "_id": 1 }).await.unwrap().unwrap().get_str("status").unwrap(),
        "unchanged"
    );

    client.cancel().await.unwrap();
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn direct_update_many_over_one_hundred_succeeds_without_history() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.db_name("mcp_direct");
    let collection = mongo.client.database(&database).collection::<Document>("items");
    collection
        .insert_many((0..150).map(|index| doc! { "_id": index, "active": false }))
        .await
        .unwrap();
    let connection_id = uuid::Uuid::new_v4();
    let (client, handle) = mcp_client(&mongo, connection_id, true, false).await;
    let arguments = serde_json::json!({
        "connection_id": connection_id.to_string(),
        "database": database,
        "collection": "items",
        "filter": {},
        "update": { "$set": { "active": true } },
        "many": true,
        "allow_all": true
    })
    .as_object()
    .unwrap()
    .clone();
    let result = client
        .call_tool(
            CallToolRequestParams::new("openmango_update_documents").with_arguments(arguments),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let content = result.structured_content.unwrap();
    assert_eq!(content["matched_count"], 150);
    assert_eq!(collection.count_documents(doc! { "active": true }).await.unwrap(), 150);
    client.cancel().await.unwrap();
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn direct_many_writes_require_allow_all_and_explicit_authority() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.db_name("mcp_allow_all");
    let collection = mongo.client.database(&database).collection::<Document>("items");
    collection.insert_many([doc! { "_id": 1 }, doc! { "_id": 2 }]).await.unwrap();
    let connection_id = uuid::Uuid::new_v4();
    let (client, handle) = mcp_client(&mongo, connection_id, true, false).await;
    let arguments = serde_json::json!({
        "connection_id": connection_id.to_string(),
        "database": database,
        "collection": "items",
        "filter": {},
        "many": true
    })
    .as_object()
    .unwrap()
    .clone();
    let result = client
        .call_tool(
            CallToolRequestParams::new("openmango_delete_documents").with_arguments(arguments),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 2);
    client.cancel().await.unwrap();
    handle.shutdown().await.unwrap();

    for (writable, read_only, expected) in
        [(false, false, "Agent writes are not enabled"), (true, true, "read-only")]
    {
        let (client, handle) = mcp_client(&mongo, connection_id, writable, read_only).await;
        let arguments = serde_json::json!({
            "connection_id": connection_id.to_string(),
            "database": mongo.db_name("mcp_allow_all"),
            "collection": "items",
            "documents": [{ "_id": uuid::Uuid::new_v4().to_string() }]
        })
        .as_object()
        .unwrap()
        .clone();
        let result = client
            .call_tool(
                CallToolRequestParams::new("openmango_insert_documents").with_arguments(arguments),
            )
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(format!("{result:?}").contains(expected));
        client.cancel().await.unwrap();
        handle.shutdown().await.unwrap();
    }
}
