// Configuration management for persistent state

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};
use std::fs;
use std::path::PathBuf;

use crate::models::connection::SavedConnection;
use crate::state::QueryLibrary;
use crate::state::settings::AppSettings;
use crate::state::workspace::WorkspaceState;

#[cfg(debug_assertions)]
const APP_NAME: &str = "openmango-dev";

#[cfg(not(debug_assertions))]
const APP_NAME: &str = "openmango";

/// Manages persistent configuration files
#[derive(Clone)]
pub struct ConfigManager {
    config_dir: PathBuf,
}

impl ConfigManager {
    /// Create a new ConfigManager, initializing the config directory if needed
    pub fn new() -> Result<Self> {
        let config_dir = Self::get_config_dir()?;

        // Ensure config directory exists
        if !config_dir.exists() {
            fs::create_dir_all(&config_dir).context("Failed to create config directory")?;
        }

        Ok(Self { config_dir })
    }

    /// Get the platform-specific config directory
    fn get_config_dir() -> Result<PathBuf> {
        dirs::config_dir().map(|p| p.join(APP_NAME)).context("Could not determine config directory")
    }

    /// Get path to a specific config file
    fn file_path(&self, filename: &str) -> PathBuf {
        self.config_dir.join(filename)
    }

    /// Load data from a JSON file
    fn load_json<T: DeserializeOwned>(&self, filename: &str) -> Result<Option<T>> {
        let path = self.file_path(filename);

        if !path.exists() {
            return Ok(None);
        }

        let data =
            fs::read_to_string(&path).with_context(|| format!("Failed to read {}", filename))?;

        let value: T = serde_json::from_str(&data)
            .with_context(|| format!("Failed to deserialize {}", filename))?;

        Ok(Some(value))
    }

    /// Save data to a JSON file (atomic via temp + rename).
    fn save_json<T: Serialize + ?Sized>(&self, filename: &str, data: &T) -> Result<()> {
        let path = self.file_path(filename);

        let json = serde_json::to_string_pretty(data)
            .with_context(|| format!("Failed to serialize {}", filename))?;

        atomic_write(&path, json.as_bytes())
            .with_context(|| format!("Failed to write {}", filename))?;

        Ok(())
    }

    // =========================================================================
    // Connections
    // =========================================================================

    const CONNECTIONS_FILE: &'static str = "connections.json";
    const QUERY_LIBRARY_FILE: &'static str = "query_library.json";
    const WORKSPACE_FILE: &'static str = "workspace.json";

    /// Load saved connections from disk
    pub fn load_connections(&self) -> Result<Vec<SavedConnection>> {
        if let Some(connections) = self.load_json(Self::CONNECTIONS_FILE)? {
            return Ok(connections);
        }
        Ok(Vec::new())
    }

    /// Save connections to disk with all secrets stripped.
    pub fn save_connections(&self, connections: &[SavedConnection]) -> Result<()> {
        let sanitized: Vec<SavedConnection> =
            connections.iter().map(|c| c.with_secrets_stripped()).collect();
        self.save_json(Self::CONNECTIONS_FILE, &sanitized)
    }

    // =========================================================================
    // Query Library
    // =========================================================================

    pub fn load_query_library(&self) -> Result<QueryLibrary> {
        Ok(self.load_json(Self::QUERY_LIBRARY_FILE)?.unwrap_or_default())
    }

    pub fn save_query_library(&self, library: &QueryLibrary) -> Result<()> {
        self.save_json(Self::QUERY_LIBRARY_FILE, library)
    }

    // =========================================================================
    // Workspace
    // =========================================================================

    /// Load workspace state from disk
    pub fn load_workspace(&self) -> Result<WorkspaceState> {
        if let Some(workspace) = self.load_json(Self::WORKSPACE_FILE)? {
            return Ok(workspace);
        }
        Ok(WorkspaceState::default())
    }

    /// Save workspace state to disk
    pub fn save_workspace(&self, workspace: &WorkspaceState) -> Result<()> {
        self.save_json(Self::WORKSPACE_FILE, workspace)
    }

    // =========================================================================
    // Settings
    // =========================================================================

    const SETTINGS_FILE: &'static str = "settings.json";

    /// Load application settings from disk
    pub fn load_settings(&self) -> Result<AppSettings> {
        if let Some(settings) = self.load_json(Self::SETTINGS_FILE)? {
            return Ok(settings);
        }
        Ok(AppSettings::default())
    }

    /// Save application settings to disk. The API key is never persisted here;
    /// it lives in the OS keychain.
    pub fn save_settings(&self, settings: &AppSettings) -> Result<()> {
        let mut to_save = settings.clone();
        to_save.ai.api_key.clear();
        self.save_json(Self::SETTINGS_FILE, &to_save)
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new().expect("Failed to initialize ConfigManager")
    }
}

