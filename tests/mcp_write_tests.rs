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
