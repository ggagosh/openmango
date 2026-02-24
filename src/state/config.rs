// Configuration management for persistent state

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

use crate::models::connection::SavedConnection;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacySavedConnectionV1 {
    id: Uuid,
    name: String,
    uri: String,
    last_connected: Option<DateTime<Utc>>,
    #[serde(default)]
    read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacySavedConnectionV0 {
    id: Uuid,
    name: String,
    uri: String,
    last_connected: Option<DateTime<Utc>>,
}

impl From<LegacySavedConnectionV1> for SavedConnection {
    fn from(value: LegacySavedConnectionV1) -> Self {
        SavedConnection {
            id: value.id,
            name: value.name,
            uri: value.uri,
            last_connected: value.last_connected,
            read_only: value.read_only,
            ssh: None,
            proxy: None,
        }
    }
}

impl From<LegacySavedConnectionV0> for SavedConnection {
    fn from(value: LegacySavedConnectionV0) -> Self {
        SavedConnection {
            id: value.id,
            name: value.name,
            uri: value.uri,
            last_connected: value.last_connected,
            read_only: false,
            ssh: None,
            proxy: None,
        }
    }
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

    /// Load data from a binary (postcard) file
    fn load<T: DeserializeOwned>(&self, filename: &str) -> Result<Option<T>> {
        let path = self.file_path(filename);

        if !path.exists() {
            return Ok(None);
        }

        let data = fs::read(&path).with_context(|| format!("Failed to read {}", filename))?;

        let value: T = postcard::from_bytes(&data)
            .with_context(|| format!("Failed to deserialize {}", filename))?;

        Ok(Some(value))
    }

    /// Save data to a binary (postcard) file (atomic via temp + rename).
    fn save<T: Serialize + ?Sized>(&self, filename: &str, data: &T) -> Result<()> {
        let path = self.file_path(filename);

        let bytes = postcard::to_allocvec(data)
            .with_context(|| format!("Failed to serialize {}", filename))?;

        atomic_write(&path, &bytes).with_context(|| format!("Failed to write {}", filename))?;

        Ok(())
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
    const CONNECTIONS_FILE_LEGACY: &'static str = "connections.bin";
    const WORKSPACE_FILE: &'static str = "workspace.bin";

    /// Load saved connections from disk
    pub fn load_connections(&self) -> Result<Vec<SavedConnection>> {
        if let Some(connections) = self.load_json(Self::CONNECTIONS_FILE)? {
            return Ok(connections);
        }

        let legacy_path = self.file_path(Self::CONNECTIONS_FILE_LEGACY);
        if !legacy_path.exists() {
            return Ok(Vec::new());
        }

        let legacy_data = fs::read(&legacy_path)
            .with_context(|| format!("Failed to read {}", Self::CONNECTIONS_FILE_LEGACY))?;

        if let Ok(connections) = postcard::from_bytes::<Vec<SavedConnection>>(&legacy_data) {
            return self.migrate_legacy_connections(connections);
        }

        if let Ok(legacy_connections) =
            postcard::from_bytes::<Vec<LegacySavedConnectionV1>>(&legacy_data)
        {
            let connections = legacy_connections.into_iter().map(Into::into).collect();
            return self.migrate_legacy_connections(connections);
        }

        if let Ok(legacy_connections) =
            postcard::from_bytes::<Vec<LegacySavedConnectionV0>>(&legacy_data)
        {
            let connections = legacy_connections.into_iter().map(Into::into).collect();
            return self.migrate_legacy_connections(connections);
        }

        Err(anyhow::anyhow!(
            "Failed to deserialize {} with current or legacy connection formats",
            Self::CONNECTIONS_FILE_LEGACY
        ))
    }

    /// Save connections to disk
    pub fn save_connections(&self, connections: &[SavedConnection]) -> Result<()> {
        self.save_json(Self::CONNECTIONS_FILE, connections)?;
        let legacy_path = self.file_path(Self::CONNECTIONS_FILE_LEGACY);
        let _ = fs::remove_file(legacy_path);
        Ok(())
    }

    // =========================================================================
    // Workspace
    // =========================================================================

    /// Load workspace state from disk
    pub fn load_workspace(&self) -> Result<WorkspaceState> {
        Ok(self.load(Self::WORKSPACE_FILE)?.unwrap_or_default())
    }

    /// Save workspace state to disk
    pub fn save_workspace(&self, workspace: &WorkspaceState) -> Result<()> {
        self.save(Self::WORKSPACE_FILE, workspace)
    }

    // =========================================================================
    // Settings
    // =========================================================================

    const SETTINGS_FILE: &'static str = "settings.json";
    const SETTINGS_FILE_LEGACY: &'static str = "settings.bin";

    /// Load application settings from disk (JSON, with postcard migration)
    pub fn load_settings(&self) -> Result<AppSettings> {
        // Try JSON first
        if let Some(settings) = self.load_json(Self::SETTINGS_FILE)? {
            return Ok(settings);
        }

        // Migrate from legacy postcard format
        if let Ok(Some(settings)) = self.load::<AppSettings>(Self::SETTINGS_FILE_LEGACY) {
            // Save as JSON and remove the old binary file
            self.save_json(Self::SETTINGS_FILE, &settings)?;
            let _ = fs::remove_file(self.file_path(Self::SETTINGS_FILE_LEGACY));
            return Ok(settings);
        }

        Ok(AppSettings::default())
    }

    /// Save application settings to disk
    pub fn save_settings(&self, settings: &AppSettings) -> Result<()> {
        self.save_json(Self::SETTINGS_FILE, settings)
    }

    fn migrate_legacy_connections(
        &self,
        connections: Vec<SavedConnection>,
    ) -> Result<Vec<SavedConnection>> {
        self.save_json(Self::CONNECTIONS_FILE, &connections)?;
        let _ = fs::remove_file(self.file_path(Self::CONNECTIONS_FILE_LEGACY));
        Ok(connections)
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
    fn load_connections_migrates_legacy_v1_bin() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        fs::create_dir_all(temp_dir.path()).expect("failed to create config dir");

        let legacy = vec![LegacySavedConnectionV1 {
            id: Uuid::new_v4(),
            name: "legacy".to_string(),
            uri: "mongodb://localhost:27017".to_string(),
            last_connected: None,
            read_only: true,
        }];
        let bytes = postcard::to_allocvec(&legacy).expect("failed to serialize legacy payload");
        fs::write(temp_dir.path().join(ConfigManager::CONNECTIONS_FILE_LEGACY), bytes)
            .expect("failed to write legacy bin");

        let loaded = manager.load_connections().expect("failed to load migrated connections");

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "legacy");
        assert!(loaded[0].read_only);
        assert!(loaded[0].ssh.is_none());
        assert!(loaded[0].proxy.is_none());
        assert!(temp_dir.path().join(ConfigManager::CONNECTIONS_FILE).exists());
        assert!(!temp_dir.path().join(ConfigManager::CONNECTIONS_FILE_LEGACY).exists());
    }

    #[test]
    fn load_connections_reads_json_first() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        fs::create_dir_all(temp_dir.path()).expect("failed to create config dir");

        let connection =
            SavedConnection::new("json".to_string(), "mongodb://localhost:27017".into());
        fs::write(
            temp_dir.path().join(ConfigManager::CONNECTIONS_FILE),
            serde_json::to_string_pretty(&vec![connection.clone()])
                .expect("failed to serialize json connections"),
        )
        .expect("failed to write json connections");

        let loaded = manager.load_connections().expect("failed to load json connections");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, connection.name);
    }
}
