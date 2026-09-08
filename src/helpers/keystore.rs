use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use gpui_kit::{App, Task};
use uuid::Uuid;

fn credentials_url(provider: &str) -> String {
    format!("com.openmango.ai.{provider}")
}

fn conn_credentials_url(id: Uuid, key: &str) -> String {
    format!("com.openmango.conn.{id}.{key}")
}

fn mcp_grant_credentials_url(id: Uuid) -> String {
    format!("com.openmango.mcp.grant.{id}")
}

const MCP_TOKEN_PROVIDER: &str = "mcp-server-token";
const HISTORY_KEY_URL: &str = "com.openmango.history.key";
const HISTORY_KEY_USER: &str = "history";

pub struct KeyStore;

impl KeyStore {
    pub fn write_history_key(cx: &App, key: &[u8; 32]) -> Task<Result<()>> {
        cx.write_credentials(HISTORY_KEY_URL, HISTORY_KEY_USER, key)
    }

    pub fn read_history_key(cx: &App) -> Task<Result<Option<Vec<u8>>>> {
        let task = cx.read_credentials(HISTORY_KEY_URL);
        cx.spawn(async move |_cx| match task.await {
            Ok(Some((user, key))) if user == HISTORY_KEY_USER => Ok(Some(key)),
            Ok(_) => Ok(None),
            Err(error) if credential_was_missing(&error) => Ok(None),
            Err(error) => Err(error),
        })
    }

    pub fn write_mcp_token(cx: &App, token: &str) -> Task<Result<()>> {
        Self::write(cx, MCP_TOKEN_PROVIDER, token)
    }

    pub fn read_mcp_token(cx: &App) -> Task<Result<Option<String>>> {
        Self::read(cx, MCP_TOKEN_PROVIDER)
    }

    pub fn delete_mcp_token(cx: &App) -> Task<Result<()>> {
        Self::delete(cx, MCP_TOKEN_PROVIDER)
    }

    pub fn write_mcp_grant(cx: &App, id: Uuid, token: &str) -> Task<Result<()>> {
        cx.write_credentials(&mcp_grant_credentials_url(id), &id.to_string(), token.as_bytes())
    }

    pub fn read_mcp_grant(cx: &App, id: Uuid) -> Task<Result<Option<String>>> {
        let task = cx.read_credentials(&mcp_grant_credentials_url(id));
        cx.spawn(async move |_cx| match task.await {
            Ok(Some((_user, token))) => Ok(Some(String::from_utf8(token)?)),
            Ok(None) => Ok(None),
            Err(error) if credential_was_missing(&error) => Ok(None),
            Err(error) => Err(error),
        })
    }

    pub fn delete_mcp_grant(cx: &App, id: Uuid) -> Task<Result<()>> {
        ignore_missing(cx, cx.delete_credentials(&mcp_grant_credentials_url(id)))
    }

    pub fn write(cx: &App, provider: &str, api_key: &str) -> Task<Result<()>> {
        let url = credentials_url(provider);
        cx.write_credentials(&url, provider, api_key.as_bytes())
    }

    pub fn read(cx: &App, provider: &str) -> Task<Result<Option<String>>> {
        let url = credentials_url(provider);
        let provider = provider.to_string();
        let task = cx.read_credentials(&url);
        cx.spawn(async move |_cx| match task.await {
            Ok(Some((user, password))) if user == provider => {
                Ok(Some(String::from_utf8(password)?))
            }
            Ok(_) => Ok(None),
            Err(error) if credential_was_missing(&error) => Ok(None),
            Err(error) => Err(error),
        })
    }

    pub fn delete(cx: &App, provider: &str) -> Task<Result<()>> {
        ignore_missing(cx, cx.delete_credentials(&credentials_url(provider)))
    }

    pub fn write_conn(cx: &App, id: Uuid, key: &str, secret: &str) -> Task<Result<()>> {
        let url = conn_credentials_url(id, key);
        let username = format!("conn.{id}.{key}");
        cx.write_credentials(&url, &username, secret.as_bytes())
    }

    pub fn read_conn(cx: &App, id: Uuid, key: &str) -> Task<Result<Option<String>>> {
        let task = cx.read_credentials(&conn_credentials_url(id, key));
        cx.spawn(async move |_cx| match task.await {
            Ok(Some((_user, password))) => Ok(Some(String::from_utf8(password)?)),
            Ok(None) => Ok(None),
            Err(error) if credential_was_missing(&error) => Ok(None),
            Err(error) => Err(error),
        })
    }

    pub fn delete_conn(cx: &App, id: Uuid, key: &str) -> Task<Result<()>> {
        ignore_missing(cx, cx.delete_credentials(&conn_credentials_url(id, key)))
    }

    pub fn read_legacy_dev_credentials() -> Result<Option<HashMap<String, String>>> {
        let path = legacy_dev_credentials_path()?;
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(path)?;
        Ok(Some(serde_json::from_str(&data)?))
    }

    pub fn delete_legacy_dev_credentials() -> Result<()> {
        let path = legacy_dev_credentials_path()?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

fn legacy_dev_credentials_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("Could not determine config directory")?
        .join("openmango-dev")
        .join("dev_credentials.json"))
}

fn ignore_missing(cx: &App, task: Task<Result<()>>) -> Task<Result<()>> {
    cx.spawn(async move |_cx| match task.await {
        Ok(()) => Ok(()),
        Err(error) if credential_was_missing(&error) => Ok(()),
        Err(error) => Err(error),
    })
}

fn credential_was_missing(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("-25300") || message.contains("not found") || message.contains("no matching")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_missing_credential_errors_are_normalized() {
        assert!(credential_was_missing(&anyhow::anyhow!("CredReadW failed: not found")));
        assert!(credential_was_missing(&anyhow::anyhow!("read password failed: -25300")));
        assert!(!credential_was_missing(&anyhow::anyhow!("permission denied")));
    }
}
