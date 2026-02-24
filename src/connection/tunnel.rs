//! SSH tunnel runtime support.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ssh2::Session;

use crate::error::{Error, Result};
use crate::models::{SshAuth, SshConfig};

#[cfg(debug_assertions)]
const APP_NAME: &str = "openmango-dev";

#[cfg(not(debug_assertions))]
const APP_NAME: &str = "openmango";

const STOP_POLL_INTERVAL: Duration = Duration::from_millis(50);
const IO_IDLE_SLEEP: Duration = Duration::from_millis(1);
const IO_BLOCK_RETRY_SLEEP: Duration = Duration::from_millis(2);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const CLIENT_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// Session timeout (ms) used during the poll loop so that blocking channel
/// read/write calls return quickly instead of hanging.
const IO_POLL_TIMEOUT_MS: u32 = 50;
/// Session timeout (ms) used when opening a new channel_direct_tcpip (needs
/// a round-trip to the remote SSH server).
const CHANNEL_OPEN_TIMEOUT_MS: u32 = 10_000;

#[derive(Default, Serialize, Deserialize)]
struct HostKeyStore {
    fingerprints: HashMap<String, String>,
}

pub struct SshTunnelHandle {
    stop_tx: Sender<()>,
    join_handle: Option<thread::JoinHandle<()>>,
    pub local_host: String,
    pub local_port: u16,
}

impl SshTunnelHandle {
    pub fn local_endpoint(&self) -> String {
        format!("{}:{}", self.local_host, self.local_port)
    }

