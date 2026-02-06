use std::path::PathBuf;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum UpdateStatus {
    Idle,
    Checking,
    Available { version: String, download_url: String },
    Downloading { version: String, progress_pct: u8 },
    ReadyToInstall { version: String, zip_path: PathBuf },
    Failed(String),
}