/// Write `data` to `path` atomically: write to a sibling temp file first, then
/// rename.  `rename` is atomic on POSIX (same filesystem), so readers never see
/// a truncated or partially-written file — they get either the old content or the
/// new content, never a corrupt intermediate.
fn atomic_write(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(path);
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut tmp, data)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    impl ConfigManager {
        fn with_config_dir(config_dir: PathBuf) -> Self {
            Self { config_dir }
        }
    }

    #[test]
    fn save_connections_strips_all_credentials_from_disk() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        let mut connection = SavedConnection::new(
            "secret matrix".to_string(),
            "mongodb://user:authority-secret@host/db?tlsCertificateKeyFilePassword=tls-secret&proxyPassword=proxy-secret&authMechanismProperties=SERVICE_NAME%3Amongodb%2CAWS_SESSION_TOKEN%3Aaws-secret"
                .into(),
        );
        connection.ssh = Some(crate::models::SshConfig {
            password: Some("ssh-secret".into()),
            identity_passphrase: Some("identity-secret".into()),
            ..crate::models::SshConfig::default()
        });
        connection.proxy = Some(crate::models::ProxyConfig {
            password: Some("modeled-proxy-secret".into()),
            ..crate::models::ProxyConfig::default()
        });

        manager.save_connections(&[connection]).expect("failed to save connections");
        let raw = fs::read_to_string(temp_dir.path().join(ConfigManager::CONNECTIONS_FILE))
            .expect("failed to read connections");

        for secret in [
            "authority-secret",
            "tls-secret",
            "proxy-secret",
            "aws-secret",
            "ssh-secret",
            "identity-secret",
            "modeled-proxy-secret",
        ] {
            assert!(!raw.contains(secret), "connections.json leaked {secret}");
        }
        assert!(raw.contains("SERVICE_NAME%3Amongodb"));
    }

    #[test]
    fn malformed_connections_file_is_left_untouched() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        let path = temp_dir.path().join(ConfigManager::CONNECTIONS_FILE);
        let malformed = r#"[{"name":"recover-me","uri":"mongodb://user:secret@host""#;
        fs::write(&path, malformed).expect("failed to write malformed fixture");

        assert!(manager.load_connections().is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), malformed);
    }

    #[test]
    fn query_library_round_trips_atomically() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        let mut library = QueryLibrary::default();
        library.record(crate::state::QueryDefinition {
            connection_id: uuid::Uuid::nil(),
            database: "app".into(),
            collection: None,
            content: crate::state::QueryContent::Forge { statement: "db.users.find({})".into() },
        });

        manager.save_query_library(&library).expect("failed to save query library");
        let loaded = manager.load_query_library().expect("failed to load query library");

        assert_eq!(loaded.history().len(), 1);
    }

    #[test]
    fn legacy_query_library_defaults_saved_metadata() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        let now = chrono::Utc::now();
        let fixture = serde_json::json!({
            "history": [],
            "saved": [{
                "id": uuid::Uuid::new_v4(),
                "name": "Legacy",
                "created_at": now,
                "updated_at": now,
                "definition": {
                    "connection_id": uuid::Uuid::new_v4(),
                    "database": "app",
                    "content": { "type": "forge", "query": { "statement": "db.users.find({})" } }
                }
            }]
        });
        fs::write(
            temp_dir.path().join(ConfigManager::QUERY_LIBRARY_FILE),
            serde_json::to_vec_pretty(&fixture).unwrap(),
        )
        .unwrap();

        let loaded = manager.load_query_library().unwrap();
        assert_eq!(loaded.saved().len(), 1);
        assert_eq!(loaded.saved()[0].description, "");
        assert!(loaded.saved()[0].tags.is_empty());
        assert_eq!(loaded.saved()[0].scope, crate::state::SavedQueryScope::Connection);
    }

    #[test]
    fn malformed_query_library_is_left_untouched() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        let path = temp_dir.path().join(ConfigManager::QUERY_LIBRARY_FILE);
        let malformed = r#"{\"history\":["#;
        fs::write(&path, malformed).expect("failed to write malformed fixture");

        assert!(manager.load_query_library().is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), malformed);
    }

    #[test]
    fn load_connections_reads_json() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        fs::create_dir_all(temp_dir.path()).expect("failed to create config dir");

        let mut connection =
            SavedConnection::new("json".to_string(), "mongodb://localhost:27017".into());
        connection.environment = Some(crate::models::ConnectionEnvironment::Production);
        connection.confirm_production_writes = true;
        fs::write(
            temp_dir.path().join(ConfigManager::CONNECTIONS_FILE),
            serde_json::to_string_pretty(&vec![connection.clone()])
                .expect("failed to serialize json connections"),
        )
        .expect("failed to write json connections");

        let loaded = manager.load_connections().expect("failed to load json connections");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, connection.name);
        assert_eq!(loaded[0].environment, connection.environment);
        assert!(loaded[0].confirm_production_writes);
    }
}