    pub fn stop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl Drop for SshTunnelHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start_ssh_tunnel(config: &SshConfig) -> Result<SshTunnelHandle> {
    validate_ssh_config(config)?;
    // Create the long-lived SSH session — this is reused for all SOCKS5 clients
    // so that each new channel is just a `channel_direct_tcpip` (milliseconds)
    // instead of a full SSH handshake (seconds).
    let (session, ssh_socket) = establish_ssh_session(config)?;

    let listener = TcpListener::bind((config.local_bind_host.as_str(), 0))?;
    listener.set_nonblocking(true)?;
    let local_port = listener.local_addr()?.port();
    let local_host = config.local_bind_host.clone();

    let (stop_tx, stop_rx) = mpsc::channel();
    let join_handle = thread::Builder::new()
        .name("openmango-ssh-tunnel".to_string())
        .spawn(move || {
            run_tunnel_loop(listener, stop_rx, session, ssh_socket);
        })
        .map_err(Error::from)?;

    Ok(SshTunnelHandle { stop_tx, join_handle: Some(join_handle), local_host, local_port })
}

struct ActiveClient {
    local_stream: TcpStream,
    channel: ssh2::Channel,
    last_activity: Instant,
}

/// A SOCKS5 client that has completed the handshake but is waiting for an SSH
/// channel.  We accept eagerly (localhost handshake is fast) so the driver's
/// SOCKS5 client gets a timely auth reply, then open the expensive
/// `channel_direct_tcpip` during an idle moment.
struct PendingClient {
    stream: TcpStream,
    host: String,
    port: u16,
}

struct PollOutcome {
    progressed: bool,
    close: bool,
}

const SOCKS5_REPLY_SUCCESS: [u8; 10] = [0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
const SOCKS5_REPLY_HOST_UNREACHABLE: [u8; 10] = [0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0];

/// Single-threaded tunnel loop: one SSH session, multiplexed channels.
///
/// All libssh2 operations happen on this thread (libssh2 is not thread-safe).
/// New SOCKS5 clients get a `channel_direct_tcpip` on the shared session,
/// which takes milliseconds instead of the seconds needed for a full SSH session.
///
/// The session stays in **blocking** mode with a short timeout so libssh2 can
/// properly pump the SSH transport layer internally.  `ssh_socket` is a dup'd
/// handle to the session's TCP stream — used to set socket timeouts.
///
/// ## Loop structure
///
/// 1. Check stop signal
/// 2. Poll active clients (relay data)
/// 3. Accept new SOCKS5 clients → push to pending queue (always)
/// 4. Open SSH channels for pending clients → promote to active (only when idle)
/// 5. Sleep if no work done
fn run_tunnel_loop(
    listener: TcpListener,
    stop_rx: Receiver<()>,
    session: Session,
    ssh_socket: TcpStream,
) {
    // The session stays blocking.  We use a short session timeout so that
    // channel read/write calls return quickly (with EAGAIN mapped to
    // WouldBlock) instead of hanging forever.
    session.set_blocking(true);
    session.set_timeout(IO_POLL_TIMEOUT_MS);
    ssh_socket.set_nonblocking(false).ok();
    ssh_socket.set_read_timeout(Some(Duration::from_millis(IO_POLL_TIMEOUT_MS as u64))).ok();
    ssh_socket.set_write_timeout(Some(Duration::from_millis(IO_POLL_TIMEOUT_MS as u64))).ok();

    let mut clients: Vec<ActiveClient> = Vec::new();
    let mut pending: Vec<PendingClient> = Vec::new();
    let mut local_to_remote = [0u8; 8192];
    let mut remote_to_local = [0u8; 8192];

    loop {
        // 1. Check stop signal
        if stop_rx.try_recv().is_ok() {
            break;
        }

        // 2. Poll active clients (relay data)
        let mut any_progress = false;
        let mut i = 0;
        while i < clients.len() {
            let outcome =
                poll_client(&mut clients[i], i, &mut local_to_remote, &mut remote_to_local);

            if outcome.progressed {
                clients[i].last_activity = Instant::now();
                any_progress = true;
            } else if clients[i].last_activity.elapsed() > CLIENT_IDLE_TIMEOUT {
                log::warn!("SSH tunnel client idle timeout reached, closing connection");
                let remaining = clients.len() - 1;
                log::debug!("[client {i}] closed (idle timeout), {remaining} clients left");
                let removed = clients.swap_remove(i);
                drop(removed);
                continue;
            }

            if outcome.close {
                let remaining = clients.len() - 1;
                log::debug!("[client {i}] closed, {remaining} clients left");
                let removed = clients.swap_remove(i);
                drop(removed);
            } else {
                i += 1;
            }
        }

        // 3. Accept new SOCKS5 clients (always — localhost handshake is fast)
        match accept_socks5_client(&listener) {
            Some(Ok(pending_client)) => {
                log::debug!(
                    "SOCKS5 handshake done for {}:{}, queued for channel open",
                    pending_client.host,
                    pending_client.port,
                );
                pending.push(pending_client);
            }
            Some(Err(AcceptError::HandshakeFailed(err))) => {
                log::error!("SOCKS5 handshake failed: {err}");
            }
            Some(Err(AcceptError::ListenerFatal(err))) => {
                log::error!("SSH tunnel listener error: {err}");
                break;
            }
            None => {} // WouldBlock — no incoming connection
        }

        // 4. Open SSH channels for pending clients (only when no active I/O)
        if !any_progress && !pending.is_empty() {
            promote_pending_clients(&mut pending, &mut clients, &session);
        }

        // 5. Sleep if nothing happened
        if !any_progress && pending.is_empty() {
            thread::sleep(if clients.is_empty() { STOP_POLL_INTERVAL } else { IO_IDLE_SLEEP });
        }
    }
}

/// Relay data for a single active client.  Returns whether progress was made
/// and whether the client should be closed.
fn poll_client(
    client: &mut ActiveClient,
    idx: usize,
    local_to_remote: &mut [u8],
    remote_to_local: &mut [u8],
) -> PollOutcome {
    let mut progressed = false;
    let mut close = false;

    // Local → Remote
    match client.local_stream.read(local_to_remote) {
        Ok(0) => {
            log::debug!("[client {idx}] local EOF, sending channel EOF");
            let _ = client.channel.send_eof();
            close = true;
        }
        Ok(n) => match write_all_blocking(&mut client.channel, &local_to_remote[..n]) {
            Ok(()) => {
                log::debug!("[client {idx}] local→remote {n} bytes");
                progressed = true;
            }
            Err(err) => {
                log::debug!("[client {idx}] local→remote write failed: {err}");
                close = true;
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(err) => {
            log::debug!("[client {idx}] local read error: {err}");
            close = true;
        }
    }

    if !close {
        // Remote → Local
        match client.channel.read(remote_to_local) {
            Ok(0) if client.channel.eof() => {
                log::debug!("[client {idx}] channel EOF");
                close = true;
            }
            Ok(0) => {}
            Ok(n) => match write_all_nonblocking(&mut client.local_stream, &remote_to_local[..n]) {
                Ok(()) => {
                    log::debug!("[client {idx}] remote→local {n} bytes");
                    progressed = true;
                }
                Err(err) => {
                    log::debug!("[client {idx}] remote→local write failed: {err}");
                    close = true;
                }
            },
            Err(err) if is_would_block_or_timeout(&err) => {}
            Err(err) => {
                log::debug!("[client {idx}] channel read error: {err}");
                close = true;
            }
        }

        if client.channel.eof() {
            log::debug!("[client {idx}] channel eof detected");
            close = true;
        }
    }

    PollOutcome { progressed, close }
}

enum AcceptError {
    HandshakeFailed(Error),
    ListenerFatal(std::io::Error),
}

/// Try to accept one TCP connection and complete the SOCKS5 handshake.
/// Returns `None` on WouldBlock (no pending connection).
fn accept_socks5_client(
    listener: &TcpListener,
) -> Option<std::result::Result<PendingClient, AcceptError>> {
    match listener.accept() {
        Ok((mut stream, _)) => {
            stream.set_nonblocking(false).ok();
            stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
            stream.set_write_timeout(Some(Duration::from_secs(10))).ok();

            match socks5_read_connect(&mut stream) {
                Ok((host, port)) => {
                    log::debug!("SOCKS5 CONNECT request to {host}:{port}");
                    Some(Ok(PendingClient { stream, host, port }))
                }
                Err(err) => Some(Err(AcceptError::HandshakeFailed(err))),
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => None,
        Err(err) => Some(Err(AcceptError::ListenerFatal(err))),
    }
}

/// Open SSH channels for pending clients and promote them to active.
fn promote_pending_clients(
    pending: &mut Vec<PendingClient>,
    clients: &mut Vec<ActiveClient>,
    session: &Session,
) {
    session.set_timeout(CHANNEL_OPEN_TIMEOUT_MS);

    // Take all pending — each channel_direct_tcpip is fast (milliseconds)
    // and we only enter here when active clients are idle.
    let queued = std::mem::take(pending);
    for mut pc in queued {
        match session.channel_direct_tcpip(&pc.host, pc.port, None) {
            Ok(channel) => {
                let _ = pc.stream.write_all(&SOCKS5_REPLY_SUCCESS);
                log::debug!("SOCKS5 tunnel ready for {}:{}", pc.host, pc.port);
                pc.stream.set_nonblocking(true).ok();
                clients.push(ActiveClient {
                    local_stream: pc.stream,
                    channel,
                    last_activity: Instant::now(),
                });
            }
            Err(err) => {
                log::error!("SSH channel to {}:{} failed: {err}", pc.host, pc.port);
                let _ = pc.stream.write_all(&SOCKS5_REPLY_HOST_UNREACHABLE);
            }
        }
    }

    session.set_timeout(IO_POLL_TIMEOUT_MS);
}

fn is_would_block_or_timeout(err: &std::io::Error) -> bool {
    matches!(err.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
}

/// Read SOCKS5 auth negotiation and CONNECT request (RFC 1928).
/// Returns the target `(host, port)` without sending the CONNECT reply — the caller
/// must send success/failure after establishing the upstream connection.
fn socks5_read_connect(stream: &mut TcpStream) -> Result<(String, u16)> {
    let mut buf = [0u8; 2];
    stream.read_exact(&mut buf)?;
    if buf[0] != 0x05 {
        return Err(Error::Parse(format!("SOCKS5: unsupported version {:#04x}", buf[0])));
    }
    let nmethods = buf[1] as usize;
    let mut methods = vec![0u8; nmethods];
    stream.read_exact(&mut methods)?;

    // Reply: version 5, no-auth (0x00).
    stream.write_all(&[0x05, 0x00])?;

    // Read CONNECT request: VER CMD RSV ATYP ...
    let mut header = [0u8; 4];
    stream.read_exact(&mut header)?;
    if header[0] != 0x05 {
        return Err(Error::Parse(format!(
            "SOCKS5: unsupported request version {:#04x}",
            header[0]
        )));
    }
    if header[1] != 0x01 {
        stream.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])?;
        return Err(Error::Parse(format!("SOCKS5: unsupported command {:#04x}", header[1])));
    }

    let (host, port) = match header[3] {
        // IPv4
        0x01 => {
            let mut addr = [0u8; 4];
            stream.read_exact(&mut addr)?;
            let mut port_buf = [0u8; 2];
            stream.read_exact(&mut port_buf)?;
            let host = format!("{}.{}.{}.{}", addr[0], addr[1], addr[2], addr[3]);
            (host, u16::from_be_bytes(port_buf))
        }
        // Domain name
        0x03 => {
            let mut len_buf = [0u8; 1];
            stream.read_exact(&mut len_buf)?;
            let mut domain = vec![0u8; len_buf[0] as usize];
            stream.read_exact(&mut domain)?;
            let mut port_buf = [0u8; 2];
            stream.read_exact(&mut port_buf)?;
            let host = String::from_utf8(domain)
                .map_err(|_| Error::Parse("SOCKS5: invalid UTF-8 in domain name".to_string()))?;
            (host, u16::from_be_bytes(port_buf))
        }
        // IPv6
        0x04 => {
            let mut addr = [0u8; 16];
            stream.read_exact(&mut addr)?;
            let mut port_buf = [0u8; 2];
            stream.read_exact(&mut port_buf)?;
            let segments: Vec<String> =
                addr.chunks(2).map(|c| format!("{:x}", u16::from_be_bytes([c[0], c[1]]))).collect();
            let host = segments.join(":");
            (host, u16::from_be_bytes(port_buf))
        }
        atyp => {
            stream.write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])?;
            return Err(Error::Parse(format!("SOCKS5: unsupported address type {:#04x}", atyp)));
        }
    };

    Ok((host, port))
}

/// Write all data through a blocking ssh2 channel.  The session timeout
/// (set_timeout) ensures each individual write returns within IO_POLL_TIMEOUT_MS.
/// libssh2 properly pumps the SSH transport on each call.
fn write_all_blocking<W: Write>(writer: &mut W, data: &[u8]) -> Result<()> {
    let mut written = 0;
    let deadline = Instant::now() + WRITE_TIMEOUT;
    while written < data.len() {
        if Instant::now() > deadline {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "SSH tunnel write timed out",
            )));
        }
        match writer.write(&data[written..]) {
            Ok(0) => {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "failed to write to channel",
                )));
            }
            Ok(bytes) => written += bytes,
            Err(err) if is_would_block_or_timeout(&err) => {
                // Session timeout fired — just retry, the deadline guards us.
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Ok(())
}

