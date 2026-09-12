use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub const RELEASES_URL: &str = "https://github.com/ggagosh/openmango/releases";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    Stable,
    Nightly,
}

impl Default for UpdateChannel {
    fn default() -> Self {
        if option_env!("OPENMANGO_RELEASE_CHANNEL").is_some_and(|channel| channel == "nightly") {
            Self::Nightly
        } else {
            Self::Stable
        }
    }
}

impl UpdateChannel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Nightly => "Nightly",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UpdateRelease {
    pub channel: UpdateChannel,
    pub version: String,
    pub release_url: String,
    pub download_url: String,
    pub checksum_url: Option<String>,
    pub sha256: Option<String>,
    pub size: u64,
    pub linux_manifest: Option<LinuxUpdateManifest>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxUpdateManifest {
    pub schema: u32,
    pub os: String,
    pub arch: String,
    pub channel: UpdateChannel,
    pub version: String,
    pub commit: String,
    pub filename: String,
    pub size: u64,
    pub sha256: String,
}

impl UpdateRelease {
    pub fn label(&self) -> String {
        match self.channel {
            UpdateChannel::Stable => format!("Stable v{}", self.version),
            UpdateChannel::Nightly => format!("Nightly {}", self.version),
        }
    }
}

/// The verified file remains owned until installation or cancellation finishes.
#[derive(Debug)]
pub struct DownloadedUpdate {
    pub release: Arc<UpdateRelease>,
    pub archive: tempfile::TempPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStage {
    Check,
    Download,
    Verify,
    Install,
}

#[derive(Debug, Clone)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate {
        channel: UpdateChannel,
    },
    Unavailable(String),
    Available(Arc<UpdateRelease>),
    Downloading {
        release: Arc<UpdateRelease>,
        received: u64,
        total: u64,
    },
    Verifying(Arc<UpdateRelease>),
    ReadyToInstall(Arc<DownloadedUpdate>),
    Installing(Arc<UpdateRelease>),
    Failed {
        stage: UpdateStage,
        message: String,
        release: Option<Arc<UpdateRelease>>,
        downloaded: Option<Arc<DownloadedUpdate>>,
    },
}

impl UpdateStatus {
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Downloading { .. } | Self::Verifying(_) | Self::Installing(_)
        )
    }

    pub fn can_check(&self) -> bool {
        !self.is_busy() && !matches!(self, Self::ReadyToInstall(_) | Self::Unavailable(_))
    }

    pub fn release(&self) -> Option<Arc<UpdateRelease>> {
        match self {
            Self::Available(release)
            | Self::Verifying(release)
            | Self::Installing(release)
            | Self::Downloading { release, .. } => Some(release.clone()),
            Self::ReadyToInstall(download) => Some(download.release.clone()),
            Self::Failed { release, .. } => release.clone(),
            _ => None,
        }
    }

    pub fn progress_pct(&self) -> Option<f32> {
        match self {
            Self::Downloading { received, total, .. } if *total > 0 => {
                Some(((*received as f64 / *total as f64) * 100.).min(99.) as f32)
            }
            _ => None,
        }
    }
}
