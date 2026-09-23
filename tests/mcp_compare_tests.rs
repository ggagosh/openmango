//! Read-only compare tools over the real MCP transport, inline and as MCP tasks.

mod common;

use std::collections::HashMap;

use common::MongoTestContainer;
use mongodb::bson::{Document, doc};
use openmango::mcp::{McpBridge, McpConnection, McpServer, McpServerHandle};
use rmcp::ServiceExt as _;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ClientCapabilities, ClientInfo,
    GetTaskParams, Implementation, TaskPayload,
};
use rmcp::transport::{
    StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
};

type Client = rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>;

async fn mcp_client(
    mongo: &MongoTestContainer,
    connection: uuid::Uuid,
    tasks: bool,
) -> (Client, McpServerHandle) {
    let server = McpServer::new(McpBridge::fixed_with_clients(
        vec![McpConnection {
            id: connection,
            name: "Test".into(),
            environment: Some("Development".into()),
            protected: false,
            read_only: true,
            writable: false,
            connected: true,
            databases: vec![],
        }],
        HashMap::from([(connection, mongo.client.clone())]),
    ));
    let handle = McpServerHandle::start(server, "test-token".into()).await.unwrap();
    let headers = HashMap::from([(
        axum::http::HeaderName::from_static("authorization"),
        axum::http::HeaderValue::from_static("Bearer test-token"),
    )]);
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://{}/mcp", handle.addr()))
            .custom_headers(headers),
    );
    let info = if tasks {
        ClientInfo::new(
            ClientCapabilities::builder().enable_tasks().build(),
            Implementation::from_build_env(),
        )
    } else {
        ClientInfo::default()
    };
    (info.serve(transport).await.unwrap(), handle)
}

fn call(name: &'static str, arguments: serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name).with_arguments(arguments.as_object().unwrap().clone())
}

async fn seed(mongo: &MongoTestContainer, database: &str) {
    let db = mongo.client.database(database);
    let [left, right] = ["left", "right"].map(|name| db.collection::<Document>(name));
    left.insert_many([
        doc! {"_id": 1, "sku": "a", "price": 10, "tags": ["x", "y"]},
        doc! {"_id": 2, "sku": "b", "price": 20},
        doc! {"_id": 3, "sku": "c"},
    ])
    .await
    .unwrap();
    right
        .insert_many([
            doc! {"_id": 1, "sku": "a", "price": 12, "tags": ["y", "x"]},
            doc! {"_id": 2, "sku": "b", "price": 20},
            doc! {"_id": 4, "sku": "d"},
        ])
        .await
        .unwrap();
    db.collection::<Document>("same").insert_one(doc! {"_id": 1}).await.unwrap();
}

#[tokio::test]
async fn compare_tools_answer_inline_and_as_tasks() {
    let mongo = MongoTestContainer::start().await;
    let database = mongo.db_name("mcp_compare");
    let other = mongo.db_name("mcp_compare_other");
    seed(&mongo, &database).await;
    mongo
        .client
        .database(&other)
        .collection::<Document>("same")
        .insert_one(doc! {"_id": 1})
        .await
        .unwrap();
    let connection = uuid::Uuid::new_v4();
    let collections = serde_json::json!({
        "left_connection_id": connection.to_string(),
        "left_database": database,
        "left_collection": "left",
        "right_connection_id": connection.to_string(),
        "right_database": database,
        "right_collection": "right",
        "ignore_array_order": true,
    });

    // Without the tasks extension the call answers directly.
    let (client, handle) = mcp_client(&mongo, connection, false).await;
    let tools = client.list_all_tools().await.unwrap();
    for name in ["openmango_compare_collections", "openmango_compare_databases"] {
        let tool = tools.iter().find(|tool| tool.name == name).expect(name);
        assert_eq!(tool.annotations.as_ref().and_then(|a| a.read_only_hint), Some(true));
    }
    let result =
        client.call_tool(call("openmango_compare_collections", collections.clone())).await.unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let content = result.structured_content.unwrap();
    assert_eq!(content["complete"], true);
    let counts = &content["counts"];
    assert_eq!(
        [
            &counts["identical"],
            &counts["different"],
            &counts["minor"],
            &counts["only_left"],
            &counts["only_right"]
        ],
        [1, 1, 0, 1, 1].map(serde_json::Value::from).each_ref(),
        "{content}"
    );
    let different = content["differences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == "different")
        .unwrap();
    assert_eq!(different["key"], serde_json::json!({"$numberInt": "1"}));
    assert_eq!(different["changed_paths"], serde_json::json!(["price"]), "tags only moved");

    let databases = serde_json::json!({
        "left_connection_id": connection.to_string(),
        "left_database": database,
        "right_connection_id": connection.to_string(),
        "right_database": other,
    });
    let result = client.call_tool(call("openmango_compare_databases", databases)).await.unwrap();
    let content = result.structured_content.unwrap();
    assert_eq!(content["complete"], true, "{content}");
    assert_eq!(content["totals"]["identical"], 1);
    assert_eq!(content["totals"]["left_only"], 2);
    // Identical collections are counted, not listed.
    let names: Vec<_> = content["collections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["left", "right"]);
    client.cancel().await.unwrap();
    handle.shutdown().await.unwrap();

    // With it, the same call becomes a task to poll.
    let (client, handle) = mcp_client(&mongo, connection, true).await;
    let response =
        client.call_tool_once(call("openmango_compare_collections", collections)).await.unwrap();
    let CallToolResponse::Task(created) = response else {
        panic!("expected a task, got {response:?}")
    };
    let task = loop {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let info =
            client.peer().get_task(GetTaskParams::new(created.task.task_id.clone())).await.unwrap();
        if info.task.status().is_terminal() {
            break info.task;
        }
    };
    let TaskPayload::Completed { result } = task.payload else { panic!("{:?}", task.payload) };
    let result: CallToolResult = serde_json::from_value(serde_json::Value::Object(result)).unwrap();
    assert_eq!(result.structured_content.unwrap()["counts"]["different"], 1);
    // An id this grant does not own is an error, never another client's result.
    assert!(client.peer().get_task(GetTaskParams::new("not-a-task".to_string())).await.is_err());
    client.cancel().await.unwrap();
    handle.shutdown().await.unwrap();
}