fn write_all_nonblocking<W: Write>(writer: &mut W, data: &[u8]) -> Result<()> {
    let mut written = 0;
    let deadline = Instant::now() + WRITE_TIMEOUT;
    while written < data.len() {
        if Instant::now() > deadline {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "SSH tunnel write timed out",
            )));
        }
        match writer.write(&data[written..]) {
            Ok(0) => {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "failed to write to stream",
                )));
            }
            Ok(bytes) => {
                written += bytes;
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(IO_BLOCK_RETRY_SLEEP);
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Ok(())
}

/// The returned `TcpStream` is a dup'd handle to the SSH socket — callers can use it
/// to toggle blocking/non-blocking mode without going through the session.
fn establish_ssh_session(config: &SshConfig) -> Result<(Session, TcpStream)> {
    let tcp = TcpStream::connect((config.host.as_str(), config.port))?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(10)))?;
    let tcp_ctl = tcp.try_clone()?;

    let mut session = Session::new()
        .map_err(|err| Error::Parse(format!("Failed to create SSH session: {err}")))?;
    session.set_tcp_stream(tcp);
    session.handshake()?;
    verify_or_learn_host_key(&session, config)?;

    match config.auth {
        SshAuth::Password => {
            let password = config.password.as_deref().ok_or_else(|| {
                Error::Parse("SSH password is required for password authentication".to_string())
            })?;
            session.userauth_password(config.username.as_str(), password)?;
        }
        SshAuth::IdentityFile => {
            let identity_file = config.identity_file.as_deref().ok_or_else(|| {
                Error::Parse(
                    "SSH identity file path is required for identity-file authentication"
                        .to_string(),
                )
            })?;
            let identity_path = resolve_identity_file_path(identity_file);
            let passphrase = config.identity_passphrase.as_deref();
            session.userauth_pubkey_file(
                config.username.as_str(),
                None,
                identity_path.as_path(),
                passphrase,
            )?;
        }
    }

    if !session.authenticated() {
        return Err(Error::Parse("SSH authentication failed".to_string()));
    }

    Ok((session, tcp_ctl))
}

