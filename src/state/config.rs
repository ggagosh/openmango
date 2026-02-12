// Configuration management for persistent state

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};
use std::fs;
use std::path::PathBuf;

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

    /// Save data to a binary (postcard) file
    fn save<T: Serialize + ?Sized>(&self, filename: &str, data: &T) -> Result<()> {
        let path = self.file_path(filename);

        let bytes = postcard::to_allocvec(data)
            .with_context(|| format!("Failed to serialize {}", filename))?;

        fs::write(&path, bytes).with_context(|| format!("Failed to write {}", filename))?;

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

    /// Save data to a JSON file
    fn save_json<T: Serialize + ?Sized>(&self, filename: &str, data: &T) -> Result<()> {
        let path = self.file_path(filename);

        let json = serde_json::to_string_pretty(data)
            .with_context(|| format!("Failed to serialize {}", filename))?;

        fs::write(&path, json).with_context(|| format!("Failed to write {}", filename))?;

        Ok(())
    }

    // =========================================================================
    // Connections
    // =========================================================================

    const CONNECTIONS_FILE: &'static str = "connections.bin";
    const WORKSPACE_FILE: &'static str = "workspace.bin";

    /// Load saved connections from disk
    pub fn load_connections(&self) -> Result<Vec<SavedConnection>> {
        Ok(self.load(Self::CONNECTIONS_FILE)?.unwrap_or_default())
    }

    /// Save connections to disk
    pub fn save_connections(&self, connections: &[SavedConnection]) -> Result<()> {
        self.save(Self::CONNECTIONS_FILE, connections)
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
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new().expect("Failed to initialize ConfigManager")
    }
}
