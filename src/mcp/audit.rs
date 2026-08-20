use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::io::AsyncWriteExt as _;
use uuid::Uuid;

const AUDIT_QUEUE_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct McpAudit {
    sender: tokio::sync::mpsc::Sender<McpAuditEvent>,
}

#[derive(Debug, Serialize)]
pub struct McpAuditEvent {
    pub timestamp: DateTime<Utc>,
    pub correlation_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub operation_class: &'static str,
    pub policy_version: u32,
    pub decision: &'static str,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_error_code: Option<&'static str>,
}

impl McpAudit {
    pub fn start(path: PathBuf) -> Self {
        let (sender, mut receiver) =
            tokio::sync::mpsc::channel::<McpAuditEvent>(AUDIT_QUEUE_CAPACITY);
        tokio::spawn(async move {
            if let Some(parent) = path.parent()
                && let Err(error) = tokio::fs::create_dir_all(parent).await
            {
                log::error!("Could not create MCP audit directory: {error}");
                return;
            }
            let mut file =
                match tokio::fs::OpenOptions::new().create(true).append(true).open(&path).await {
                    Ok(file) => file,
                    Err(error) => {
                        log::error!("Could not open MCP audit file: {error}");
                        return;
                    }
                };
            set_owner_only_permissions(&path).await;
            while let Some(event) = receiver.recv().await {
                let Ok(mut line) = serde_json::to_vec(&event) else {
                    continue;
                };
                line.push(b'\n');
                if let Err(error) = file.write_all(&line).await {
                    log::error!("Could not write MCP audit event: {error}");
                    return;
                }
            }
        });
        Self { sender }
    }

    pub fn record(&self, event: McpAuditEvent) {
        if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) = self.sender.try_send(event) {
            log::warn!("MCP audit queue is full; dropping metadata-only event");
        }
    }
}

#[cfg(unix)]
async fn set_owner_only_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;

    if let Some(parent) = path.parent() {
        let _ = tokio::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).await;
    }
    let _ = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await;
}

#[cfg(not(unix))]
async fn set_owner_only_permissions(_path: &std::path::Path) {}
