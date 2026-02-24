//! Transport integration test helpers (SSH tunnel / SOCKS5 proxy).

use std::ffi::OsString;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tempfile::TempDir;
use testcontainers::core::{CmdWaitFor, ExecCommand, Host, IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use testcontainers_modules::mongo::Mongo;
use uuid::Uuid;

use openmango::models::{SshAuth, SshConfig};

pub const KNOWN_HOSTS_ENV: &str = "OPENMANGO_SSH_KNOWN_HOSTS_PATH";

static KNOWN_HOSTS_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub struct KnownHostsEnvGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Option<OsString>,
}

impl KnownHostsEnvGuard {
    pub fn set(path: impl AsRef<Path>) -> Self {
        let lock = KNOWN_HOSTS_ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("known-hosts env lock poisoned");
        let previous = std::env::var_os(KNOWN_HOSTS_ENV);
        // SAFETY: integration tests in this repository run with --test-threads=1 per binary.
        unsafe {
            std::env::set_var(KNOWN_HOSTS_ENV, path.as_ref());
        }
        Self { _lock: lock, previous }
    }
}

impl Drop for KnownHostsEnvGuard {
    fn drop(&mut self) {
        // SAFETY: integration tests in this repository run with --test-threads=1 per binary.
        unsafe {
            if let Some(previous) = self.previous.take() {
                std::env::set_var(KNOWN_HOSTS_ENV, previous);
            } else {
                std::env::remove_var(KNOWN_HOSTS_ENV);
            }
        }
    }
}

pub fn write_known_hosts_file(
    path: impl AsRef<Path>,
    host_id: &str,
    fingerprint: &str,
) -> Result<()> {
    let payload = serde_json::json!({
        "fingerprints": {
            host_id: fingerprint
        }
    });
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(&payload)?)?;
    Ok(())
}

pub fn parse_local_endpoint(endpoint: &str) -> Result<SocketAddr> {
    endpoint.parse::<SocketAddr>().with_context(|| format!("failed to parse endpoint: {endpoint}"))
}

pub fn endpoint_accepts_connections(endpoint: &str) -> Result<bool> {
    let addr = parse_local_endpoint(endpoint)?;
    Ok(TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok())
}

pub fn wait_for_endpoint_closed(endpoint: &str, timeout: Duration) -> Result<bool> {
    let addr = parse_local_endpoint(endpoint)?;
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err() {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(50));
    }

    Ok(false)
}

pub fn generate_ed25519_keypair(temp_dir: &TempDir) -> Result<(PathBuf, String)> {
    let private_key = temp_dir.path().join("id_ed25519");
    let private_key_str = private_key
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("private key path is not valid UTF-8"))?;

    let status = Command::new("ssh-keygen")
        .args(["-t", "ed25519", "-N", "", "-f", private_key_str, "-q"])
        .status()
        .context("failed to spawn ssh-keygen (required for identity-file integration test)")?;

    if !status.success() {
        bail!("ssh-keygen returned non-zero exit code: {status}");
    }

    let public_key = fs::read_to_string(private_key.with_extension("pub"))
        .context("failed to read generated public key")?;

    Ok((private_key, public_key.trim().to_string()))
}

pub struct TransportStack {
    #[allow(dead_code)]
    mongo: ContainerAsync<Mongo>,
    ssh: ContainerAsync<GenericImage>,
    pub mongo_host_port: u16,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_username: String,
    pub ssh_password: String,
}

impl TransportStack {
    pub async fn start() -> Result<Self> {
        let mongo = Mongo::default()
            .with_tag("7.0")
            .start()
            .await
            .context("failed to start mongo container for transport tests")?;
        let mongo_host_port = mongo
            .get_host_port_ipv4(27017.tcp())
            .await
            .context("failed to read mongo mapped host port")?;

        let ssh_password = format!("om-pass-{}", Uuid::new_v4().simple());
        let ssh_image = GenericImage::new("testcontainers/sshd", "1.3.0")
            .with_exposed_port(22.tcp())
            .with_wait_for(WaitFor::seconds(2))
            .with_entrypoint("/bin/sh");
        let ssh = ssh_image
            .with_cmd([
                "-lc",
                "echo \"root:${PASSWORD}\" | chpasswd && exec /usr/sbin/sshd -D -e -o PermitRootLogin=yes -o PasswordAuthentication=yes -o AllowTcpForwarding=yes -o GatewayPorts=yes",
            ])
            .with_env_var("PASSWORD", ssh_password.clone())
            .with_host("host.docker.internal", Host::HostGateway)
            .start()
            .await
            .context("failed to start sshd container for transport tests")?;

        let ssh_host = ssh.get_host().await.context("failed to read ssh host")?.to_string();
        let ssh_port =
            ssh.get_host_port_ipv4(22.tcp()).await.context("failed to read ssh mapped port")?;

        Ok(Self {
            mongo,
            ssh,
            mongo_host_port,
            ssh_host,
            ssh_port,
            ssh_username: "root".to_string(),
            ssh_password,
        })
    }

    pub fn mongo_uri_for_ssh(&self) -> String {
        format!("mongodb://host.docker.internal:{}/?directConnection=true", self.mongo_host_port)
    }

    pub fn mongo_uri_for_host(&self) -> String {
        format!("mongodb://127.0.0.1:{}/?directConnection=true", self.mongo_host_port)
    }

    pub fn ssh_password_config(&self, strict_host_key_checking: bool) -> SshConfig {
        SshConfig {
            enabled: true,
            host: self.ssh_host.clone(),
            port: self.ssh_port,
            username: self.ssh_username.clone(),
            auth: SshAuth::Password,
            password: Some(self.ssh_password.clone()),
            identity_file: None,
            identity_passphrase: None,
            strict_host_key_checking,
            local_bind_host: "127.0.0.1".to_string(),
        }
    }

    pub fn ssh_identity_config(
        &self,
        identity_file: PathBuf,
        strict_host_key_checking: bool,
    ) -> SshConfig {
        SshConfig {
            enabled: true,
            host: self.ssh_host.clone(),
            port: self.ssh_port,
            username: self.ssh_username.clone(),
            auth: SshAuth::IdentityFile,
            password: None,
            identity_file: Some(identity_file.to_string_lossy().to_string()),
            identity_passphrase: None,
            strict_host_key_checking,
            local_bind_host: "127.0.0.1".to_string(),
        }
    }

    pub async fn install_authorized_key(&self, public_key: &str) -> Result<()> {
        let command = format!(
            "mkdir -p /root/.ssh && chmod 700 /root/.ssh && printf '%s\\n' {} >> /root/.ssh/authorized_keys && chmod 600 /root/.ssh/authorized_keys",
            shell_single_quote(public_key.trim())
        );

        self.ssh
            .exec(
                ExecCommand::new(["/bin/sh", "-lc", command.as_str()])
                    .with_cmd_ready_condition(CmdWaitFor::exit_code(0)),
            )
            .await
            .context("failed to install SSH public key into test sshd container")?;

        Ok(())
    }
}

fn shell_single_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "'\"'\"'"))
}
