use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use futures::StreamExt as _;
use minisign_verify::{PublicKey, Signature};
use serde_json::Value;

use crate::state::app_state::updater::{
    LinuxUpdateManifest, RELEASES_URL, UpdateChannel, UpdateRelease,
};

pub(in crate::state::commands::updater) fn public_key() -> Result<PublicKey> {
    let key = option_env!("OPENMANGO_LINUX_UPDATE_PUBLIC_KEY")
        .filter(|key| !key.trim().is_empty())
        .context("This build has no Linux update verification key. Install a signed official release to enable automatic updates")?;
    PublicKey::from_base64(key.trim()).context("The Linux update verification key is invalid")
}

pub(super) async fn candidate(
    release: &Value,
    channel: UpdateChannel,
    version: String,
) -> Result<UpdateRelease> {
    let key = public_key()?;
    let architecture = std::env::consts::ARCH;
    let suffix = match architecture {
        "x86_64" => "linux-x86_64.AppImage",
        "aarch64" => "linux-arm64.AppImage",
        _ => anyhow::bail!("Automatic updates are unavailable for this Linux architecture"),
    };
    let assets = release["assets"].as_array().context("The release contains no assets")?;
    let artifact = assets
        .iter()
        .find(|asset| {
            asset["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("OpenMango-") && name.ends_with(suffix))
        })
        .context("The release has no AppImage for this Linux architecture")?;
    let name = artifact["name"].as_str().context("The AppImage has no filename")?;
    let metadata_url = asset_url(assets, &format!("{name}.json"))?;
    let signature_url = asset_url(assets, &format!("{name}.json.minisig"))?;
    let client = super::client(Duration::from_secs(30))?;
    let metadata = read_limited(&client, &metadata_url, 64 * 1024).await?;
    let signature = read_limited(&client, &signature_url, 8 * 1024).await?;
    let manifest = decode_manifest(&metadata, std::str::from_utf8(&signature)?, &key)?;
    let commit = if channel == UpdateChannel::Nightly {
        super::nightly_sha(release["body"].as_str().unwrap_or_default())
            .context("The nightly release has no commit")?
    } else {
        ""
    };
    validate_manifest(&manifest, name, channel, &version, commit, architecture)?;
    ensure!(
        artifact["size"].as_u64() == Some(manifest.size),
        "The AppImage size does not match its signed metadata"
    );
    if let Some(digest) =
        artifact["digest"].as_str().and_then(|digest| digest.strip_prefix("sha256:"))
    {
        ensure!(
            digest == manifest.sha256,
            "The AppImage digest does not match its signed metadata"
        );
    }
    Ok(UpdateRelease {
        channel,
        version,
        release_url: match channel {
            UpdateChannel::Stable => format!("{RELEASES_URL}/latest"),
            UpdateChannel::Nightly => format!("{RELEASES_URL}/tag/nightly"),
        },
        download_url: asset_url(assets, name)?,
        checksum_url: None,
        sha256: Some(manifest.sha256.clone()),
        size: manifest.size,
        linux_manifest: Some(manifest),
    })
}

fn asset_url(assets: &[Value], name: &str) -> Result<String> {
    let id = assets
        .iter()
        .find(|asset| asset["name"].as_str() == Some(name))
        .and_then(|asset| asset["id"].as_u64())
        .with_context(|| {
            format!("The release is missing {name}; check again after publication finishes")
        })?;
    Ok(format!("{}/releases/assets/{id}", super::REPOSITORY_API))
}

async fn read_limited(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await?
        .error_for_status()?;
    ensure!(
        response.content_length().is_none_or(|size| size <= limit as u64),
        "Update metadata is too large"
    );
    let mut result = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        ensure!(result.len().saturating_add(chunk.len()) <= limit, "Update metadata is too large");
        result.extend_from_slice(&chunk);
    }
    Ok(result)
}

fn decode_manifest(bytes: &[u8], signature: &str, key: &PublicKey) -> Result<LinuxUpdateManifest> {
    let signature = Signature::decode(signature).context("Invalid Linux update signature")?;
    key.verify(bytes, &signature, false)
        .context("The Linux update signature does not match the trusted publisher")?;
    serde_json::from_slice(bytes).context("The signed Linux update metadata is invalid")
}

