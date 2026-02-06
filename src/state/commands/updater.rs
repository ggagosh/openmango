use std::path::PathBuf;
use std::process::Command;

use gpui::{App, AppContext as _, Entity};

use crate::state::AppState;
use crate::state::app_state::updater::UpdateStatus;
use crate::state::events::AppEvent;

use super::AppCommands;

#[cfg(target_arch = "aarch64")]
const ARCH_SUFFIX: &str = "macos-arm64";
#[cfg(target_arch = "x86_64")]
const ARCH_SUFFIX: &str = "macos-x86_64";

const GITHUB_API: &str = "https://api.github.com/repos/ggagosh/openmango/releases";

/// Result of checking a single release channel.
struct ReleaseCandidate {
    version: String,
    download_url: String,
}

/// Find the arch-matching zip asset from a release JSON.
fn find_asset_url(release: &serde_json::Value) -> Option<String> {
    release["assets"].as_array()?.iter().find_map(|a| {
        let name = a["name"].as_str()?;
        if name.contains(ARCH_SUFFIX) && name.ends_with(".zip") {
            a["browser_download_url"].as_str().map(String::from)
        } else {
            None
        }
    })
}

/// Extract the commit SHA from a nightly release body.
/// Body format: "...**Commit:** abc123def..."
fn parse_nightly_sha(body: &str) -> Option<&str> {
    let marker = "**Commit:** ";
    let start = body.find(marker)? + marker.len();
    let rest = &body[start..];
    // SHA is the next word (up to whitespace or end)
    let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
    let sha = &rest[..end];
    if sha.len() >= 7 { Some(sha) } else { None }
}

