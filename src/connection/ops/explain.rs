//! Explain command operations for find and aggregation.

use mongodb::Client;
use mongodb::bson::{Document, doc};

use crate::connection::ConnectionManager;
use crate::error::Result;

pub struct ExplainFindRequest {
    pub database: String,
    pub collection: String,
    pub filter: Option<Document>,
    pub sort: Option<Document>,
    pub projection: Option<Document>,
    pub verbosity: String,
}

impl ConnectionManager {
    /// Run explain for a `find` command using selected verbosity.
    pub fn explain_find(&self, client: &Client, request: ExplainFindRequest) -> Result<Document> {
        let client = client.clone();
        let ExplainFindRequest { database, collection, filter, sort, projection, verbosity } =
            request;
        let filter = filter.unwrap_or_default();
        let sort = sort.unwrap_or_default();
        let projection = projection.unwrap_or_default();

        self.runtime.block_on(async move {
            let db = client.database(&database);
            let mut find_cmd = doc! { "find": collection };
            if !filter.is_empty() {
                find_cmd.insert("filter", filter);
            }
            if !sort.is_empty() {
                find_cmd.insert("sort", sort);
            }
            if !projection.is_empty() {
                find_cmd.insert("projection", projection);
            }

            let command = doc! {
                "explain": find_cmd,
                "verbosity": verbosity,
            };
            let explain = db.run_command(command).await?;
            Ok(explain)
        })
    }

    /// Run explain for an `aggregate` command using selected verbosity.
    pub fn explain_aggregation(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        pipeline: Vec<Document>,
        verbosity: &str,
    ) -> Result<Document> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let verbosity = verbosity.to_string();

        self.runtime.block_on(async move {
            let db = client.database(&database);
            let command = doc! {
                "explain": {
                    "aggregate": collection,
                    "pipeline": pipeline,
                    "cursor": {}
                },
                "verbosity": verbosity
            };
            let explain = db.run_command(command).await?;
            Ok(explain)
        })
    }
}