fn verify_or_learn_host_key(session: &Session, config: &SshConfig) -> Result<()> {
    if !config.strict_host_key_checking {
        return Ok(());
    }

    let (host_key, _host_key_type) = session
        .host_key()
        .ok_or_else(|| Error::Parse("Unable to read SSH host key from server".to_string()))?;
    let fingerprint = ssh_host_key_fingerprint(host_key);
    let host_id = format!("{}:{}", config.host, config.port);
    let mut store = load_host_key_store()?;

    match store.fingerprints.get(&host_id) {
        Some(existing) if existing == &fingerprint => Ok(()),
        Some(existing) => Err(Error::Parse(format!(
            "SSH host key mismatch for {host_id}. expected {existing}, got {fingerprint}"
        ))),
        None => {
            store.fingerprints.insert(host_id.clone(), fingerprint);
            save_host_key_store(&store)?;
            log::info!("Trusted SSH host key for {}", host_id);
            Ok(())
        }
    }
}

fn ssh_host_key_fingerprint(host_key: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(host_key);
    let digest = hasher.finalize();
    format!("SHA256:{}", base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest))
}

fn validate_ssh_config(config: &SshConfig) -> Result<()> {
    if config.host.trim().is_empty() {
        return Err(Error::Parse("SSH host is required".to_string()));
    }
    if config.username.trim().is_empty() {
        return Err(Error::Parse("SSH username is required".to_string()));
    }
    if config.port == 0 {
        return Err(Error::Parse("SSH port must be greater than 0".to_string()));
    }
    if config.local_bind_host.trim().is_empty() {
        return Err(Error::Parse("SSH local bind host is required".to_string()));
    }

    match config.auth {
        SshAuth::Password => {
            let password = config.password.as_deref().unwrap_or_default();
            if password.trim().is_empty() {
                return Err(Error::Parse(
                    "SSH password is required for password authentication".to_string(),
                ));
            }
        }
        SshAuth::IdentityFile => {
            let identity_file = config.identity_file.as_deref().ok_or_else(|| {
                Error::Parse(
                    "SSH identity file path is required for identity-file authentication"
                        .to_string(),
                )
            })?;
            let identity_path = resolve_identity_file_path(identity_file);
            if !identity_path.exists() {
                return Err(Error::Parse(format!(
                    "SSH identity file does not exist: {}",
                    identity_path.display()
                )));
            }
            if !identity_path.is_file() {
                return Err(Error::Parse(format!(
                    "SSH identity file path is not a file: {}",
                    identity_path.display()
                )));
            }
        }
    }

    Ok(())
}

