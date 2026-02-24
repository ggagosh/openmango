//! Integration tests for SSH tunnel and SOCKS5 proxy connection flows.

mod common;

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use fast_socks5::server::{self, Socks5Socket};
use openmango::connection::ConnectionManager;
use openmango::error::Error as AppError;
use openmango::models::{ProxyConfig, ProxyKind, SavedConnection};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use uuid::Uuid;

use common::transport::{
    KnownHostsEnvGuard, TransportStack, endpoint_accepts_connections, generate_ed25519_keypair,
    wait_for_endpoint_closed, write_known_hosts_file,
};

struct LocalSocksProxy {
    port: u16,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

impl LocalSocksProxy {
    async fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("failed to bind local SOCKS5 listener");
        let port =
            listener.local_addr().expect("failed to read local SOCKS5 listener address").port();
        let config = Arc::new(server::Config::<server::DenyAuthentication>::default());
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    incoming = listener.accept() => {
                        let Ok((stream, _peer_addr)) = incoming else {
                            break;
                        };

                        let config = config.clone();
                        tokio::spawn(async move {
                            let mut socket = Socks5Socket::new(stream, config);
                            socket.set_reply_ip(IpAddr::V4(Ipv4Addr::LOCALHOST));
                            let _ = socket.upgrade_to_socks5().await;
                        });
                    }
                }
            }
        });

        Self { port, shutdown_tx: Some(shutdown_tx), handle }
    }

    fn config(&self) -> ProxyConfig {
        ProxyConfig {
            enabled: true,
            kind: ProxyKind::Socks5,
            host: "127.0.0.1".to_string(),
            port: self.port,
            username: None,
            password: None,
        }
    }
}

impl Drop for LocalSocksProxy {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.handle.abort();
    }
}

fn make_saved_connection(name: &str, uri: String) -> SavedConnection {
    SavedConnection::new(name.to_string(), uri)
}

fn init_logger() {
    let _ = env_logger::builder().is_test(true).try_init();
}

async fn run_blocking<T, F>(f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .expect("blocking task panicked while running connection-manager operation")
}

#[tokio::test]
async fn ssh_password_connect_managed_success_sets_runtime_meta() {
    init_logger();
    let stack = TransportStack::start().await.expect("failed to start transport stack");
    let mut saved = make_saved_connection("ssh-password", stack.mongo_uri_for_ssh());
    saved.ssh = Some(stack.ssh_password_config(false));

    let runtime_meta = run_blocking(move || -> Result<_, AppError> {
        let manager = ConnectionManager::new();
        let connection_id = Uuid::new_v4();
        let (_client, runtime_meta) = manager.connect_managed(connection_id, &saved)?;
        manager.disconnect(connection_id);
        Ok(runtime_meta)
    })
    .await
    .expect("managed SSH connection should succeed");

    assert!(runtime_meta.ssh_tunnel_active);
    assert!(runtime_meta.ssh_local_endpoint.is_some());
    assert!(!runtime_meta.proxy_active);
}

#[tokio::test]
async fn ssh_identity_file_connect_managed_success() {
    init_logger();
    let stack = TransportStack::start().await.expect("failed to start transport stack");
    let key_dir = TempDir::new().expect("failed to create temp dir for ssh keys");
    let (private_key, public_key) =
        generate_ed25519_keypair(&key_dir).expect("failed to generate test identity key pair");
    stack
        .install_authorized_key(&public_key)
        .await
        .expect("failed to install public key into sshd");

    let mut saved = make_saved_connection("ssh-identity", stack.mongo_uri_for_ssh());
    saved.ssh = Some(stack.ssh_identity_config(private_key, false));

    let runtime_meta = run_blocking(move || -> Result<_, AppError> {
        let manager = ConnectionManager::new();
        let connection_id = Uuid::new_v4();
        let (_client, runtime_meta) = manager.connect_managed(connection_id, &saved)?;
        manager.disconnect(connection_id);
        Ok(runtime_meta)
    })
    .await
    .expect("managed SSH identity-file connection should succeed");

    assert!(runtime_meta.ssh_tunnel_active);
    assert!(runtime_meta.ssh_local_endpoint.is_some());
}

