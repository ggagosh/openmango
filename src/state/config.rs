// Configuration management for persistent state

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};
use std::fs;
use std::path::PathBuf;

use crate::models::connection::SavedConnection;
use crate::state::workspace::WorkspaceState;

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

    /// Load data from a binary file
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

    /// Save data to a binary file
    fn save<T: Serialize + ?Sized>(&self, filename: &str, data: &T) -> Result<()> {
        let path = self.file_path(filename);

        let bytes = postcard::to_allocvec(data)
            .with_context(|| format!("Failed to serialize {}", filename))?;

        fs::write(&path, bytes).with_context(|| format!("Failed to write {}", filename))?;

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
    // Preferences (placeholder for future)
    // =========================================================================

    // const PREFERENCES_FILE: &'static str = "settings.bin";
    //
    // pub fn load_preferences(&self) -> Result<Preferences> { ... }
    // pub fn save_preferences(&self, prefs: &Preferences) -> Result<()> { ... }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new().expect("Failed to initialize ConfigManager")
    }
}