fn resolve_identity_file_path(raw_path: &str) -> PathBuf {
    let trimmed = raw_path.trim();
    if let Some(home_relative) = trimmed.strip_prefix("~/")
        && let Some(home_dir) = dirs::home_dir()
    {
        return home_dir.join(home_relative);
    }
    if trimmed == "~"
        && let Some(home_dir) = dirs::home_dir()
    {
        return home_dir;
    }

    Path::new(trimmed).to_path_buf()
}

fn host_key_store_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("OPENMANGO_SSH_KNOWN_HOSTS_PATH") {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        return Ok(path);
    }

    let config_dir = dirs::config_dir()
        .ok_or_else(|| Error::Parse("Could not determine config directory".to_string()))?
        .join(APP_NAME);
    fs::create_dir_all(&config_dir)?;
    Ok(config_dir.join("known_hosts.json"))
}

fn load_host_key_store() -> Result<HostKeyStore> {
    let path = host_key_store_path()?;
    if !path.exists() {
        return Ok(HostKeyStore::default());
    }
    let contents = fs::read_to_string(path)?;
    let parsed: HostKeyStore = serde_json::from_str(&contents)
        .map_err(|err| Error::Parse(format!("Failed to parse host key store: {err}")))?;
    Ok(parsed)
}

fn save_host_key_store(store: &HostKeyStore) -> Result<()> {
    let path = host_key_store_path()?;
    let serialized = serde_json::to_string_pretty(store)
        .map_err(|err| Error::Parse(format!("Failed to serialize host key store: {err}")))?;
    fs::write(path, serialized)?;
    Ok(())
}