#[tokio::test]
async fn ssh_disconnect_closes_tunnel_endpoint() {
    init_logger();
    let stack = TransportStack::start().await.expect("failed to start transport stack");
    let mut saved = make_saved_connection("ssh-disconnect", stack.mongo_uri_for_ssh());
    saved.ssh = Some(stack.ssh_password_config(false));

    let (accepts_before, closes_after_disconnect) = run_blocking(move || -> Result<_, AppError> {
        let manager = ConnectionManager::new();
        let connection_id = Uuid::new_v4();
        let (_client, runtime_meta) = manager.connect_managed(connection_id, &saved)?;

        let endpoint =
            runtime_meta.ssh_local_endpoint.expect("ssh local endpoint should be present");
        let accepts_before =
            endpoint_accepts_connections(&endpoint).expect("endpoint check should succeed");
        manager.disconnect(connection_id);
        let closes_after_disconnect = wait_for_endpoint_closed(&endpoint, Duration::from_secs(3))
            .expect("endpoint close check should succeed");

        Ok((accepts_before, closes_after_disconnect))
    })
    .await
    .expect("managed SSH connection should succeed");

    assert!(accepts_before, "expected tunnel endpoint to accept connections before disconnect");
    assert!(
        closes_after_disconnect,
        "expected local SSH tunnel endpoint to close after disconnect"
    );
}

#[tokio::test]
async fn ssh_test_connection_then_connect_managed_succeeds() {
    init_logger();
    let stack = TransportStack::start().await.expect("failed to start transport stack");
    let mut saved = make_saved_connection("ssh-test-then-connect", stack.mongo_uri_for_ssh());
    saved.ssh = Some(stack.ssh_password_config(false));

    let runtime_meta = run_blocking(move || -> Result<_, AppError> {
        let manager = ConnectionManager::new();
        manager.test_connection(&saved, Duration::from_secs(5))?;

        let connection_id = Uuid::new_v4();
        let (client, runtime_meta) = manager.connect_managed(connection_id, &saved)?;
        let databases = manager.list_databases(&client)?;
        assert!(databases.iter().any(|name| name == "admin"));
        manager.disconnect(connection_id);

        Ok(runtime_meta)
    })
    .await
    .expect("test_connection and managed connect should succeed");

    assert!(runtime_meta.ssh_tunnel_active);
}

#[tokio::test]
async fn ssh_strict_tofu_first_connect_persists_host_key() {
    init_logger();
    let stack = TransportStack::start().await.expect("failed to start transport stack");
    let known_hosts_dir = TempDir::new().expect("failed to create temp dir for known_hosts");
    let known_hosts_path = known_hosts_dir.path().join("known_hosts.json");
    let _known_hosts_guard = KnownHostsEnvGuard::set(&known_hosts_path);

    let mut saved = make_saved_connection("ssh-tofu", stack.mongo_uri_for_ssh());
    saved.ssh = Some(stack.ssh_password_config(true));

    run_blocking(move || -> Result<(), AppError> {
        let manager = ConnectionManager::new();
        let connection_id = Uuid::new_v4();
        let _ = manager.connect_managed(connection_id, &saved)?;
        manager.disconnect(connection_id);
        Ok(())
    })
    .await
    .expect("first strict host-key connect should trust on first use");

    let payload =
        std::fs::read_to_string(&known_hosts_path).expect("known_hosts file should exist");
    let parsed: serde_json::Value =
        serde_json::from_str(&payload).expect("known_hosts should be valid JSON");
    let host_id = format!("{}:{}", stack.ssh_host, stack.ssh_port);
    let fingerprint = parsed["fingerprints"][&host_id].as_str().unwrap_or_default();

    assert!(!fingerprint.trim().is_empty(), "expected persisted host fingerprint for {host_id}");
}

#[tokio::test]
async fn ssh_strict_host_key_mismatch_fails() {
    init_logger();
    let stack = TransportStack::start().await.expect("failed to start transport stack");
    let known_hosts_dir = TempDir::new().expect("failed to create temp dir for known_hosts");
    let known_hosts_path = known_hosts_dir.path().join("known_hosts.json");
    let _known_hosts_guard = KnownHostsEnvGuard::set(&known_hosts_path);

    let host_id = format!("{}:{}", stack.ssh_host, stack.ssh_port);
    write_known_hosts_file(&known_hosts_path, &host_id, "SHA256:not-the-real-fingerprint")
        .expect("failed to seed mismatched known_hosts");

    let mut saved = make_saved_connection("ssh-mismatch", stack.mongo_uri_for_ssh());
    saved.ssh = Some(stack.ssh_password_config(true));

    let err = run_blocking(move || {
        let manager = ConnectionManager::new();
        manager
            .connect_managed(Uuid::new_v4(), &saved)
            .expect_err("strict host-key mismatch should fail")
    })
    .await;
    let message = err.to_string();
    assert!(
        message.contains("SSH host key mismatch"),
        "unexpected mismatch error message: {message}"
    );
}

