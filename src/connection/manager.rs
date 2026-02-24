//! Core ConnectionManager struct and basic connection methods.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::Duration;

use mongodb::Client;
use mongodb::bson::doc;
use mongodb::results::CollectionSpecification;
use parking_lot::Mutex;
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::connection::tunnel::{SshTunnelHandle, start_ssh_tunnel};
use crate::error::{Error, Result};
use crate::models::{ConnectionRuntimeMeta, ProxyConfig, ProxyKind, SavedConnection};

const SSH_PROXY_CONFLICT_ERROR: &str = "SSH tunnel and SOCKS5 proxy cannot be enabled together yet";

/// Manages MongoDB client connections with cached runtime resources.
pub struct ConnectionManager {
    /// Tokio runtime for MongoDB async operations
    pub(crate) runtime: Runtime,
    /// Active SSH tunnel handles by connection id
    ssh_tunnels: Mutex<HashMap<Uuid, SshTunnelHandle>>,
}

impl ConnectionManager {
    /// Create a new connection manager
    pub fn new() -> Self {
        let runtime = Runtime::new().expect("Failed to create Tokio runtime");
        Self { runtime, ssh_tunnels: Mutex::new(HashMap::new()) }
    }

    /// Get a handle to the Tokio runtime for spawning parallel tasks
    pub fn runtime_handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    /// Connect to MongoDB using the saved connection config (runs in Tokio runtime).
    ///
    /// This unmanaged connect path is for legacy callers and does not preserve SSH tunnels.
    /// Use `connect_managed` for persisted active connections.
    pub fn connect(&self, config: &SavedConnection) -> Result<Client> {
        if config.ssh.as_ref().is_some_and(|ssh| ssh.enabled) {
            return Err(Error::Parse(
                "SSH connections require managed connect context".to_string(),
            ));
        }
        let (client, _runtime_meta, _tunnel) = self.connect_prepared(config)?;
        Ok(client)
    }

    /// Connect with runtime resource ownership (SSH tunnel lifecycle bound to connection id).
    pub fn connect_managed(
        &self,
        connection_id: Uuid,
        config: &SavedConnection,
    ) -> Result<(Client, ConnectionRuntimeMeta)> {
        self.stop_tunnel(connection_id);
        let (client, runtime_meta, tunnel) = self.connect_prepared(config)?;
        if let Some(tunnel) = tunnel {
            self.ssh_tunnels.lock().insert(connection_id, tunnel);
        }
        Ok((client, runtime_meta))
    }

    /// Disconnect runtime resources for a connection.
    pub fn disconnect(&self, connection_id: Uuid) {
        self.stop_tunnel(connection_id);
    }

    /// Test connectivity with a timeout (runs in Tokio runtime).
    ///
    /// SSH tunnel (if configured) is created only for the test and always cleaned up.
    pub fn test_connection(&self, config: &SavedConnection, timeout: Duration) -> Result<()> {
        self.test_connection_with_progress(config, timeout, |_| {})
    }

    /// Test connectivity while streaming progress step labels.
    pub fn test_connection_with_progress<F>(
        &self,
        config: &SavedConnection,
        timeout: Duration,
        on_progress: F,
    ) -> Result<()>
    where
        F: FnMut(String),
    {
        self.test_connection_internal(config, timeout, on_progress)
    }

