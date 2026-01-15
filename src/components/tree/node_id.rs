//! Type-safe tree node identifiers for the sidebar tree.

use gpui::{ElementId, SharedString};
use uuid::Uuid;

/// Type-safe identifier for nodes in the sidebar tree.
/// Replaces fragile string parsing with colon separators.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TreeNodeId {
    /// A MongoDB connection node
    Connection(Uuid),
    /// A database within a connection
    Database { connection: Uuid, database: String },
    /// A collection within a database
    Collection { connection: Uuid, database: String, collection: String },
}

impl TreeNodeId {
    /// Create a connection node ID
    pub fn connection(id: Uuid) -> Self {
        Self::Connection(id)
    }

    /// Create a database node ID
    pub fn database(connection: Uuid, database: impl Into<String>) -> Self {
        Self::Database { connection, database: database.into() }
    }

    /// Create a collection node ID
    pub fn collection(
        connection: Uuid,
        database: impl Into<String>,
        collection: impl Into<String>,
    ) -> Self {
        Self::Collection { connection, database: database.into(), collection: collection.into() }
    }

    /// Check if this is a connection node
    pub fn is_connection(&self) -> bool {
        matches!(self, Self::Connection(_))
    }

    /// Check if this is a database node
    pub fn is_database(&self) -> bool {
        matches!(self, Self::Database { .. })
    }

    /// Check if this is a collection node
    pub fn is_collection(&self) -> bool {
        matches!(self, Self::Collection { .. })
    }

    /// Get the connection UUID
    pub fn connection_id(&self) -> Uuid {
        match self {
            Self::Connection(id) => *id,
            Self::Database { connection, .. } => *connection,
            Self::Collection { connection, .. } => *connection,
        }
    }

    /// Get the database name if this is a database or collection node
    pub fn database_name(&self) -> Option<&str> {
        match self {
            Self::Connection(_) => None,
            Self::Database { database, .. } => Some(database),
            Self::Collection { database, .. } => Some(database),
        }
    }

    /// Get the collection name if this is a collection node
    pub fn collection_name(&self) -> Option<&str> {
        match self {
            Self::Collection { collection, .. } => Some(collection),
            _ => None,
        }
    }

    /// Convert to a string representation for use as tree item ID.
    /// Format: "conn:{uuid}" | "db:{uuid}:{database}" | "col:{uuid}:{database}:{collection}"
    pub fn to_tree_id(&self) -> String {
        match self {
            Self::Connection(id) => format!("conn:{}", id),
            Self::Database { connection, database } => format!("db:{}:{}", connection, database),
            Self::Collection { connection, database, collection } => {
                format!("col:{}:{}:{}", connection, database, collection)
            }
        }
    }

    /// Parse from a tree item ID string.
    /// Returns None if the format is invalid.
    pub fn from_tree_id(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.splitn(4, ':').collect();

        match parts.as_slice() {
            ["conn", uuid_str] => {
                let id = Uuid::parse_str(uuid_str).ok()?;
                Some(Self::Connection(id))
            }
            ["db", uuid_str, database] => {
                let id = Uuid::parse_str(uuid_str).ok()?;
                Some(Self::Database { connection: id, database: (*database).to_string() })
            }
            ["col", uuid_str, database, collection] => {
                let id = Uuid::parse_str(uuid_str).ok()?;
                Some(Self::Collection {
                    connection: id,
                    database: (*database).to_string(),
                    collection: (*collection).to_string(),
                })
            }
            _ => None,
        }
    }
}

impl From<TreeNodeId> for ElementId {
    fn from(id: TreeNodeId) -> Self {
        ElementId::Name(SharedString::from(id.to_tree_id()))
    }
}

impl From<&TreeNodeId> for ElementId {
    fn from(id: &TreeNodeId) -> Self {
        ElementId::Name(SharedString::from(id.to_tree_id()))
    }
}

impl std::fmt::Display for TreeNodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_tree_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let uuid = Uuid::new_v4();

        let conn = TreeNodeId::connection(uuid);
        assert_eq!(TreeNodeId::from_tree_id(&conn.to_tree_id()), Some(conn.clone()));

        let db = TreeNodeId::database(uuid, "mydb");
        assert_eq!(TreeNodeId::from_tree_id(&db.to_tree_id()), Some(db.clone()));

        let col = TreeNodeId::collection(uuid, "mydb", "mycol");
        assert_eq!(TreeNodeId::from_tree_id(&col.to_tree_id()), Some(col.clone()));
    }
}
