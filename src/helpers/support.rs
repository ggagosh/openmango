use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::state::AppState;

const MAX_INCLUDED_LOG_BYTES: u64 = 2 * 1024 * 1024;

pub fn app_log_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("com.openmango.app")
        .join("logs")
        .join("openmango.log")
}

pub fn init_logging() {
    let path = app_log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 5 * 1024 * 1024) {
        let rotated = path.with_extension("log.1");
        let _ = fs::remove_file(&rotated);
        let _ = fs::rename(&path, rotated);
    }
    let file = OpenOptions::new().create(true).append(true).open(&path).ok();
    let mut builder = env_logger::Builder::from_env(env_logger::Env::default());
    builder.target(env_logger::Target::Pipe(Box::new(TeeLogWriter { file })));
    builder.init();
}

struct TeeLogWriter {
    file: Option<fs::File>,
}

impl Write for TeeLogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(bytes);
        if let Some(file) = &mut self.file {
            file.write_all(bytes)?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        if let Some(file) = &mut self.file {
            file.flush()?;
        }
        Ok(())
    }
}

pub fn export_support_bundle(state: &AppState, destination: &Path) -> anyhow::Result<()> {
    let mut contents = String::new();
    writeln!(contents, "OpenMango support bundle")?;
    writeln!(contents, "Version: {}", env!("CARGO_PKG_VERSION"))?;
    writeln!(contents, "Git SHA: {}", env!("OPENMANGO_GIT_SHA"))?;
    writeln!(contents, "OS: {}", std::env::consts::OS)?;
    writeln!(contents, "Architecture: {}", std::env::consts::ARCH)?;
    writeln!(contents, "Log location: {}", app_log_path().display())?;
    writeln!(contents, "Auto update: {}", state.settings.auto_update)?;
    writeln!(contents, "AI enabled: {}", state.settings.ai.enabled)?;
    writeln!(contents, "AI provider: {}", state.settings.ai.provider.label())?;
    writeln!(contents, "AI model: {}", state.settings.ai.model)?;
    writeln!(contents, "\nConnections (credentials removed):")?;
    for connection in &state.connections {
        writeln!(
            contents,
            "- {} | {} | read_only={} | ssh={} | socks5={}",
            connection.name,
            crate::helpers::strip_uri_secrets(&connection.uri),
            connection.read_only,
            connection.ssh.as_ref().is_some_and(|ssh| ssh.enabled),
            connection.proxy.as_ref().is_some_and(|proxy| proxy.enabled),
        )?;
    }

    writeln!(contents, "\nRecent log (up to 2 MiB):")?;
    match read_log_tail(&app_log_path()) {
        Ok(log) => contents.push_str(&redact_log_secrets(state, log)),
        Err(error) => writeln!(contents, "[log unavailable: {error}]")?,
    }

    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(contents.as_bytes())?;
    staged.as_file().sync_all()?;
    staged.persist(destination).map_err(|error| error.error)?;
    Ok(())
}

fn redact_log_secrets(state: &AppState, mut log: String) -> String {
    let mut secrets = vec![state.settings.ai.api_key.clone()];
    for connection in &state.connections {
        let uri_secrets = crate::helpers::extract_uri_secrets(&connection.uri);
        secrets.extend(
            [
                uri_secrets.password,
                uri_secrets.tls_certificate_key_file_password,
                uri_secrets.proxy_password,
                uri_secrets.aws_session_token,
                connection.ssh.as_ref().and_then(|ssh| ssh.password.clone()),
                connection.ssh.as_ref().and_then(|ssh| ssh.identity_passphrase.clone()),
                connection.proxy.as_ref().and_then(|proxy| proxy.password.clone()),
            ]
            .into_iter()
            .flatten(),
        );
    }
    for secret in secrets.into_iter().filter(|secret| !secret.is_empty()) {
        log = log.replace(&secret, "*****");
    }
    log
}

fn read_log_tail(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let length = file.metadata()?.len();
    if length > MAX_INCLUDED_LOG_BYTES {
        file.seek(SeekFrom::Start(length - MAX_INCLUDED_LOG_BYTES))?;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_bundle_omits_connection_and_ai_secrets() {
        let mut state = AppState::new();
        state.settings.ai.api_key = "ai-secret".to_string();
        state.connections.push(crate::models::SavedConnection::new(
            "private".to_string(),
            "mongodb://user:db-secret@localhost:27017/admin".to_string(),
        ));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("support.txt");

        export_support_bundle(&state, &path).unwrap();

        let exported = fs::read_to_string(path).unwrap();
        assert!(!exported.contains("ai-secret"));
        assert!(!exported.contains("db-secret"));
        assert!(exported.contains("Log location:"));
        let redacted = redact_log_secrets(&state, "ai-secret db-secret harmless".to_string());
        assert_eq!(redacted, "***** ***** harmless");
    }
}