#[tokio::test]
async fn socks5_proxy_connect_success_sets_runtime_meta() {
    init_logger();
    let stack = TransportStack::start().await.expect("failed to start transport stack");
    let proxy = LocalSocksProxy::start().await;

    let mut saved = make_saved_connection("socks5-proxy", stack.mongo_uri_for_host());
    saved.proxy = Some(proxy.config());

    let runtime_meta = run_blocking(move || -> Result<_, AppError> {
        let manager = ConnectionManager::new();
        let connection_id = Uuid::new_v4();
        let (_client, runtime_meta) = manager.connect_managed(connection_id, &saved)?;
        manager.disconnect(connection_id);
        Ok(runtime_meta)
    })
    .await
    .expect("managed proxy connection should succeed");

    assert!(runtime_meta.proxy_active);
    assert!(!runtime_meta.ssh_tunnel_active);
}

#[tokio::test]
async fn socks5_proxy_validation_errors() {
    init_logger();
    let mut missing_host = make_saved_connection(
        "proxy-missing-host",
        "mongodb://localhost:27017/?directConnection=true".to_string(),
    );
    missing_host.proxy = Some(ProxyConfig {
        enabled: true,
        kind: ProxyKind::Socks5,
        host: String::new(),
        port: 1080,
        username: None,
        password: None,
    });

    let mut bad_port = make_saved_connection(
        "proxy-bad-port",
        "mongodb://localhost:27017/?directConnection=true".to_string(),
    );
    bad_port.proxy = Some(ProxyConfig {
        enabled: true,
        kind: ProxyKind::Socks5,
        host: "127.0.0.1".to_string(),
        port: 0,
        username: None,
        password: None,
    });

    let (host_err, port_err) = run_blocking(move || {
        let manager = ConnectionManager::new();
        let host_err = manager
            .test_connection(&missing_host, Duration::from_secs(1))
            .expect_err("empty proxy host should fail validation");
        let port_err = manager
            .test_connection(&bad_port, Duration::from_secs(1))
            .expect_err("proxy port=0 should fail validation");
        (host_err.to_string(), port_err.to_string())
    })
    .await;

    assert!(
        host_err.contains("SOCKS5 proxy host is required"),
        "unexpected empty-host error: {host_err}"
    );
    assert!(
        port_err.contains("SOCKS5 proxy port must be greater than 0"),
        "unexpected bad-port error: {port_err}"
    );
}

#[tokio::test]
async fn ssh_and_proxy_enabled_together_rejected() {
    init_logger();
    let stack = TransportStack::start().await.expect("failed to start transport stack");
    let proxy = LocalSocksProxy::start().await;

    let mut saved = make_saved_connection("ssh-proxy-conflict", stack.mongo_uri_for_ssh());
    saved.ssh = Some(stack.ssh_password_config(false));
    saved.proxy = Some(proxy.config());

    let err = run_blocking(move || {
        let manager = ConnectionManager::new();
        manager
            .test_connection(&saved, Duration::from_secs(5))
            .expect_err("enabling SSH and SOCKS5 together should fail fast")
    })
    .await;

    assert!(
        err.to_string().contains("cannot be enabled together"),
        "unexpected conflict error: {err}"
    );
}

#[tokio::test]
async fn unmanaged_connect_rejects_enabled_ssh() {
    init_logger();
    let stack = TransportStack::start().await.expect("failed to start transport stack");

    let mut saved = make_saved_connection("unmanaged-ssh", stack.mongo_uri_for_ssh());
    saved.ssh = Some(stack.ssh_password_config(false));

    let err = run_blocking(move || {
        let manager = ConnectionManager::new();
        manager.connect(&saved).expect_err("unmanaged connect should reject enabled SSH config")
    })
    .await;
    assert!(
        err.to_string().contains("SSH connections require managed connect context"),
        "unexpected unmanaged-connect error: {}",
        err
    );
}
