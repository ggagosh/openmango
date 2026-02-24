// Connection configuration models

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SshAuth {
    #[default]
    Password,
    IdentityFile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub auth: SshAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_passphrase: Option<String>,
    #[serde(default = "default_strict_host_key_checking")]
    pub strict_host_key_checking: bool,
    #[serde(default = "default_local_bind_host")]
    pub local_bind_host: String,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: default_ssh_port(),
            username: String::new(),
            auth: SshAuth::default(),
            password: None,
            identity_file: None,
            identity_passphrase: None,
            strict_host_key_checking: default_strict_host_key_checking(),
            local_bind_host: default_local_bind_host(),
        }
    }
}

fn default_ssh_port() -> u16 {
    22
}

fn default_strict_host_key_checking() -> bool {
    true
}

fn default_local_bind_host() -> String {
    "127.0.0.1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProxyKind {
    #[default]
    Socks5,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub kind: ProxyKind,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_proxy_port")]
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: ProxyKind::default(),
            host: String::new(),
            port: default_proxy_port(),
            username: None,
            password: None,
        }
    }
}

fn default_proxy_port() -> u16 {
    1080
}

#[derive(Debug, Clone, Default)]
pub struct ConnectionRuntimeMeta {
    pub ssh_tunnel_active: bool,
    pub ssh_local_endpoint: Option<String>,
    pub proxy_active: bool,
}

/// A saved connection configuration (persisted to disk)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedConnection {
    pub id: Uuid,
    pub name: String,
    pub uri: String,
    pub last_connected: Option<DateTime<Utc>>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
}

impl SavedConnection {
    pub fn new(name: String, uri: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            uri,
            last_connected: None,
            read_only: false,
            ssh: None,
            proxy: None,
        }
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
    pub runtime_meta: ConnectionRuntimeMeta,
}
