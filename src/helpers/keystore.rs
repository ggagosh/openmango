use anyhow::Result;
use gpui::{App, Task};
use uuid::Uuid;

/// Per-provider keychain URL prefix.
fn credentials_url(provider: &str) -> String {
    format!("com.openmango.ai.{provider}")
}

/// Per-connection keychain URL.
fn conn_credentials_url(id: Uuid, key: &str) -> String {
    format!("com.openmango.conn.{id}.{key}")
}

/// Whether to use the OS keychain even in debug builds.
#[cfg(debug_assertions)]
fn use_keychain_override() -> bool {
    std::env::var("OPENMANGO_DEV_USE_KEYCHAIN").ok().is_some_and(|v| v == "1")
}

pub struct KeyStore;

// ── Release builds (or debug with override) ─────────────────────────

#[cfg(not(debug_assertions))]
impl KeyStore {
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
            Err(e) => Err(e),
        })
    }

    pub fn delete(cx: &App, provider: &str) -> Task<Result<()>> {
        let url = credentials_url(provider);
        cx.delete_credentials(&url)
    }

    pub fn write_conn(cx: &App, id: Uuid, key: &str, secret: &str) -> Task<Result<()>> {
        let url = conn_credentials_url(id, key);
        let username = format!("conn.{id}.{key}");
        cx.write_credentials(&url, &username, secret.as_bytes())
    }

    pub fn read_conn(cx: &App, id: Uuid, key: &str) -> Task<Result<Option<String>>> {
        let url = conn_credentials_url(id, key);
        let task = cx.read_credentials(&url);
        cx.spawn(async move |_cx| match task.await {
            Ok(Some((_user, password))) => Ok(Some(String::from_utf8(password)?)),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        })
    }

    pub fn delete_conn(cx: &App, id: Uuid, key: &str) -> Task<Result<()>> {
        let url = conn_credentials_url(id, key);
        cx.delete_credentials(&url)
    }
}

// ── Debug builds ────────────────────────────────────────────────────

#[cfg(debug_assertions)]
impl KeyStore {
    pub fn write(cx: &App, provider: &str, api_key: &str) -> Task<Result<()>> {
        if use_keychain_override() {
            let url = credentials_url(provider);
            return cx.write_credentials(&url, provider, api_key.as_bytes());
        }
        let provider = provider.to_string();
        let api_key = api_key.to_string();
        cx.spawn(async move |_cx| dev_file::write(&provider, &api_key))
    }

    pub fn read(cx: &App, provider: &str) -> Task<Result<Option<String>>> {
        if use_keychain_override() {
            let url = credentials_url(provider);
            let provider = provider.to_string();
            let task = cx.read_credentials(&url);
            return cx.spawn(async move |_cx| match task.await {
                Ok(Some((user, password))) if user == provider => {
                    Ok(Some(String::from_utf8(password)?))
                }
                Ok(_) => Ok(None),
                Err(e) => Err(e),
            });
        }
        let provider = provider.to_string();
        cx.spawn(async move |_cx| dev_file::read(&provider))
    }

    pub fn delete(cx: &App, provider: &str) -> Task<Result<()>> {
        if use_keychain_override() {
            let url = credentials_url(provider);
            return cx.delete_credentials(&url);
        }
        let provider = provider.to_string();
        cx.spawn(async move |_cx| dev_file::delete(&provider))
    }

    pub fn write_conn(cx: &App, id: Uuid, key: &str, secret: &str) -> Task<Result<()>> {
        if use_keychain_override() {
            let url = conn_credentials_url(id, key);
            let username = format!("conn.{id}.{key}");
            return cx.write_credentials(&url, &username, secret.as_bytes());
        }
        let dev_key = format!("conn.{id}.{key}");
        let secret = secret.to_string();
        cx.spawn(async move |_cx| dev_file::write(&dev_key, &secret))
    }

    pub fn read_conn(cx: &App, id: Uuid, key: &str) -> Task<Result<Option<String>>> {
        if use_keychain_override() {
            let url = conn_credentials_url(id, key);
            let task = cx.read_credentials(&url);
            return cx.spawn(async move |_cx| match task.await {
                Ok(Some((_user, password))) => Ok(Some(String::from_utf8(password)?)),
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            });
        }
        let dev_key = format!("conn.{id}.{key}");
        cx.spawn(async move |_cx| dev_file::read(&dev_key))
    }

    pub fn delete_conn(cx: &App, id: Uuid, key: &str) -> Task<Result<()>> {
        if use_keychain_override() {
            let url = conn_credentials_url(id, key);
            return cx.delete_credentials(&url);
        }
        let dev_key = format!("conn.{id}.{key}");
        cx.spawn(async move |_cx| dev_file::delete(&dev_key))
    }
}

#[cfg(debug_assertions)]
mod dev_file {
    use anyhow::{Context, Result};
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    fn dev_credentials_path() -> Result<PathBuf> {
        let dir = dirs::config_dir()
            .context("Could not determine config directory")?
            .join("openmango-dev");
        fs::create_dir_all(&dir)?;
        Ok(dir.join("dev_credentials.json"))
    }

    fn load() -> Result<HashMap<String, String>> {
        let path = dev_credentials_path()?;
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let data = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&data)?)
    }

    fn save(map: &HashMap<String, String>) -> Result<()> {
        let path = dev_credentials_path()?;
        let json = serde_json::to_string_pretty(map)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn write(provider: &str, api_key: &str) -> Result<()> {
        let mut map = load()?;
        map.insert(provider.to_string(), api_key.to_string());
        save(&map)
    }

    pub fn read(provider: &str) -> Result<Option<String>> {
        let map = load()?;
        Ok(map.get(provider).cloned())
    }

    pub fn delete(provider: &str) -> Result<()> {
        let mut map = load()?;
        map.remove(provider);
        save(&map)
    }
}