fn validate_manifest(
    manifest: &LinuxUpdateManifest,
    filename: &str,
    channel: UpdateChannel,
    version: &str,
    commit: &str,
    architecture: &str,
) -> Result<()> {
    ensure!(manifest.schema == 1 && manifest.os == "linux", "Unsupported Linux update metadata");
    ensure!(
        manifest.arch == architecture && manifest.channel == channel,
        "The signed update targets a different platform or channel"
    );
    let suffix = if architecture == "aarch64" { "arm64" } else { "x86_64" };
    let expected_name = format!("OpenMango-{}-linux-{suffix}.AppImage", manifest.version);
    ensure!(
        manifest.filename == filename && filename == expected_name,
        "The signed AppImage filename does not match the release"
    );
    let _: semver::Version = manifest.version.parse().context("Invalid signed update version")?;
    ensure!(
        matches!(manifest.commit.len(), 40 | 64) && super::valid_sha(&manifest.commit),
        "Invalid signed build identity"
    );
    match channel {
        UpdateChannel::Stable => ensure!(
            manifest.version == version,
            "The signed update version does not match the release"
        ),
        UpdateChannel::Nightly => ensure!(
            manifest.commit == commit,
            "The signed nightly build does not match the release"
        ),
    }
    ensure!(manifest.size > 0, "The signed AppImage is empty");
    ensure!(
        super::parse_checksum(&manifest.sha256)? == manifest.sha256 && manifest.sha256.len() == 64,
        "Invalid signed AppImage digest"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_checked_before_metadata_is_trusted() {
        // A disposable test key; its private half was destroyed after signing this fixture.
        let key =
            PublicKey::from_base64("RWRbmDuYwD0W5UqMQVhCKumigCaxeXZUQl+opEMGDnuxlt+5TgBTZJgS")
                .unwrap();
        let bytes = include_bytes!("../../../../../tests/fixtures/linux-update.json");
        let signature = include_str!("../../../../../tests/fixtures/linux-update.json.minisig");
        let manifest = decode_manifest(bytes, signature, &key).unwrap();
        assert_eq!(manifest.version, "0.2.1");
        let mut tampered = bytes.to_vec();
        tampered[0] = b'[';
        assert!(decode_manifest(&tampered, signature, &key).is_err());
        assert!(
            decode_manifest(
                bytes,
                &signature.replace("timestamp:1789217508", "timestamp:1789217509"),
                &key
            )
            .is_err()
        );
        let other_key =
            PublicKey::from_base64("RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3")
                .unwrap();
        assert!(decode_manifest(bytes, signature, &other_key).is_err());
    }

    #[tokio::test]
    async fn metadata_limit_covers_declared_and_chunked_lengths() {
        use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

        for response in [
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nhel\r\n2\r\nlo\r\n0\r\n\r\n",
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                loop {
                    line.clear();
                    assert!(stream.read_line(&mut line).await.unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                }
                stream.get_mut().write_all(response.as_bytes()).await.unwrap();
            });
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();
            assert!(
                read_limited(&client, &url, 4).await.unwrap_err().to_string().contains("too large")
            );
            server.await.unwrap();
        }
    }

    #[test]
    fn signed_identity_binds_channel_architecture_and_artifact() {
        let mut manifest = LinuxUpdateManifest {
            schema: 1,
            os: "linux".into(),
            arch: "aarch64".into(),
            channel: UpdateChannel::Stable,
            version: "0.2.1".into(),
            commit: "a".repeat(40),
            filename: "OpenMango-0.2.1-linux-arm64.AppImage".into(),
            size: 123,
            sha256: "b".repeat(64),
        };
        let name = manifest.filename.clone();
        validate_manifest(&manifest, &name, UpdateChannel::Stable, "0.2.1", "", "aarch64").unwrap();
        assert!(
            validate_manifest(&manifest, &name, UpdateChannel::Stable, "0.2.1", "", "x86_64")
                .is_err()
        );
        assert!(
            validate_manifest(&manifest, &name, UpdateChannel::Stable, "0.2.2", "", "aarch64")
                .is_err()
        );
        manifest.channel = UpdateChannel::Nightly;
        assert!(
            validate_manifest(&manifest, &name, UpdateChannel::Stable, "0.2.1", "", "aarch64")
                .is_err()
        );
        assert!(
            validate_manifest(
                &manifest,
                &name,
                UpdateChannel::Nightly,
                "aaaaaaa",
                &"c".repeat(40),
                "aarch64"
            )
            .is_err()
        );
    }
}