    fn test_connection_internal<F>(
        &self,
        config: &SavedConnection,
        timeout: Duration,
        mut on_progress: F,
    ) -> Result<()>
    where
        F: FnMut(String),
    {
        let mut steps = vec!["Preparing transport settings".to_string()];
        on_progress("Preparing transport settings".to_string());

        let (effective_uri, runtime_meta, tunnel) = match self.prepare_connection(config) {
            Ok(prepared) => prepared,
            Err(err) => return Err(annotate_connection_error(err, &steps, None)),
        };

        if runtime_meta.ssh_tunnel_active {
            let endpoint = runtime_meta
                .ssh_local_endpoint
                .as_deref()
                .unwrap_or("local tunnel endpoint unavailable");
            let step = format!("SSH tunnel established at {endpoint}");
            steps.push(step.clone());
            on_progress(step);
            if let Some(target_hosts) = uri_hosts_for_trace(&config.uri) {
                let step = format!("MongoDB target via tunnel: {target_hosts}");
                steps.push(step.clone());
                on_progress(step);
            }
        } else {
            let step = "SSH tunnel disabled".to_string();
            steps.push(step.clone());
            on_progress(step);
        }

        if runtime_meta.proxy_active {
            let step = "SOCKS5 proxy settings applied".to_string();
            steps.push(step.clone());
            on_progress(step);
        } else {
            let step = "Proxy disabled".to_string();
            steps.push(step.clone());
            on_progress(step);
        }

        let phase_timeout = if runtime_meta.ssh_tunnel_active || runtime_meta.proxy_active {
            timeout.max(Duration::from_secs(15))
        } else {
            timeout
        };
        let step = format!("Per-step timeout: {}s", phase_timeout.as_secs());
        steps.push(step.clone());
        on_progress(step);

        let step = "Creating MongoDB client".to_string();
        steps.push(step.clone());
        on_progress(step);
        let client = match self.runtime.block_on(async {
            tokio::time::timeout(phase_timeout, Client::with_uri_str(&effective_uri)).await
        }) {
            Ok(Ok(client)) => {
                let step = "MongoDB client created".to_string();
                steps.push(step.clone());
                on_progress(step);
                client
            }
            Ok(Err(err)) => {
                drop(tunnel);
                return Err(annotate_connection_error(
                    Error::from(err),
                    &steps,
                    Some(&runtime_meta),
                ));
            }
            Err(_) => {
                drop(tunnel);
                return Err(annotate_connection_error(
                    Error::Timeout(
                        "Connection timed out while creating MongoDB client".to_string(),
                    ),
                    &steps,
                    Some(&runtime_meta),
                ));
            }
        };

        let step = "Running admin ping".to_string();
        steps.push(step.clone());
        on_progress(step);
        let ping_outcome = self.runtime.block_on(async {
            tokio::time::timeout(
                phase_timeout,
                client.database("admin").run_command(doc! { "ping": 1 }),
            )
            .await
        });

        drop(tunnel);

        match ping_outcome {
            Ok(Ok(_)) => {
                on_progress("Connection test completed".to_string());
                Ok(())
            }
            Ok(Err(err)) => {
                Err(annotate_connection_error(Error::from(err), &steps, Some(&runtime_meta)))
            }
            Err(_) => Err(annotate_connection_error(
                Error::Timeout("Connection timed out while running ping".to_string()),
                &steps,
                Some(&runtime_meta),
            )),
        }
    }

    /// List databases for a connected client (runs in Tokio runtime)
    pub fn list_databases(&self, client: &Client) -> Result<Vec<String>> {
        let client = client.clone();
        self.runtime.block_on(async {
            let mut databases = client.list_database_names().await?;
            databases.sort_unstable_by_key(|name| name.to_lowercase());
            Ok(databases)
        })
    }