impl AppCommands {
    /// Check GitHub for a newer release (stable or nightly). Runs silently on startup.
    pub fn check_for_updates(state: Entity<AppState>, cx: &mut App) {
        state.update(cx, |state, cx| {
            state.update_status = UpdateStatus::Checking;
            cx.notify();
        });

        let current_version = env!("CARGO_PKG_VERSION").to_string();
        // Allow runtime override for testing: OPENMANGO_TEST_SHA=fake just dev
        let current_sha = std::env::var("OPENMANGO_TEST_SHA")
            .unwrap_or_else(|_| env!("OPENMANGO_GIT_SHA").to_string());

        log::info!(
            "Update check: version={current_version}, sha={}, arch={ARCH_SUFFIX}",
            &current_sha[..7.min(current_sha.len())]
        );

        // GPUI uses smol, but reqwest/hyper needs Tokio — spin up a one-shot runtime
        let task = cx.background_spawn(async move {
            tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
                let client = reqwest::Client::builder()
                    .user_agent(format!("OpenMango/{current_version}"))
                    .build()?;

                // Check both channels in parallel
                let (stable_resp, nightly_resp) = futures::join!(
                    client.get(format!("{GITHUB_API}/latest")).send(),
                    client.get(format!("{GITHUB_API}/tags/nightly")).send(),
                );

                // --- Stable channel ---
                let stable = match stable_resp {
                    Ok(r) if r.status().is_success() => {
                        let json: serde_json::Value = r.json().await?;
                        let tag = json["tag_name"].as_str().unwrap_or_default();
                        let version_str = tag.strip_prefix('v').unwrap_or(tag);
                        let remote: semver::Version =
                            version_str.parse().ok().unwrap_or(semver::Version::new(0, 0, 0));
                        let local: semver::Version = current_version.parse()?;
                        if remote > local {
                            find_asset_url(&json).map(|url| ReleaseCandidate {
                                version: version_str.to_string(),
                                download_url: url,
                            })
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                // If there's a newer stable release, always prefer it
                if let Some(stable) = stable {
                    log::info!("Update found: stable v{}", stable.version);
                    return Ok::<Option<(String, String)>, anyhow::Error>(Some((
                        stable.version,
                        stable.download_url,
                    )));
                }

                // --- Nightly channel ---
                // Only check nightly if we have a build SHA
                if !current_sha.is_empty()
                    && let Ok(r) = nightly_resp
                    && r.status().is_success()
                {
                    let json: serde_json::Value = r.json().await?;
                    let body = json["body"].as_str().unwrap_or_default();
                    if let Some(remote_sha) = parse_nightly_sha(body) {
                        log::info!(
                            "Nightly check: local={}, remote={}",
                            &current_sha[..7.min(current_sha.len())],
                            &remote_sha[..7.min(remote_sha.len())]
                        );
                        if remote_sha != current_sha
                            && let Some(url) = find_asset_url(&json)
                        {
                            let short_sha = &remote_sha[..7.min(remote_sha.len())];
                            log::info!("Update found: nightly ({short_sha})");
                            return Ok(Some((format!("nightly ({})", short_sha), url)));
                        }
                    }
                }

                Ok(None)
            })
        });

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<Option<(String, String)>, anyhow::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(Some((version, download_url))) => {
                        state.update(cx, |state, cx| {
                            state.update_status =
                                UpdateStatus::Available { version: version.clone(), download_url };
                            let event = AppEvent::UpdateAvailable { version };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                    Ok(None) => {
                        state.update(cx, |state, cx| {
                            state.update_status = UpdateStatus::Idle;
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::debug!("Update check failed: {e}");
                        state.update(cx, |state, cx| {
                            state.update_status = UpdateStatus::Idle;
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }

    /// Download the update zip with progress tracking.
    pub fn download_update(state: Entity<AppState>, cx: &mut App) {
        use futures::StreamExt as _;

        let (version, download_url) = {
            let s = state.read(cx);
            match &s.update_status {
                UpdateStatus::Available { version, download_url } => {
                    (version.clone(), download_url.clone())
                }
                _ => return,
            }
        };

        state.update(cx, |state, cx| {
            state.update_status =
                UpdateStatus::Downloading { version: version.clone(), progress_pct: 0 };
            cx.notify();
        });

        let (progress_tx, mut progress_rx) = futures::channel::mpsc::unbounded::<u8>();

        let download_url_clone = download_url.clone();
        let task = cx.background_spawn({
            let version = version.clone();
            async move {
                tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(
                    async {
                        let client = reqwest::Client::builder()
                            .user_agent(format!("OpenMango/{}", env!("CARGO_PKG_VERSION")))
                            .build()?;

                        let resp =
                            client.get(&download_url_clone).send().await?.error_for_status()?;
                        let total = resp.content_length().unwrap_or(0);

                        // Prepare cache dir
                        let cache_dir = dirs::cache_dir()
                            .unwrap_or_else(|| PathBuf::from("/tmp"))
                            .join("com.openmango.app");
                        std::fs::create_dir_all(&cache_dir)?;
                        let zip_path = cache_dir.join("OpenMango-update.zip");

                        let mut stream = resp.bytes_stream();
                        let mut file = std::fs::File::create(&zip_path)?;
                        let mut downloaded: u64 = 0;
                        let mut last_pct: u8 = 0;

                        use std::io::Write;
                        while let Some(chunk) = stream.next().await {
                            let chunk = chunk?;
                            file.write_all(&chunk)?;
                            downloaded += chunk.len() as u64;
                            let pct = if total > 0 {
                                ((downloaded * 100) / total).min(100) as u8
                            } else {
                                // No Content-Length: estimate indeterminate progress
                                // Cap at 99 until we know it's truly done
                                99u8.min((downloaded / (1024 * 100)) as u8)
                            };
                            if pct != last_pct {
                                last_pct = pct;
                                let _ = progress_tx.unbounded_send(pct);
                            }
                        }

                        Ok::<(PathBuf, String), anyhow::Error>((zip_path, version))
                    },
                )
            }
        });

        // Forward progress updates to UI
        cx.spawn({
            let state = state.clone();
            let version = version.clone();
            async move |cx: &mut gpui::AsyncApp| {
                while let Some(pct) = progress_rx.next().await {
                    let version = version.clone();
                    let _ = cx.update(|cx| {
                        state.update(cx, |state, cx| {
                            state.update_status =
                                UpdateStatus::Downloading { version, progress_pct: pct };
                            cx.notify();
                        });
                    });
                }
            }
        })
        .detach();

        cx.spawn({
            let state = state.clone();
            let version_for_err = version.clone();

            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(PathBuf, String), anyhow::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok((zip_path, version)) => {
                        state.update(cx, |state, cx| {
                            state.update_status =
                                UpdateStatus::ReadyToInstall { version, zip_path };
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Update download failed: {e}");
                        // Clean up partial download
                        let cache_dir = dirs::cache_dir()
                            .unwrap_or_else(|| PathBuf::from("/tmp"))
                            .join("com.openmango.app");
                        let _ = std::fs::remove_file(cache_dir.join("OpenMango-update.zip"));

                        let version = version_for_err.clone();
                        let url = download_url.clone();
                        state.update(cx, |state, cx| {
                            // Set back to Available so user can retry
                            state.update_status =
                                UpdateStatus::Available { version, download_url: url };
                            state.set_status_message(Some(crate::state::StatusMessage::error(
                                format!("Update failed: {e}"),
                            )));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }

    /// Replace the running app bundle with the downloaded update and relaunch.
    pub fn install_update(state: Entity<AppState>, cx: &mut App) {
        let zip_path = {
            let s = state.read(cx);
            match &s.update_status {
                UpdateStatus::ReadyToInstall { zip_path, .. } => zip_path.clone(),
                _ => return,
            }
        };

        // Determine target .app path:
        // If running from .app bundle → replace in-place
        // If running from cargo run → install to /Applications/
        let app_bundle = match std::env::current_exe() {
            Ok(exe) => match exe.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) {
                Some(bundle) if bundle.extension().is_some_and(|e| e == "app") => {
                    bundle.to_path_buf()
                }
                _ => {
                    log::info!("Not running from .app bundle, will install to /Applications/");
                    PathBuf::from("/Applications/OpenMango.app")
                }
            },
            Err(e) => {
                log::error!("Cannot get current exe: {e}");
                PathBuf::from("/Applications/OpenMango.app")
            }
        };

        // Run extract + swap on a background thread to avoid blocking the UI
        let task = cx.background_spawn({
            let app_bundle = app_bundle.clone();
            let zip_path = zip_path.clone();
            async move { Self::do_install(&app_bundle, &zip_path) }
        });

        cx.spawn({
            let state = state.clone();
            let app_bundle = app_bundle.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), anyhow::Error> = task.await;
                match result {
                    Ok(()) => {
                        // Relaunch and quit
                        let _ = Command::new("open").arg("-n").arg(&app_bundle).spawn();
                        let _ = cx.update(|cx| cx.quit());
                    }
                    Err(e) => {
                        log::error!("Install failed: {e}");
                        let _ = cx.update(|cx| {
                            state.update(cx, |state, cx| {
                                state.set_status_message(Some(crate::state::StatusMessage::error(
                                    format!("Install failed: {e}"),
                                )));
                                cx.notify();
                            });
                        });
                    }
                }
            }
        })
        .detach();
    }

    /// Extract zip, swap .app bundle, clean up. Runs on a background thread.
    fn do_install(
        app_bundle: &std::path::Path,
        zip_path: &std::path::Path,
    ) -> Result<(), anyhow::Error> {
        let parent = app_bundle.parent().unwrap_or_else(|| std::path::Path::new("/tmp"));
        let temp_dir = parent.join(".openmango-update-tmp");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir)?;

        // Extract the zip
        let extract =
            Command::new("ditto").args(["-x", "-k"]).arg(zip_path).arg(&temp_dir).output()?;

        if !extract.status.success() {
            let _ = std::fs::remove_dir_all(&temp_dir);
            anyhow::bail!("ditto extraction failed: {}", String::from_utf8_lossy(&extract.stderr));
        }

        // Find the .app in the extracted directory
        let extracted_app = std::fs::read_dir(&temp_dir)?.find_map(|e| {
            let path = e.ok()?.path();
            if path.extension().is_some_and(|ext| ext == "app") { Some(path) } else { None }
        });

        let Some(extracted_app) = extracted_app else {
            let _ = std::fs::remove_dir_all(&temp_dir);
            anyhow::bail!("No .app found in extracted update");
        };

        // If existing .app exists, back it up first
        if app_bundle.exists() {
            let backup = app_bundle.with_extension("app.bak");
            let _ = std::fs::remove_dir_all(&backup);

            let mv_backup = Command::new("mv").arg(app_bundle).arg(&backup).output()?;
            if !mv_backup.status.success() {
                let _ = std::fs::remove_dir_all(&temp_dir);
                anyhow::bail!("Failed to move current app to backup");
            }

            let mv_new = Command::new("mv").arg(&extracted_app).arg(app_bundle).output()?;
            if !mv_new.status.success() {
                // Restore backup
                let _ = Command::new("mv").arg(&backup).arg(app_bundle).output();
                let _ = std::fs::remove_dir_all(&temp_dir);
                anyhow::bail!("Failed to move extracted app into place");
            }

            let _ = std::fs::remove_dir_all(&backup);
        } else {
            // Fresh install (e.g. first time from cargo run → /Applications/)
            let mv_new = Command::new("mv").arg(&extracted_app).arg(app_bundle).output()?;
            if !mv_new.status.success() {
                let _ = std::fs::remove_dir_all(&temp_dir);
                anyhow::bail!("Failed to move app to {}", app_bundle.display());
            }
        }

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = std::fs::remove_file(zip_path);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nightly_sha_from_body() {
        let body = "Automated nightly build from main branch.\n\n**Commit:** abc1234def5678\n\n> This is a development build.";
        assert_eq!(parse_nightly_sha(body), Some("abc1234def5678"));
    }

    #[test]
    fn parse_nightly_sha_missing() {
        assert_eq!(parse_nightly_sha("no commit here"), None);
    }

    #[test]
    fn parse_nightly_sha_too_short() {
        let body = "**Commit:** abc";
        assert_eq!(parse_nightly_sha(body), None);
    }
}
