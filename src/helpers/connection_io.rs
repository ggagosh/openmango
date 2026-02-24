//! Connection import/export types and logic.

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::helpers::{extract_uri_password, inject_uri_password, redact_uri_password};
use crate::models::{ProxyConfig, SavedConnection, SshConfig};

use super::crypto;

const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportMode {
    Redacted,
    Encrypted,
    Plaintext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedConnection {
    pub name: String,
    pub uri: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TransportSecrets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssh_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssh_identity_passphrase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proxy_password: Option<String>,
}

impl TransportSecrets {
    fn has_any(&self) -> bool {
        self.ssh_password.as_deref().is_some_and(|v| !v.trim().is_empty())
            || self.ssh_identity_passphrase.as_deref().is_some_and(|v| !v.trim().is_empty())
            || self.proxy_password.as_deref().is_some_and(|v| !v.trim().is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionExportFile {
    pub version: u32,
    pub app: String,
    pub exported_at: DateTime<Utc>,
    pub mode: ExportMode,
    pub connections: Vec<ExportedConnection>,
}

/// Build an export file from a list of saved connections.
pub fn build_export(
    connections: &[SavedConnection],
    mode: ExportMode,
    passphrase: Option<&str>,
) -> Result<ConnectionExportFile> {
    let mut exported = Vec::with_capacity(connections.len());

    for conn in connections {
        let (sanitized_ssh, sanitized_proxy, transport_secrets) =
            sanitize_transport(conn.ssh.clone(), conn.proxy.clone());
        let entry = match mode {
            ExportMode::Redacted => ExportedConnection {
                name: conn.name.clone(),
                uri: redact_uri_password(&conn.uri),
                read_only: conn.read_only,
                encrypted_password: None,
                encrypted_transport: None,
                ssh: sanitized_ssh,
                proxy: sanitized_proxy,
            },
            ExportMode::Encrypted => {
                let passphrase = passphrase
                    .ok_or_else(|| anyhow::anyhow!("passphrase required for encrypted export"))?;
                let password = extract_uri_password(&conn.uri);
                let encrypted = match &password {
                    Some(pw) => Some(crypto::encrypt_password(pw, passphrase)?),
                    None => None,
                };
                let encrypted_transport = if transport_secrets.has_any() {
                    let payload = serde_json::to_string(&transport_secrets)?;
                    Some(crypto::encrypt_password(&payload, passphrase)?)
                } else {
                    None
                };
                ExportedConnection {
                    name: conn.name.clone(),
                    uri: redact_uri_password(&conn.uri),
                    read_only: conn.read_only,
                    encrypted_password: encrypted,
                    encrypted_transport,
                    ssh: sanitized_ssh,
                    proxy: sanitized_proxy,
                }
            }
            ExportMode::Plaintext => ExportedConnection {
                name: conn.name.clone(),
                uri: conn.uri.clone(),
                read_only: conn.read_only,
                encrypted_password: None,
                encrypted_transport: None,
                ssh: conn.ssh.clone(),
                proxy: conn.proxy.clone(),
            },
        };
        exported.push(entry);
    }

    Ok(ConnectionExportFile {
        version: CURRENT_VERSION,
        app: "openmango".to_string(),
        exported_at: Utc::now(),
        mode,
        connections: exported,
    })
}

/// Parse an import file from JSON.
pub fn parse_import(json: &str) -> Result<ConnectionExportFile> {
    let file: ConnectionExportFile = serde_json::from_str(json)?;
    if file.version > CURRENT_VERSION {
        bail!("unsupported export version {} (max supported: {})", file.version, CURRENT_VERSION);
    }
    Ok(file)
}

/// Decrypt all encrypted passwords in an import file and inject them back into URIs.
pub fn decrypt_import_file(file: &mut ConnectionExportFile, passphrase: &str) -> Result<()> {
    for conn in &mut file.connections {
        if let Some(encrypted) = &conn.encrypted_password {
            let password = crypto::decrypt_password(encrypted, passphrase)?;
            conn.uri = inject_uri_password(&conn.uri, Some(&password));
            conn.encrypted_password = None;
        }
        if let Some(encrypted_transport) = &conn.encrypted_transport {
            let payload = crypto::decrypt_password(encrypted_transport, passphrase)?;
            let secrets: TransportSecrets = serde_json::from_str(&payload)?;
            apply_transport_secrets(conn, secrets);
            conn.encrypted_transport = None;
        }
    }
    file.mode = ExportMode::Plaintext;
    Ok(())
}

/// Produce final SavedConnections from an import file, auto-renaming duplicates.
pub fn resolve_import(
    file: &ConnectionExportFile,
    existing: &[SavedConnection],
) -> Vec<SavedConnection> {
    let existing_names: std::collections::HashSet<&str> =
        existing.iter().map(|c| c.name.as_str()).collect();

    file.connections
        .iter()
        .map(|ec| {
            let name = if existing_names.contains(ec.name.as_str()) {
                format!("{} (imported)", ec.name)
            } else {
                ec.name.clone()
            };
            let mut conn = SavedConnection::new(name, ec.uri.clone());
            conn.read_only = ec.read_only;
            conn.ssh = ec.ssh.clone();
            conn.proxy = ec.proxy.clone();
            conn
        })
        .collect()
}

fn sanitize_transport(
    ssh: Option<SshConfig>,
    proxy: Option<ProxyConfig>,
) -> (Option<SshConfig>, Option<ProxyConfig>, TransportSecrets) {
    let mut secrets = TransportSecrets::default();

    let mut sanitized_ssh = ssh;
    if let Some(ssh_cfg) = sanitized_ssh.as_mut() {
        secrets.ssh_password = ssh_cfg.password.take();
        secrets.ssh_identity_passphrase = ssh_cfg.identity_passphrase.take();
    }

    let mut sanitized_proxy = proxy;
    if let Some(proxy_cfg) = sanitized_proxy.as_mut() {
        secrets.proxy_password = proxy_cfg.password.take();
    }

    (sanitized_ssh, sanitized_proxy, secrets)
}

fn apply_transport_secrets(conn: &mut ExportedConnection, secrets: TransportSecrets) {
    if let Some(ssh_cfg) = conn.ssh.as_mut() {
        if secrets.ssh_password.as_deref().is_some_and(|v| !v.is_empty()) {
            ssh_cfg.password = secrets.ssh_password;
        }
        if secrets.ssh_identity_passphrase.as_deref().is_some_and(|v| !v.is_empty()) {
            ssh_cfg.identity_passphrase = secrets.ssh_identity_passphrase;
        }
    }

    if let Some(proxy_cfg) = conn.proxy.as_mut()
        && secrets.proxy_password.as_deref().is_some_and(|v| !v.is_empty())
    {
        proxy_cfg.password = secrets.proxy_password;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProxyKind, SshAuth};
    use uuid::Uuid;

    fn make_connections() -> Vec<SavedConnection> {
        vec![
            SavedConnection {
                id: Uuid::new_v4(),
                name: "Local".into(),
                uri: "mongodb://admin:secret@localhost:27017".into(),
                last_connected: None,
                read_only: false,
                ssh: Some(SshConfig {
                    enabled: true,
                    host: "bastion".into(),
                    port: 22,
                    username: "ubuntu".into(),
                    auth: SshAuth::Password,
                    password: Some("ssh-password".into()),
                    identity_file: None,
                    identity_passphrase: Some("ssh-passphrase".into()),
                    strict_host_key_checking: true,
                    local_bind_host: "127.0.0.1".into(),
                }),
                proxy: Some(ProxyConfig {
                    enabled: true,
                    kind: ProxyKind::Socks5,
                    host: "127.0.0.1".into(),
                    port: 1080,
                    username: Some("proxy-user".into()),
                    password: Some("proxy-password".into()),
                }),
            },
            SavedConnection {
                id: Uuid::new_v4(),
                name: "Atlas".into(),
                uri: "mongodb+srv://user:pass@cluster0.abc.mongodb.net/mydb".into(),
                last_connected: Some(Utc::now()),
                read_only: true,
                ssh: None,
                proxy: None,
            },
        ]
    }

    #[test]
    fn export_redacted_hides_passwords() {
        let conns = make_connections();
        let file = build_export(&conns, ExportMode::Redacted, None).unwrap();
        assert_eq!(file.mode, ExportMode::Redacted);
        assert_eq!(file.connections.len(), 2);
        for ec in &file.connections {
            assert!(!ec.uri.contains("secret"));
            assert!(!ec.uri.contains("pass"));
            assert!(ec.encrypted_password.is_none());
            assert!(ec.encrypted_transport.is_none());
            if let Some(ssh) = &ec.ssh {
                assert!(ssh.password.is_none());
                assert!(ssh.identity_passphrase.is_none());
            }
            if let Some(proxy) = &ec.proxy {
                assert!(proxy.password.is_none());
            }
        }
    }

    #[test]
    fn export_plaintext_keeps_passwords() {
        let conns = make_connections();
        let file = build_export(&conns, ExportMode::Plaintext, None).unwrap();
        assert_eq!(file.mode, ExportMode::Plaintext);
        assert!(file.connections[0].uri.contains("secret"));
        assert!(file.connections[1].uri.contains("pass"));
        assert_eq!(
            file.connections[0].ssh.as_ref().and_then(|cfg| cfg.password.as_deref()),
            Some("ssh-password")
        );
        assert_eq!(
            file.connections[0].proxy.as_ref().and_then(|cfg| cfg.password.as_deref()),
            Some("proxy-password")
        );
    }

    #[test]
    fn export_encrypted_round_trip() {
        let conns = make_connections();
        let passphrase = "test-passphrase";
        let mut file = build_export(&conns, ExportMode::Encrypted, Some(passphrase)).unwrap();
        assert_eq!(file.mode, ExportMode::Encrypted);
        for ec in &file.connections {
            assert!(ec.encrypted_password.is_some());
            assert!(!ec.uri.contains("secret"));
            if let Some(ssh) = &ec.ssh {
                assert!(ssh.password.is_none());
                assert!(ssh.identity_passphrase.is_none());
            }
            if let Some(proxy) = &ec.proxy {
                assert!(proxy.password.is_none());
            }
        }
        assert!(file.connections[0].encrypted_transport.is_some());
        decrypt_import_file(&mut file, passphrase).unwrap();
        assert!(file.connections[0].uri.contains("secret"));
        assert!(file.connections[1].uri.contains("pass"));
        assert_eq!(
            file.connections[0].ssh.as_ref().and_then(|cfg| cfg.password.as_deref()),
            Some("ssh-password")
        );
        assert_eq!(
            file.connections[0].ssh.as_ref().and_then(|cfg| cfg.identity_passphrase.as_deref()),
            Some("ssh-passphrase")
        );
        assert_eq!(
            file.connections[0].proxy.as_ref().and_then(|cfg| cfg.password.as_deref()),
            Some("proxy-password")
        );
    }

    #[test]
    fn encrypted_wrong_passphrase_fails() {
        let conns = make_connections();
        let mut file = build_export(&conns, ExportMode::Encrypted, Some("correct")).unwrap();
        let result = decrypt_import_file(&mut file, "wrong");
        assert!(result.is_err());
    }

    #[test]
    fn parse_and_version_validation() {
        let conns = make_connections();
        let file = build_export(&conns, ExportMode::Redacted, None).unwrap();
        let json = serde_json::to_string_pretty(&file).unwrap();
        let parsed = parse_import(&json).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.connections.len(), 2);

        // Future version should fail
        let bad = json.replace("\"version\": 1", "\"version\": 99");
        assert!(parse_import(&bad).is_err());
    }

    #[test]
    fn resolve_import_auto_renames_duplicates() {
        let existing = vec![SavedConnection {
            id: Uuid::new_v4(),
            name: "Local".into(),
            uri: "mongodb://localhost:27017".into(),
            last_connected: None,
            read_only: false,
            ssh: None,
            proxy: None,
        }];

        let file = ConnectionExportFile {
            version: 1,
            app: "openmango".into(),
            exported_at: Utc::now(),
            mode: ExportMode::Redacted,
            connections: vec![
                ExportedConnection {
                    name: "Local".into(),
                    uri: "mongodb://localhost:27017".into(),
                    read_only: false,
                    encrypted_password: None,
                    encrypted_transport: None,
                    ssh: None,
                    proxy: None,
                },
                ExportedConnection {
                    name: "Atlas".into(),
                    uri: "mongodb+srv://cluster0.abc.mongodb.net".into(),
                    read_only: true,
                    encrypted_password: None,
                    encrypted_transport: None,
                    ssh: None,
                    proxy: None,
                },
            ],
        };

        let resolved = resolve_import(&file, &existing);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].name, "Local (imported)");
        assert_eq!(resolved[1].name, "Atlas");
        // New UUIDs
        assert_ne!(resolved[0].id, existing[0].id);
    }

    #[test]
    fn no_password_uri_handles_gracefully() {
        let conns = vec![SavedConnection {
            id: Uuid::new_v4(),
            name: "NoAuth".into(),
            uri: "mongodb://localhost:27017".into(),
            last_connected: None,
            read_only: false,
            ssh: None,
            proxy: None,
        }];

        let file = build_export(&conns, ExportMode::Encrypted, Some("pass")).unwrap();
        assert!(file.connections[0].encrypted_password.is_none());

        let file = build_export(&conns, ExportMode::Redacted, None).unwrap();
        assert_eq!(file.connections[0].uri, "mongodb://localhost:27017");
    }
}
