use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use crate::state::app_state::updater::{RELEASES_URL, UpdateChannel, UpdateRelease};

pub(super) const REPOSITORY_API: &str = "https://api.github.com/repos/ggagosh/openmango";

pub(super) fn client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!("OpenMango/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(timeout)
        .build()
        .context("Could not initialize the update connection")
}

pub(super) async fn check(
    channel: UpdateChannel,
    installed_channel: UpdateChannel,
    current_version: &str,
    current_sha: &str,
) -> Result<Option<Arc<UpdateRelease>>> {
    let client = client(Duration::from_secs(30))?;
    let endpoint = match channel {
        UpdateChannel::Stable => "latest",
        UpdateChannel::Nightly => "tags/nightly",
    };
    let response = client
        .get(format!("{REPOSITORY_API}/releases/{endpoint}"))
        .send()
        .await
        .context("Could not contact GitHub for update information")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let release: Value = response
        .error_for_status()
        .context("GitHub could not provide update information")?
        .json()
        .await
        .context("GitHub returned invalid release information")?;
    let version = match channel {
        UpdateChannel::Stable => {
            let tag =
                release["tag_name"].as_str().context("The stable release has no version tag")?;
            let version = tag.strip_prefix('v').unwrap_or(tag);
            let remote: semver::Version =
                version.parse().context("The stable release version is invalid")?;
            let local: semver::Version = current_version.parse()?;
            if installed_channel == channel && remote <= local {
                return Ok(None);
            }
            version.to_string()
        }
        UpdateChannel::Nightly => {
            let sha = nightly_sha(release["body"].as_str().unwrap_or_default())
                .context("The nightly release has no valid build identifier")?;
            if same_commit(current_sha, sha) {
                return Ok(None);
            }
            if installed_channel == channel {
                if !valid_sha(current_sha) {
                    bail!(
                        "This build cannot be compared with the nightly release. Open Releases to choose a build."
                    );
                }
                let comparison: Value = client
                    .get(format!("{REPOSITORY_API}/compare/{current_sha}...{sha}"))
                    .send()
                    .await
                    .context("Could not compare nightly builds")?
                    .error_for_status()
                    .context(
                        "The installed nightly could not be compared with the published build",
                    )?
                    .json()
                    .await
                    .context("GitHub returned an invalid build comparison")?;
                if !nightly_is_newer(&comparison)? {
                    return Ok(None);
                }
            }
            sha[..7].to_string()
        }
    };
    Ok(Some(Arc::new(candidate(&release, channel, version)?)))
}

fn valid_sha(sha: &str) -> bool {
    (7..=64).contains(&sha.len()) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn same_commit(local: &str, remote: &str) -> bool {
    valid_sha(local)
        && valid_sha(remote)
        && (local.starts_with(remote) || remote.starts_with(local))
}

fn nightly_sha(body: &str) -> Option<&str> {
    let sha = body.split_once("**Commit:**")?.1.split_whitespace().next()?;
    valid_sha(sha).then_some(sha)
}

fn nightly_is_newer(comparison: &Value) -> Result<bool> {
    match comparison["status"].as_str() {
        Some("ahead") => Ok(true),
        Some("behind" | "identical") => Ok(false),
        _ => bail!(
            "The nightly build is from a different history. Open Releases to choose a build; automatic downgrade was stopped."
        ),
    }
}

fn candidate(release: &Value, channel: UpdateChannel, version: String) -> Result<UpdateRelease> {
    let suffix = match std::env::consts::ARCH {
        "aarch64" => "macos-arm64.zip",
        "x86_64" => "macos-x86_64.zip",
        _ => bail!("Automatic updates are not available for this architecture"),
    };
    let assets = release["assets"].as_array().context("This release has no downloadable files")?;
    let asset = assets
        .iter()
        .find(|asset| {
            asset["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("OpenMango-") && name.ends_with(suffix))
        })
        .context("The release does not contain an update for this Mac")?;
    let name = asset["name"].as_str().context("The update file has no name")?;
    let asset_id = asset["id"].as_u64().context("The update file has no asset identifier")?;
    let size = asset["size"]
        .as_u64()
        .filter(|size| *size > 0)
        .context("The update file is empty or still being published")?;
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .map(parse_checksum)
        .transpose()?;
    let checksum_name = format!("{name}.sha256");
    let checksum_url = assets
        .iter()
        .find(|asset| asset["name"].as_str() == Some(&checksum_name))
        .and_then(|asset| asset["id"].as_u64())
        .map(|id| format!("{REPOSITORY_API}/releases/assets/{id}"));
    if sha256.is_none() && checksum_url.is_none() {
        bail!("The release is missing its SHA-256 checksum");
    }
    Ok(UpdateRelease {
        channel,
        version,
        release_url: match channel {
            UpdateChannel::Stable => format!("{RELEASES_URL}/latest"),
            UpdateChannel::Nightly => format!("{RELEASES_URL}/tag/nightly"),
        },
        // Asset IDs remain tied to the selected release, even when nightly is republished.
        download_url: format!("{REPOSITORY_API}/releases/assets/{asset_id}"),
        checksum_url,
        sha256,
        size,
    })
}

pub(super) fn parse_checksum(text: &str) -> Result<String> {
    let digest = text.split_whitespace().next().context("The checksum file is empty")?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("The release has an invalid SHA-256 checksum");
    }
    Ok(digest.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::{nightly_is_newer, nightly_sha, parse_checksum, same_commit};
    use serde_json::json;

    #[test]
    fn nightly_updates_require_forward_history_and_valid_identity() {
        assert!(nightly_is_newer(&json!({"status":"ahead"})).unwrap());
        for status in ["behind", "identical"] {
            assert!(!nightly_is_newer(&json!({"status":status})).unwrap());
        }
        assert!(nightly_is_newer(&json!({"status":"diverged"})).is_err());
        assert!(same_commit("abc1234", "abc1234def5678"));
        assert_eq!(nightly_sha("**Commit:** abc1234def5678\n"), Some("abc1234def5678"));
        assert_eq!(nightly_sha("**Commit:** not-a-sha"), None);
        assert_eq!(
            parse_checksum(&format!("{} file.zip", "A".repeat(64))).unwrap(),
            "a".repeat(64)
        );
        assert!(parse_checksum("invalid").is_err());
    }
}
