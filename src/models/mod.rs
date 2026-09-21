// Data structures and types
#![allow(unused_imports)]

pub mod connection;
mod tree_node_id;

pub use connection::{
    ActiveConnection, CollectionDetail, ConnectionColor, ConnectionEnvironment,
    ConnectionRuntimeMeta, ConnectionWriteIdentity, ProxyConfig, ProxyKind, SavedConnection,
    SshAuth, SshConfig, is_system_collection,
};
pub use tree_node_id::TreeNodeId;