    /// List collections in a database (runs in Tokio runtime)
    pub fn list_collections(&self, client: &Client, database: &str) -> Result<Vec<String>> {
        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let mut collections = db.list_collection_names().await?;
            collections.sort_unstable_by_key(|name| name.to_lowercase());
            Ok(collections)
        })
    }

    /// List collection specs in a database (runs in Tokio runtime)
    pub fn list_collection_specs(
        &self,
        client: &Client,
        database: &str,
    ) -> Result<Vec<CollectionSpecification>> {
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let cursor = db.list_collections().await?;
            let mut specs: Vec<CollectionSpecification> = cursor.try_collect().await?;
            specs.sort_unstable_by_key(|spec| spec.name.to_lowercase());
            Ok(specs)
        })
    }

    /// Create a collection in a database (runs in Tokio runtime)
    pub fn create_collection(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            db.create_collection(&collection).await?;
            Ok(())
        })
    }

    /// Drop a collection in a database (runs in Tokio runtime)
    pub fn drop_collection(&self, client: &Client, database: &str, collection: &str) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            coll.drop().await?;
            Ok(())
        })
    }

    /// Rename a collection in a database (runs in Tokio runtime)
    pub fn rename_collection(
        &self,
        client: &Client,
        database: &str,
        from: &str,
        to: &str,
    ) -> Result<()> {
        let client = client.clone();
        let from = format!("{database}.{from}");
        let to = format!("{database}.{to}");
        self.runtime.block_on(async {
            let admin = client.database("admin");
            admin
                .run_command(doc! { "renameCollection": from, "to": to, "dropTarget": false })
                .await?;
            Ok(())
        })
    }

    /// Drop a database (runs in Tokio runtime)
    pub fn drop_database(&self, client: &Client, database: &str) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            db.drop().await?;
            Ok(())
        })
    }

    /// List all collection names in a database (runs in Tokio runtime).
    pub fn list_collection_names(&self, client: &Client, database: &str) -> Result<Vec<String>> {
        let client = client.clone();
        let database = database.to_string();

        self.runtime.block_on(async {
            let db = client.database(&database);
            let names = db.list_collection_names().await?;
            Ok(names)
        })
    }

    fn connect_prepared(
        &self,
        config: &SavedConnection,
    ) -> Result<(Client, ConnectionRuntimeMeta, Option<SshTunnelHandle>)> {
        let (effective_uri, runtime_meta, tunnel) = self.prepare_connection(config)?;
        let timeout = Duration::from_secs(30);

        let client = self
            .runtime
            .block_on(async {
                let client = tokio::time::timeout(timeout, Client::with_uri_str(&effective_uri))
                    .await
                    .map_err(|_| {
                        Error::Timeout(
                            "Connection timed out while creating MongoDB client".to_string(),
                        )
                    })?
                    .map_err(Error::from)?;
                tokio::time::timeout(
                    timeout,
                    client.database("admin").run_command(doc! { "ping": 1 }),
                )
                .await
                .map_err(|_| Error::Timeout("Connection timed out while running ping".to_string()))?
                .map_err(Error::from)?;
                Ok::<Client, Error>(client)
            })
            .map_err(|err| annotate_connection_error(err, &[], Some(&runtime_meta)))?;

        Ok((client, runtime_meta, tunnel))
    }

    fn prepare_connection(
        &self,
        config: &SavedConnection,
    ) -> Result<(String, ConnectionRuntimeMeta, Option<SshTunnelHandle>)> {
        if transport_combo_enabled(config) {
            return Err(Error::Parse(SSH_PROXY_CONFLICT_ERROR.to_string()));
        }

        let mut effective_uri = config.uri.clone();
        let mut runtime_meta = ConnectionRuntimeMeta::default();
        let mut tunnel_handle = None;

        if let Some(ssh) = config.ssh.as_ref().filter(|ssh| ssh.enabled) {
            let tunnel = start_ssh_tunnel(ssh)?;
            effective_uri =
                set_query_param(&effective_uri, "proxyHost", Some(tunnel.local_host.clone()))?;
            effective_uri =
                set_query_param(&effective_uri, "proxyPort", Some(tunnel.local_port.to_string()))?;
            effective_uri =
                set_query_param(&effective_uri, "directConnection", Some("true".to_string()))?;
            // In SSH mode we proxy a specific endpoint; keeping replicaSet can force
            // server selection to wait for a primary that may not be reachable.
            if effective_uri.to_ascii_lowercase().contains("replicaset") {
                log::debug!(
                    "Removed replicaSet from URI (incompatible with directConnection over SSH)"
                );
            }
            effective_uri = set_query_param(&effective_uri, "replicaSet", None)?;
            runtime_meta.ssh_tunnel_active = true;
            runtime_meta.ssh_local_endpoint = Some(tunnel.local_endpoint());
            tunnel_handle = Some(tunnel);
        }

        if let Some(proxy) = config.proxy.as_ref().filter(|proxy| proxy.enabled) {
            validate_proxy_config(proxy)?;
            effective_uri = apply_proxy_to_uri(&effective_uri, proxy)?;
            runtime_meta.proxy_active = true;
        }

        log::debug!("effective URI: {}", redact_uri_password(&effective_uri));

        Ok((effective_uri, runtime_meta, tunnel_handle))
    }

    fn stop_tunnel(&self, connection_id: Uuid) {
        if let Some(mut tunnel) = self.ssh_tunnels.lock().remove(&connection_id) {
            tunnel.stop();
        }
    }
}

