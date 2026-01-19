// Connection configuration models

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A saved connection configuration (persisted to disk)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedConnection {
    pub id: Uuid,
    pub name: String,
    pub uri: String,
    pub last_connected: Option<DateTime<Utc>>,
    #[serde(default)]
    pub read_only: bool,
}

impl SavedConnection {
    pub fn new(name: String, uri: String) -> Self {
        Self { id: Uuid::new_v4(), name, uri, last_connected: None, read_only: false }
    }
}

/// An active connection (runtime only, not persisted)
#[derive(Clone)]
pub struct ActiveConnection {
    pub config: SavedConnection,
    pub client: mongodb::Client,
    pub databases: Vec<String>,
    /// Collections per database (db_name -> collection_names)
    pub collections: HashMap<String, Vec<String>>,
}
