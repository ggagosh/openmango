//! Aggregation pipeline operations.

use std::time::Duration;

use futures::TryStreamExt;
use mongodb::Client;
use mongodb::bson::{Document, doc};

use crate::connection::ConnectionManager;
use crate::connection::types::AggregatePipelineError;

pub async fn aggregate_pipeline_async(
    client: &Client,
    database: &str,
    collection: &str,
    mut pipeline: Vec<Document>,
    limit: Option<i64>,
    append_limit: bool,
    max_time: Option<Duration>,
) -> crate::error::Result<Vec<Document>> {
    if append_limit
        && let Some(limit) = limit
        && limit > 0
    {
        pipeline.push(doc! { "$limit": limit });
    }

    let coll = client.database(database).collection::<Document>(collection);
    let mut aggregate = coll.aggregate(pipeline);
    if let Some(max_time) = max_time {
        aggregate = aggregate.max_time(max_time);
    }
    Ok(aggregate.await?.try_collect().await?)
}

impl ConnectionManager {
    /// Run an aggregation pipeline for a collection with abort support (runs in Tokio runtime)
    #[allow(clippy::too_many_arguments)]
    pub fn aggregate_pipeline_abortable(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        pipeline: Vec<Document>,
        limit: Option<i64>,
        append_limit: bool,
        abort_registration: futures::future::AbortRegistration,
    ) -> std::result::Result<Vec<Document>, AggregatePipelineError> {
        use futures::future::Abortable;

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let fut = aggregate_pipeline_async(
                &client,
                &database,
                &collection,
                pipeline,
                limit,
                append_limit,
                None,
            );
            match Abortable::new(fut, abort_registration).await {
                Ok(result) => result.map_err(AggregatePipelineError::from),
                Err(_aborted) => Err(AggregatePipelineError::Aborted),
            }
        })
    }
}