impl Drop for ConnectionManager {
    fn drop(&mut self) {
        for (_id, mut tunnel) in self.ssh_tunnels.get_mut().drain() {
            tunnel.stop();
        }
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_proxy_config(proxy: &ProxyConfig) -> Result<()> {
    if !matches!(proxy.kind, ProxyKind::Socks5) {
        return Err(Error::Parse("Only SOCKS5 proxy is supported".to_string()));
    }
    if proxy.host.trim().is_empty() {
        return Err(Error::Parse("SOCKS5 proxy host is required".to_string()));
    }
    if proxy.port == 0 {
        return Err(Error::Parse("SOCKS5 proxy port must be greater than 0".to_string()));
    }
    Ok(())
}

fn apply_proxy_to_uri(uri: &str, proxy: &ProxyConfig) -> Result<String> {
    let mut uri = set_query_param(uri, "proxyHost", Some(proxy.host.trim().to_string()))?;
    uri = set_query_param(&uri, "proxyPort", Some(proxy.port.to_string()))?;
    uri = set_query_param(&uri, "proxyUsername", proxy.username.clone())?;
    uri = set_query_param(&uri, "proxyPassword", proxy.password.clone())?;
    Ok(uri)
}

fn set_query_param(uri: &str, key: &str, value: Option<String>) -> Result<String> {
    let mut parts = parse_uri_parts(uri)?;
    parts.query.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
    if let Some(value) = value
        && !value.trim().is_empty()
    {
        parts.query.push((key.to_string(), percent_encode_query_value(&value)));
    }
    Ok(parts.to_uri())
}

#[derive(Debug, Clone)]
struct UriParts {
    scheme: String,
    authority: String,
    path: Option<String>,
    query: Vec<(String, String)>,
}

impl UriParts {
    fn to_uri(&self) -> String {
        let mut out = format!("{}://{}", self.scheme, self.authority);
        if let Some(path) = &self.path {
            out.push('/');
            out.push_str(path);
        }
        if !self.query.is_empty() {
            out.push('?');
            for (index, (key, value)) in self.query.iter().enumerate() {
                out.push_str(key);
                out.push('=');
                out.push_str(value);
                if index + 1 < self.query.len() {
                    out.push('&');
                }
            }
        }
        out
    }
}

fn parse_uri_parts(uri: &str) -> Result<UriParts> {
    let trimmed = uri.trim();
    let (scheme, rest) = trimmed
        .split_once("://")
        .ok_or_else(|| Error::Parse("URI must include scheme".to_string()))?;
    let (base, query_string) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, path) = match base.split_once('/') {
        Some((authority, path)) => (authority.to_string(), Some(path.to_string())),
        None => (base.to_string(), None),
    };

    if authority.trim().is_empty() {
        return Err(Error::Parse("URI is missing host".to_string()));
    }

    let mut query = Vec::new();
    if !query_string.trim().is_empty() {
        for pair in query_string.split('&') {
            if pair.trim().is_empty() {
                continue;
            }
            if let Some((key, value)) = pair.split_once('=') {
                query.push((key.to_string(), value.to_string()));
            } else {
                query.push((pair.to_string(), String::new()));
            }
        }
    }

    Ok(UriParts { scheme: scheme.to_string(), authority, path, query })
}

fn uri_hosts_for_trace(uri: &str) -> Option<String> {
    let parts = parse_uri_parts(uri).ok()?;
    let hosts = parts
        .authority
        .rsplit_once('@')
        .map(|(_, hosts)| hosts.to_string())
        .unwrap_or(parts.authority);
    Some(hosts)
}

fn transport_combo_enabled(config: &SavedConnection) -> bool {
    config.ssh.as_ref().is_some_and(|ssh| ssh.enabled)
        && config.proxy.as_ref().is_some_and(|proxy| proxy.enabled)
}

fn redact_uri_password(uri: &str) -> String {
    let Some(parts) = parse_uri_parts(uri).ok() else {
        return "***".to_string();
    };
    // Mask the password portion of userinfo (user:pass@host)
    let authority = if let Some((userinfo, hosts)) = parts.authority.split_once('@') {
        if let Some((user, _password)) = userinfo.split_once(':') {
            format!("{user}:***@{hosts}")
        } else {
            parts.authority.clone()
        }
    } else {
        parts.authority.clone()
    };
    let redacted = UriParts { authority, ..parts };
    redacted.to_uri()
}

/// Percent-encode a query parameter value per RFC 3986 §2.1.
///
/// NOTE: This is only called for values that `set_query_param` *injects*
/// (proxyHost, proxyPort, etc.), never for values preserved from the user's
/// original URI.  If it were applied to already-encoded values (e.g. `p%40ss`)
/// it would double-encode the `%` to `%25`.
fn percent_encode_query_value(value: &str) -> String {
    fn is_unreserved(byte: u8) -> bool {
        matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~')
    }

    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if is_unreserved(*byte) {
            out.push(char::from(*byte));
        } else {
            out.push('%');
            let _ = write!(&mut out, "{byte:02X}");
        }
    }
    out
}

