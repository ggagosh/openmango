//! Aggregation pipeline operations.

use mongodb::Client;
use mongodb::bson::{Document, doc};

use crate::connection::ConnectionManager;
use crate::connection::types::AggregatePipelineError;

impl ConnectionManager {
    /// Run an aggregation pipeline for a collection with abort support (runs in Tokio runtime)
    #[allow(clippy::too_many_arguments)]
    pub fn aggregate_pipeline_abortable(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        mut pipeline: Vec<Document>,
        limit: Option<i64>,
        append_limit: bool,
        abort_registration: futures::future::AbortRegistration,
    ) -> std::result::Result<Vec<Document>, AggregatePipelineError> {
        use futures::TryStreamExt;
        use futures::future::Abortable;

        if append_limit
            && let Some(limit) = limit
            && limit > 0
        {
            pipeline.push(doc! { "$limit": limit });
        }

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(&collection);
            let fut = async move {
                let cursor = coll.aggregate(pipeline).await?;
                let docs: Vec<Document> = cursor.try_collect().await?;
                Ok::<_, crate::error::Error>(docs)
            };
            match Abortable::new(fut, abort_registration).await {
                Ok(result) => result.map_err(AggregatePipelineError::from),
                Err(_aborted) => Err(AggregatePipelineError::Aborted),
            }
        })
    }
}
