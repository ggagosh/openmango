//! Explain command operations for find and aggregation.

use std::time::Duration;

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

pub async fn explain_find_async(
    client: &Client,
    request: ExplainFindRequest,
    max_time: Duration,
) -> Result<Document> {
    let ExplainFindRequest { database, collection, filter, sort, projection, verbosity } = request;
    let mut find_cmd = doc! { "find": collection };
    if let Some(filter) = filter
        && !filter.is_empty()
    {
        find_cmd.insert("filter", filter);
    }
    if let Some(sort) = sort
        && !sort.is_empty()
    {
        find_cmd.insert("sort", sort);
    }
    if let Some(projection) = projection
        && !projection.is_empty()
    {
        find_cmd.insert("projection", projection);
    }
    let command = doc! {
        "explain": find_cmd,
        "verbosity": verbosity,
        "maxTimeMS": duration_ms(max_time),
    };
    Ok(client.database(&database).run_command(command).await?)
}

pub async fn explain_aggregation_async(
    client: &Client,
    database: &str,
    collection: &str,
    pipeline: Vec<Document>,
    verbosity: &str,
    max_time: Duration,
) -> Result<Document> {
    let command = doc! {
        "explain": {
            "aggregate": collection,
            "pipeline": pipeline,
            "cursor": {}
        },
        "verbosity": verbosity,
        "maxTimeMS": duration_ms(max_time),
    };
    Ok(client.database(database).run_command(command).await?)
}

fn duration_ms(duration: Duration) -> i64 {
    duration.as_millis().min(i64::MAX as u128) as i64
}

impl ConnectionManager {
    /// Run explain for a `find` command using selected verbosity.
    pub fn explain_find(&self, client: &Client, request: ExplainFindRequest) -> Result<Document> {
        self.runtime.block_on(explain_find_async(client, request, Duration::from_secs(30)))
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
        self.runtime.block_on(explain_aggregation_async(
            client,
            database,
            collection,
            pipeline,
            verbosity,
            Duration::from_secs(30),
        ))
    }
}