fn annotate_connection_error(
    err: Error,
    steps: &[String],
    runtime_meta: Option<&ConnectionRuntimeMeta>,
) -> Error {
    let mut message = err.to_string();

    if let Some(meta) = runtime_meta
        && let Some(hint) = connection_hint(&message, meta)
    {
        message.push_str("\n\nHint:\n");
        message.push_str(hint);
    }

    if !steps.is_empty() {
        message.push_str("\n\nTest trace:\n");
        for step in steps {
            message.push_str("- ");
            message.push_str(step);
            message.push('\n');
        }
    }

    Error::Parse(message.trim_end().to_string())
}

fn connection_hint(message: &str, runtime_meta: &ConnectionRuntimeMeta) -> Option<&'static str> {
    let lower = message.to_ascii_lowercase();

    if runtime_meta.ssh_tunnel_active
        && (lower.contains("server selection timeout")
            || lower.contains("no available servers")
            || lower.contains("timed out while running ping"))
    {
        return Some(
            "Tunnel is up, but MongoDB server selection did not complete. Most common causes are replica-set topology (no primary/secondary target) or unreachable advertised members. Use single-host URI + directConnection=true, remove replicaSet from URI, or set readPreference=secondaryPreferred for read-only access. Also verify the SSH host can reach MongoDB host:port.",
        );
    }

    if lower.contains("server selection timeout") {
        return Some(
            "Server selection timed out. Verify network reachability to MongoDB host:port and increase serverSelectionTimeoutMS if needed.",
        );
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{SSH_PROXY_CONFLICT_ERROR, set_query_param, transport_combo_enabled};
    use crate::error::Error;
    use crate::models::{ProxyConfig, ProxyKind, SavedConnection, SshAuth, SshConfig};

    #[test]
    fn set_query_param_percent_encodes_reserved_chars() {
        let uri = "mongodb://localhost:27017/?directConnection=true";
        let updated = set_query_param(uri, "proxyPassword", Some("p@ss:word/with?chars&=".into()))
            .expect("query parameter should be set");
        assert!(updated.contains("proxyPassword=p%40ss%3Aword%2Fwith%3Fchars%26%3D"));
    }

    #[test]
    fn transport_combo_enabled_detects_ssh_and_proxy() {
        let mut saved =
            SavedConnection::new("combo".to_string(), "mongodb://localhost:27017".into());
        saved.ssh = Some(SshConfig {
            enabled: true,
            host: "bastion".to_string(),
            port: 22,
            username: "root".to_string(),
            auth: SshAuth::Password,
            password: Some("secret".to_string()),
            identity_file: None,
            identity_passphrase: None,
            strict_host_key_checking: false,
            local_bind_host: "127.0.0.1".to_string(),
        });
        saved.proxy = Some(ProxyConfig {
            enabled: true,
            kind: ProxyKind::Socks5,
            host: "127.0.0.1".to_string(),
            port: 1080,
            username: None,
            password: None,
        });

        assert!(transport_combo_enabled(&saved));
    }

    #[test]
    fn conflict_error_message_is_stable() {
        let err = Error::Parse(SSH_PROXY_CONFLICT_ERROR.to_string());
        assert!(err.to_string().contains("cannot be enabled together"));
    }
}
