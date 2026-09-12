use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use futures::{StreamExt as _, channel::mpsc::UnboundedSender};
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;

use crate::state::app_state::updater::{DownloadedUpdate, UpdateRelease};

pub(super) enum Progress {
    Downloading { received: u64, total: u64 },
    Verifying,
    Finished(Result<DownloadedUpdate>),
}

fn staging_file(cache: &Path) -> Result<tempfile::NamedTempFile> {
    std::fs::create_dir_all(cache).context("Could not create the update download folder")?;
    tempfile::Builder::new()
        .prefix("OpenMango-update-")
        .suffix(".download")
        .tempfile_in(cache)
        .context("Could not create a temporary update file")
}

pub(super) async fn download(
    release: Arc<UpdateRelease>,
    progress: UnboundedSender<Progress>,
) -> Result<DownloadedUpdate> {
    let client = super::release::client(Duration::from_secs(15 * 60))?;
    let expected_checksum = if let Some(digest) = &release.sha256 {
        digest.clone()
    } else {
        let url = release.checksum_url.as_ref().context("The update has no checksum")?;
        let text = client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/octet-stream")
            .send()
            .await
            .context("Could not download the update checksum")?
            .error_for_status()
            .context("The checksum is no longer available; check for updates again")?
            .text()
            .await
            .context("Could not read the update checksum")?;
        super::release::parse_checksum(&text)?
    };
    let response = client
        .get(&release.download_url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .context("Could not start the update download")?
        .error_for_status()
        .context("The update file is no longer available; check for updates again")?;
    let cache = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("com.openmango.app")
        .join("updates");
    let staged = staging_file(&cache)?;
    let mut file = tokio::fs::File::from_std(
        staged.reopen().context("Could not open the temporary update file")?,
    );
    let mut stream = response.bytes_stream();
    let mut hasher = Sha256::new();
    let mut received = 0_u64;
    let mut last_percent = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("The update download was interrupted")?;
        received = received.saturating_add(chunk.len() as u64);
        ensure!(
            received <= release.size,
            "The update file is larger than its published size. Check for updates again."
        );
        file.write_all(&chunk)
            .await
            .context("Could not write the downloaded update; check free disk space")?;
        hasher.update(&chunk);
        let percent = received.saturating_mul(100).checked_div(release.size).unwrap_or(0).min(99);
        if percent != last_percent {
            last_percent = percent;
            let _ =
                progress.unbounded_send(Progress::Downloading { received, total: release.size });
        }
    }
    let _ = progress.unbounded_send(Progress::Verifying);
    ensure!(received == release.size, "The update download is incomplete. Download it again.");
    file.flush().await.context("Could not finish writing the update")?;
    file.sync_all().await.context("Could not save the update to disk")?;
    drop(file);
    ensure!(
        format!("{:x}", hasher.finalize()) == expected_checksum,
        "The update checksum did not match. Check for updates and download again."
    );
    ensure!(
        staged.path().is_file(),
        "The download folder was cleared before verification finished. Download the update again."
    );
    // No shared filename, destructive cleanup, or final rename: ownership promotes the verified file.
    Ok(DownloadedUpdate { release, archive: staged.into_temp_path() })
}

#[cfg(test)]
mod tests {
    use super::staging_file;
    use std::io::Write as _;

    #[test]
    fn failed_download_cleanup_cannot_remove_another_download() {
        let cache = tempfile::tempdir().unwrap();
        let mut first = staging_file(cache.path()).unwrap();
        let second = staging_file(cache.path()).unwrap();
        assert_ne!(first.path(), second.path());
        first.write_all(b"completed download").unwrap();
        drop(second);
        let verified = first.into_temp_path();
        assert_eq!(std::fs::read(&verified).unwrap(), b"completed download");
        let path = verified.to_path_buf();
        drop(verified);
        assert!(!path.exists());
    }
}
