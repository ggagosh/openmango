//! Common test utilities and fixtures for integration tests using Testcontainers.
//!
//! A single MongoDB 7.0 container is shared per test binary (Rust compiles each
//! `tests/*.rs` file as a separate binary). Per-test isolation is achieved by
//! namespacing every database name with a short UUID suffix.
//!
//! The container runs on a dedicated background thread with its own tokio runtime,
//! avoiding "runtime shutdown" errors that occur when `#[tokio::test]` runtimes
//! (one per test function) are torn down independently.
//!
//! An `atexit` hook ensures the container is removed when the process exits.

#![allow(dead_code)]

pub mod fixtures;

use mongodb::bson::{Document, doc};
use mongodb::{Client, options::ClientOptions};
use std::sync::OnceLock;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;

/// Connection info for the shared container.
struct SharedContainer {
    connection_string: String,
}

static SHARED: OnceLock<SharedContainer> = OnceLock::new();

/// Docker container ID — stored globally so the `atexit` handler can remove it.
static CONTAINER_ID: OnceLock<String> = OnceLock::new();

unsafe extern "C" {
    fn atexit(f: extern "C" fn()) -> i32;
}

/// Called by the C runtime on process exit. Forcibly removes the shared container.
extern "C" fn remove_container() {
    if let Some(id) = CONTAINER_ID.get() {
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", id])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// Initialize the shared container (called once per test binary).
///
/// Spawns a background thread with its own tokio runtime so the container
/// outlives every `#[tokio::test]` runtime in the binary.
fn get_or_init_shared() -> &'static SharedContainer {
    SHARED.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create container runtime");

            rt.block_on(async {
                let container = Mongo::default()
                    .with_tag("7.0")
                    .start()
                    .await
                    .expect("Failed to start MongoDB container");

                // Store container ID for the atexit cleanup hook.
                let _ = CONTAINER_ID.set(container.id().to_string());
                unsafe {
                    atexit(remove_container);
                }

                let host = container.get_host().await.expect("Failed to get host");
                let port = container.get_host_port_ipv4(27017).await.expect("Failed to get port");
                let connection_string = format!("mongodb://{}:{}", host, port);

                // Readiness probe
                let opts = ClientOptions::parse(&connection_string).await.expect("Failed to parse");
                let probe = Client::with_options(opts).expect("Failed to create probe client");
                for _ in 0..30 {
                    if probe.list_database_names().await.is_ok() {
                        break;
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
                drop(probe);

                tx.send(connection_string).expect("Failed to send connection string");

                // Park forever — keeps the container alive until the process exits.
                std::future::pending::<()>().await;
            });
        });

        SharedContainer {
            connection_string: rx.recv().expect("Failed to receive connection string"),
        }
    })
}

/// A lightweight handle to the shared MongoDB container with per-test isolation.
///
/// Each handle gets a unique `test_id` so that `database("foo")` returns a
/// database named `foo_{test_id}`, preventing cross-test interference.
pub struct MongoTestContainer {
    pub client: Client,
    pub connection_string: String,
    test_id: String,
}

impl MongoTestContainer {
    /// Get a handle to the shared MongoDB container with a unique test namespace.
    ///
    /// Each call creates a fresh `Client` on the caller's tokio runtime,
    /// avoiding cross-runtime issues.
    pub async fn start() -> Self {
        let shared = get_or_init_shared();

        let client_options = ClientOptions::parse(&shared.connection_string)
            .await
            .expect("Failed to parse connection string");
        let client = Client::with_options(client_options).expect("Failed to create client");

        // Use first 8 chars of UUID v4 as a short, unique namespace suffix.
        let test_id = uuid::Uuid::new_v4().to_string()[..8].to_string();

        Self { client, connection_string: shared.connection_string.clone(), test_id }
    }

    /// Return the namespaced database name for this test.
    ///
    /// Use this when you need the raw name string (e.g. in `$out` pipelines or
    /// assertions against `dbStats`).
    pub fn db_name(&self, name: &str) -> String {
        format!("{}_{}", name, self.test_id)
    }

    /// Get a database for testing (automatically namespaced).
    pub fn database(&self, name: &str) -> mongodb::Database {
        self.client.database(&self.db_name(name))
    }

    /// Get a collection for testing (automatically namespaced).
    pub fn collection<T: Send + Sync>(&self, db: &str, collection: &str) -> mongodb::Collection<T> {
        self.database(db).collection(collection)
    }
}

/// Create a simple test document with common fields.
pub fn test_document(name: &str) -> Document {
    doc! {
        "name": name,
        "value": 42,
        "active": true,
    }
}

/// Create a test document with an explicit _id field.
pub fn test_document_with_id(id: &str, name: &str) -> Document {
    doc! {
        "_id": id,
        "name": name,
        "value": 42,
        "active": true,
    }
}
