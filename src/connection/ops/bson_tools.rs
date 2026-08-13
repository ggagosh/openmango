//! BSON export/import using mongodump/mongorestore tools.

use std::io::Write as _;
use std::path::Path;
use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use tempfile::NamedTempFile;

use crate::connection::ConnectionManager;
use crate::connection::tools::{mongodump_path, mongorestore_path};
use crate::connection::types::{
    BsonOutputFormat, BsonToolProgress, BsonToolRunOutcome, CancellationToken,
};
use crate::error::{Error, Result};

/// Strip the database name from a MongoDB connection URI so it doesn't
/// conflict with the explicit `--db` flag passed to mongodump/mongorestore.
///
/// `mongodb://user:pass@host:27017/admin?authSource=admin`
/// becomes `mongodb://user:pass@host:27017/?authSource=admin`
fn strip_uri_database(uri: &str) -> String {
    // Find the scheme separator
    let authority_start = match uri.find("://") {
        Some(i) => i + 3,
        None => return uri.to_string(),
    };

    // Find the first '/' after the authority (host:port) section.
    // The database name sits between this '/' and the next '?' (or end).
    let rest = &uri[authority_start..];
    let slash_pos = match rest.find('/') {
        Some(i) => authority_start + i,
        None => return uri.to_string(), // no database component
    };

    // Everything after the slash up to '?' is the database name — remove it.
    let after_slash = &uri[slash_pos + 1..];
    let query_start = after_slash.find('?');

    let database = query_start.map(|index| &after_slash[..index]).unwrap_or(after_slash);
    let mut stripped = match query_start {
        Some(qi) => format!("{}/{}", &uri[..slash_pos], &after_slash[qi..]),
        None => format!("{}/", &uri[..slash_pos]),
    };
    let has_auth_source = stripped
        .split_once('?')
        .map(|(_, query)| {
            query.split('&').any(|pair| {
                pair.split_once('=')
                    .map(|(key, _)| key)
                    .unwrap_or(pair)
                    .eq_ignore_ascii_case("authSource")
            })
        })
        .unwrap_or(false);
    if !database.is_empty() && !has_auth_source {
        stripped.push(if stripped.contains('?') { '&' } else { '?' });
        stripped.push_str("authSource=");
        stripped.push_str(database);
    }
    stripped
}

struct SecureToolCommand {
    command: Command,
    _config: NamedTempFile,
}

fn secure_tool_command(program: &Path, connection_string: &str) -> Result<SecureToolCommand> {
    let mut config =
        tempfile::Builder::new().prefix("openmango-bson-").suffix(".yml").tempfile()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        config.as_file().set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    let uri = strip_uri_database(connection_string);
    writeln!(config, "uri: '{}'", uri.replace('\'', "''"))?;
    config.flush()?;

    let mut command = Command::new(program);
    command.arg("--config").arg(config.path());
    Ok(SecureToolCommand { command, _config: config })
}

fn tool_safe_uri(uri: &str) -> String {
    let stripped = crate::helpers::strip_uri_secrets(uri);
    let Some(authority_start) = stripped.find("://").map(|index| index + 3) else {
        return stripped;
    };
    let authority_end = stripped[authority_start..]
        .find(['/', '?', '#'])
        .map(|index| authority_start + index)
        .unwrap_or(stripped.len());
    let authority = &stripped[authority_start..authority_end];
    let Some(userinfo_end) = authority.rfind('@') else {
        return stripped;
    };
    format!(
        "{}{}{}",
        &stripped[..authority_start],
        &authority[userinfo_end + 1..],
        &stripped[authority_end..]
    )
}

fn sanitize_mongodb_uris(message: &str) -> String {
    let mut sanitized = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(start) =
        [rest.find("mongodb://"), rest.find("mongodb+srv://")].into_iter().flatten().min()
    {
        sanitized.push_str(&rest[..start]);
        let uri_and_after = &rest[start..];
        let end = uri_and_after
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(character, '\'' | '"' | '<' | '>' | ')' | ']' | '}')
            })
            .unwrap_or(uri_and_after.len());
        sanitized.push_str(&tool_safe_uri(&uri_and_after[..end]));
        rest = &uri_and_after[end..];
    }
    sanitized.push_str(rest);
    sanitized
}

fn percent_decode_secret(secret: &str) -> Option<String> {
    let bytes = secret.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut changed = false;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16)?;
            let low = (bytes[index + 2] as char).to_digit(16)?;
            decoded.push(((high << 4) | low) as u8);
            index += 3;
            changed = true;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    changed.then(|| String::from_utf8_lossy(&decoded).into_owned())
}

enum ToolProcessResult {
    Exited { status: ExitStatus, error_lines: Vec<String> },
    Cancelled { termination_succeeded: bool },
}

fn monitor_tool_process<F, P>(
    child: &mut Child,
    stderr: ChildStderr,
    cancellation: &CancellationToken,
    parse_progress: P,
    on_progress: &F,
) -> Result<ToolProcessResult>
where
    F: Fn(BsonToolProgress),
    P: Fn(&str) -> Option<BsonToolProgress>,
{
    let (line_tx, line_rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        use std::io::{BufRead as _, BufReader};
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(std::result::Result::ok) {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut error_lines = Vec::new();
    let process_line = |line: String, error_lines: &mut Vec<String>| {
        if let Some(progress) = parse_progress(&line) {
            on_progress(progress);
        } else if line.contains("error") || line.contains("Error") || line.contains("failed") {
            error_lines.push(line);
        }
    };

    loop {
        if cancellation.is_cancelled() {
            if child.try_wait()?.is_some() {
                return Ok(ToolProcessResult::Cancelled { termination_succeeded: true });
            }
            let killed = child.kill().is_ok();
            let waited = child.wait().is_ok();
            return Ok(ToolProcessResult::Cancelled { termination_succeeded: killed && waited });
        }

        if let Some(status) = child.try_wait()? {
            while let Ok(line) = line_rx.try_recv() {
                process_line(line, &mut error_lines);
            }
            return Ok(ToolProcessResult::Exited { status, error_lines });
        }

        match line_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => process_line(line, &mut error_lines),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let status = child.wait()?;
                return Ok(ToolProcessResult::Exited { status, error_lines });
            }
        }
    }
}

fn sanitize_tool_error(message: &str, connection_string: &str) -> String {
    let uri = strip_uri_database(connection_string);
    let mut sanitized = sanitize_mongodb_uris(message);
    sanitized = sanitized.replace(connection_string, &tool_safe_uri(connection_string));
    sanitized = sanitized.replace(&uri, &tool_safe_uri(&uri));
    let secrets = crate::helpers::extract_uri_secrets(connection_string);
    for secret in [
        secrets.password,
        secrets.tls_certificate_key_file_password,
        secrets.proxy_password,
        secrets.aws_session_token,
    ]
    .into_iter()
    .flatten()
    .filter(|secret| !secret.is_empty())
    {
        sanitized = sanitized.replace(&secret, "*****");
        if let Some(decoded) = percent_decode_secret(&secret) {
            sanitized = sanitized.replace(&decoded, "*****");
        }
    }
    sanitized
}

impl ConnectionManager {
    /// Export a database to BSON format using mongodump (runs synchronously).
    /// Prefer `export_database_bson_with_progress` for progress tracking.
    #[allow(dead_code)]
    pub fn export_database_bson(
        &self,
        connection_string: &str,
        database: &str,
        output_format: BsonOutputFormat,
        path: &Path,
        gzip: bool,
        exclude_collections: &[String],
    ) -> Result<()> {
        let mongodump = mongodump_path().ok_or_else(|| {
            Error::ToolNotFound(
                "mongodump not found. Run 'just download-tools' or install MongoDB Database Tools."
                    .into(),
            )
        })?;

        let mut secure_command = secure_tool_command(&mongodump, connection_string)?;
        let cmd = &mut secure_command.command;
        cmd.arg("--db").arg(database);

        if gzip {
            cmd.arg("--gzip");
        }

        // Add exclude collection flags
        for collection in exclude_collections {
            cmd.arg("--excludeCollection").arg(collection);
        }

        let final_path = match output_format {
            BsonOutputFormat::Archive
                if path.extension().is_none_or(|extension| extension != "archive") =>
            {
                path.with_extension("archive")
            }
            _ => path.to_path_buf(),
        };
        let parent = final_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let staging =
            tempfile::Builder::new().prefix(".openmango-bson-export-").tempdir_in(parent)?;
        let staged_path = staging.path().join(match output_format {
            BsonOutputFormat::Archive => "export.archive",
            BsonOutputFormat::Folder => "export",
        });
        match output_format {
            BsonOutputFormat::Folder => {
                cmd.arg("--out").arg(&staged_path);
            }
            BsonOutputFormat::Archive => {
                cmd.arg(format!("--archive={}", staged_path.display()));
            }
        }

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Parse(format!(
                "mongodump failed: {}",
                sanitize_tool_error(&stderr, connection_string)
            )));
        }

        crate::connection::ops::export::promote_export_path(&staged_path, &final_path)?;
        Ok(())
    }

    /// Import a database from BSON format using mongorestore (runs synchronously).
    /// Prefer `import_database_bson_with_progress` for progress tracking.
    #[allow(dead_code)]
    pub fn import_database_bson(
        &self,
        connection_string: &str,
        database: &str,
        path: &Path,
        drop_before: bool,
    ) -> Result<()> {
        let mongorestore = mongorestore_path().ok_or_else(|| {
            Error::ToolNotFound(
                "mongorestore not found. Run 'just download-tools' or install MongoDB Database Tools."
                    .into(),
            )
        })?;

        let mut secure_command = secure_tool_command(&mongorestore, connection_string)?;
        let cmd = &mut secure_command.command;
        cmd.arg("--db").arg(database);

        if drop_before {
            cmd.arg("--drop");
        }

        // Detect if path is archive or folder
        if path.extension().map(|e| e == "archive").unwrap_or(false) {
            // --archive requires = format: --archive=/path/to/file
            cmd.arg(format!("--archive={}", path.display()));
        } else {
            // mongodump creates a subfolder with the database name
            let db_path = path.join(database);
            if db_path.exists() {
                cmd.arg("--dir").arg(&db_path);
            } else {
                cmd.arg("--dir").arg(path);
            }
        }

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Parse(format!(
                "mongorestore failed: {}",
                sanitize_tool_error(&stderr, connection_string)
            )));
        }

        Ok(())
    }

    /// Export a database to BSON format with progress tracking.
    /// The callback receives progress updates parsed from mongodump stderr.
    #[allow(clippy::too_many_arguments)]
    pub fn export_database_bson_with_progress<F>(
        &self,
        connection_string: &str,
        database: &str,
        output_format: BsonOutputFormat,
        path: &Path,
        gzip: bool,
        exclude_collections: &[String],
        cancellation: CancellationToken,
        on_progress: F,
    ) -> Result<BsonToolRunOutcome>
    where
        F: Fn(BsonToolProgress) + Send + 'static,
    {
        let mongodump = mongodump_path().ok_or_else(|| {
            Error::ToolNotFound(
                "mongodump not found. Run 'just download-tools' or install MongoDB Database Tools."
                    .into(),
            )
        })?;

        let mut secure_command = secure_tool_command(&mongodump, connection_string)?;
        let cmd = &mut secure_command.command;
        cmd.arg("--db")
            .arg(database)
            .arg("-v") // Enable verbose output for progress
            .stderr(Stdio::piped());

        if gzip {
            cmd.arg("--gzip");
        }

        for collection in exclude_collections {
            cmd.arg("--excludeCollection").arg(collection);
        }

        let final_path = match output_format {
            BsonOutputFormat::Archive
                if path.extension().is_none_or(|extension| extension != "archive") =>
            {
                path.with_extension("archive")
            }
            _ => path.to_path_buf(),
        };
        let parent = final_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let staging =
            tempfile::Builder::new().prefix(".openmango-bson-export-").tempdir_in(parent)?;
        let staged_path = staging.path().join(match output_format {
            BsonOutputFormat::Archive => "export.archive",
            BsonOutputFormat::Folder => "export",
        });
        match output_format {
            BsonOutputFormat::Folder => {
                cmd.arg("--out").arg(&staged_path);
            }
            BsonOutputFormat::Archive => {
                cmd.arg(format!("--archive={}", staged_path.display()));
            }
        }

        let mut child = cmd.spawn()?;
        let stderr = child.stderr.take().expect("stderr was piped");
        match monitor_tool_process(
            &mut child,
            stderr,
            &cancellation,
            parse_mongodump_line,
            &on_progress,
        )? {
            ToolProcessResult::Cancelled { termination_succeeded } => {
                Ok(BsonToolRunOutcome::Cancelled { termination_succeeded })
            }
            ToolProcessResult::Exited { status, error_lines } if !status.success() => {
                let error_msg = if error_lines.is_empty() {
                    "mongodump failed".to_string()
                } else {
                    format!(
                        "mongodump failed: {}",
                        sanitize_tool_error(&error_lines.join("\n"), connection_string)
                    )
                };
                Err(Error::Parse(error_msg))
            }
            ToolProcessResult::Exited { .. } => {
                if cancellation.is_cancelled() {
                    return Ok(BsonToolRunOutcome::Cancelled { termination_succeeded: true });
                }
                if output_format == BsonOutputFormat::Folder && !staged_path.join(database).exists()
                {
                    std::fs::create_dir_all(staged_path.join(database))?;
                }
                crate::connection::ops::export::promote_export_path(&staged_path, &final_path)?;
                Ok(BsonToolRunOutcome::Completed)
            }
        }
    }

    /// Import a database from BSON format with progress tracking.
    /// The callback receives (collection_name, bytes_processed, bytes_total, is_complete).
    #[allow(clippy::too_many_arguments)]
    pub fn import_database_bson_with_progress<F>(
        &self,
        connection_string: &str,
        source_database: &str,
        database: &str,
        path: &Path,
        drop_before: bool,
        cancellation: CancellationToken,
        on_progress: F,
    ) -> Result<BsonToolRunOutcome>
    where
        F: Fn(BsonToolProgress) + Send + 'static,
    {
        let mongorestore = mongorestore_path().ok_or_else(|| {
            Error::ToolNotFound(
                "mongorestore not found. Run 'just download-tools' or install MongoDB Database Tools."
                    .into(),
            )
        })?;

        let mut secure_command = secure_tool_command(&mongorestore, connection_string)?;
        let cmd = &mut secure_command.command;
        cmd.arg("-v").stderr(Stdio::piped());

        if drop_before {
            cmd.arg("--drop");
        }

        if path.extension().is_some_and(|extension| extension == "archive") {
            cmd.arg("--nsInclude").arg(format!("{source_database}.*"));
            if source_database != database {
                cmd.arg("--nsFrom")
                    .arg(format!("{source_database}.*"))
                    .arg("--nsTo")
                    .arg(format!("{database}.*"));
            }
            cmd.arg(format!("--archive={}", path.display()));
        } else {
            cmd.arg("--db").arg(database);
            let db_path = path.join(database);
            if db_path.exists() {
                cmd.arg("--dir").arg(&db_path);
            } else {
                cmd.arg("--dir").arg(path);
            }
        }

        let mut child = cmd.spawn()?;
        let stderr = child.stderr.take().expect("stderr was piped");
        match monitor_tool_process(
            &mut child,
            stderr,
            &cancellation,
            parse_mongorestore_line,
            &on_progress,
        )? {
            ToolProcessResult::Cancelled { termination_succeeded } => {
                Ok(BsonToolRunOutcome::Cancelled { termination_succeeded })
            }
            ToolProcessResult::Exited { status, error_lines } if !status.success() => {
                let error_msg = if error_lines.is_empty() {
                    "mongorestore failed".to_string()
                } else {
                    format!(
                        "mongorestore failed: {}",
                        sanitize_tool_error(&error_lines.join("\n"), connection_string)
                    )
                };
                Err(Error::Parse(error_msg))
            }
            ToolProcessResult::Exited { .. } => Ok(BsonToolRunOutcome::Completed),
        }
    }
}

/// Parse a mongodump stderr line into progress information.
/// Example lines:
/// - `2026-02-01T17:46:07.737+0400<TAB>writing sample_training.grades to /path/grades.bson`
/// - `2026-02-01T17:46:07.928+0400<TAB>[....]  sample_training.grades  0/100000  (0.0%)`
/// - `2026-02-01T17:46:10.550+0400<TAB>done dumping sample_training.routes (66985 documents)`
fn parse_mongodump_line(line: &str) -> Option<BsonToolProgress> {
    // Skip timestamp prefix (everything before first tab)
    let content = line.split('\t').nth(1)?;

    // Check for "writing <db>.<collection> to <path>"
    if content.starts_with("writing ") {
        let rest = content.strip_prefix("writing ")?;
        let collection = rest.split(" to ").next()?;
        // Extract just the collection name (after the dot)
        let coll_name = collection.split('.').nth(1).unwrap_or(collection);
        return Some(BsonToolProgress::Started { collection: coll_name.to_string() });
    }

    // Check for progress bar: "[####....]  db.collection  current/total  (percent%)"
    if content.starts_with('[') && content.contains('/') {
        // Parse: [####....]  db.collection  current/total  (percent%)
        let parts: Vec<&str> = content.split_whitespace().collect();
        if parts.len() >= 4 {
            // parts[0] = progress bar like [####....]
            // parts[1] = db.collection
            // parts[2] = current/total
            // parts[3] = (percent%)
            let collection = parts[1].split('.').nth(1).unwrap_or(parts[1]);
            let counts = parts[2];
            let percent_str = parts[3].trim_start_matches('(').trim_end_matches("%)");

            if let Some((current_str, total_str)) = counts.split_once('/') {
                let current: u64 = current_str.parse().ok()?;
                let total: u64 = total_str.parse().ok()?;
                let percent: f32 = percent_str.parse().unwrap_or(0.0);

                return Some(BsonToolProgress::Progress {
                    collection: collection.to_string(),
                    current,
                    total,
                    percent,
                });
            }
        }
    }

    // Check for "done dumping <db>.<collection> (<count> documents)"
    if content.starts_with("done dumping ") {
        let rest = content.strip_prefix("done dumping ")?;
        // Format: "db.collection (count documents)"
        let (coll_part, count_part) = rest.split_once(" (")?;
        let collection = coll_part.split('.').nth(1).unwrap_or(coll_part);
        let count_str = count_part.split_whitespace().next()?;
        let documents: u64 = count_str.parse().ok()?;

        return Some(BsonToolProgress::Completed { collection: collection.to_string(), documents });
    }

    None
}

/// Parse a mongorestore stderr line into progress information.
/// Example lines:
/// - `2026-02-01T17:46:51.029+0400<TAB>restoring test_restore.companies from /path/companies.bson`
/// - `2026-02-01T17:46:53.489+0400<TAB>[####....]  test_restore.companies  6.46MB/34.8MB  (18.6%)`
/// - `2026-02-01T17:46:55.906+0400<TAB>finished restoring test_restore.posts (500 documents, 0 failures)`
fn parse_mongorestore_line(line: &str) -> Option<BsonToolProgress> {
    // Skip timestamp prefix (everything before first tab)
    let content = line.split('\t').nth(1)?;

    // Check for "restoring <db>.<collection> from <path>"
    if content.starts_with("restoring ") {
        let rest = content.strip_prefix("restoring ")?;
        let collection = rest.split(" from ").next()?;
        let coll_name = collection.split('.').nth(1).unwrap_or(collection);
        return Some(BsonToolProgress::Started { collection: coll_name.to_string() });
    }

    // Check for progress bar: "[####....]  db.collection  size/total  (percent%)"
    // Note: mongorestore uses bytes (e.g., "6.46MB/34.8MB") instead of document counts
    if content.starts_with('[') && content.contains('/') {
        let parts: Vec<&str> = content.split_whitespace().collect();
        if parts.len() >= 4 {
            let collection = parts[1].split('.').nth(1).unwrap_or(parts[1]);
            let sizes = parts[2];
            let percent_str = parts[3].trim_start_matches('(').trim_end_matches("%)");

            if let Some((current_str, total_str)) = sizes.split_once('/') {
                // Parse size strings like "6.46MB" or "455KB" to bytes
                let current = parse_size_to_bytes(current_str);
                let total = parse_size_to_bytes(total_str);
                let percent: f32 = percent_str.parse().unwrap_or(0.0);

                return Some(BsonToolProgress::Progress {
                    collection: collection.to_string(),
                    current,
                    total,
                    percent,
                });
            }
        }
    }

    // Check for "finished restoring <db>.<collection> (<count> documents, <failures> failures)"
    if content.starts_with("finished restoring ") {
        let rest = content.strip_prefix("finished restoring ")?;
        let (coll_part, count_part) = rest.split_once(" (")?;
        let collection = coll_part.split('.').nth(1).unwrap_or(coll_part);
        let count_str = count_part.split_whitespace().next()?;
        let documents: u64 = count_str.parse().ok()?;

        return Some(BsonToolProgress::Completed { collection: collection.to_string(), documents });
    }

    None
}

/// Parse a size string like "6.46MB" or "455KB" to bytes.
fn parse_size_to_bytes(s: &str) -> u64 {
    let s = s.trim();
    if s.ends_with("GB") {
        let num: f64 = s.trim_end_matches("GB").parse().unwrap_or(0.0);
        (num * 1024.0 * 1024.0 * 1024.0) as u64
    } else if s.ends_with("MB") {
        let num: f64 = s.trim_end_matches("MB").parse().unwrap_or(0.0);
        (num * 1024.0 * 1024.0) as u64
    } else if s.ends_with("KB") {
        let num: f64 = s.trim_end_matches("KB").parse().unwrap_or(0.0);
        (num * 1024.0) as u64
    } else if s.ends_with('B') {
        s.trim_end_matches('B').parse().unwrap_or(0)
    } else {
        s.parse().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mongodump_line() {
        // Test "writing" line
        let line =
            "2026-02-01T18:00:05.658+0400\twriting sample_training.grades to /tmp/grades.bson";
        let result = parse_mongodump_line(line);
        assert!(
            matches!(result, Some(BsonToolProgress::Started { ref collection }) if collection == "grades")
        );

        // Test progress line
        let line = "2026-02-01T18:00:08.763+0400\t[........................]     sample_training.routes   101/66985  (0.2%)";
        let result = parse_mongodump_line(line);
        assert!(
            matches!(result, Some(BsonToolProgress::Progress { ref collection, current, total, .. })
            if collection == "routes" && current == 101 && total == 66985)
        );

        // Test done line
        let line =
            "2026-02-01T18:00:10.772+0400\tdone dumping sample_training.routes (66985 documents)";
        let result = parse_mongodump_line(line);
        assert!(matches!(result, Some(BsonToolProgress::Completed { ref collection, documents })
            if collection == "routes" && documents == 66985));

        // Test unrelated line
        let line = "2026-02-01T18:00:05.657+0400\tdumping up to 4 collections in parallel";
        let result = parse_mongodump_line(line);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_mongorestore_line() {
        // Test "restoring" line
        let line = "2026-02-01T17:46:51.029+0400\trestoring test_restore.companies from /path/companies.bson";
        let result = parse_mongorestore_line(line);
        assert!(
            matches!(result, Some(BsonToolProgress::Started { ref collection }) if collection == "companies")
        );

        // Test progress line (bytes)
        let line = "2026-02-01T17:46:53.489+0400\t[####....................]    test_restore.companies  6.46MB/34.8MB   (18.6%)";
        let result = parse_mongorestore_line(line);
        assert!(
            matches!(result, Some(BsonToolProgress::Progress { ref collection, .. }) if collection == "companies")
        );

        // Test finished line
        let line = "2026-02-01T17:46:55.906+0400\tfinished restoring test_restore.posts (500 documents, 0 failures)";
        let result = parse_mongorestore_line(line);
        assert!(matches!(result, Some(BsonToolProgress::Completed { ref collection, documents })
            if collection == "posts" && documents == 500));
    }

    #[test]
    fn test_parse_size_to_bytes() {
        assert_eq!(parse_size_to_bytes("1KB"), 1024);
        assert_eq!(parse_size_to_bytes("1MB"), 1024 * 1024);
        assert_eq!(parse_size_to_bytes("1GB"), 1024 * 1024 * 1024);
        assert_eq!(parse_size_to_bytes("6.46MB"), (6.46 * 1024.0 * 1024.0) as u64);
        assert_eq!(parse_size_to_bytes("455KB"), (455.0 * 1024.0) as u64);
    }

    #[test]
    fn test_strip_uri_database() {
        // URI with database and query params
        assert_eq!(
            strip_uri_database("mongodb://user:pass@host:27017/admin?authSource=admin"),
            "mongodb://user:pass@host:27017/?authSource=admin"
        );
        // URI with database, no query params
        assert_eq!(
            strip_uri_database("mongodb://host:27017/mydb"),
            "mongodb://host:27017/?authSource=mydb"
        );
        // URI without database
        assert_eq!(strip_uri_database("mongodb://host:27017"), "mongodb://host:27017");
        // SRV URI with database
        assert_eq!(
            strip_uri_database(
                "mongodb+srv://user:pass@cluster.example.com/admin?retryWrites=true"
            ),
            "mongodb+srv://user:pass@cluster.example.com/?retryWrites=true&authSource=admin"
        );
        // URI with empty database (just slash)
        assert_eq!(
            strip_uri_database("mongodb://host:27017/?authSource=admin"),
            "mongodb://host:27017/?authSource=admin"
        );
    }

    #[test]
    fn secure_tool_command_keeps_credentials_out_of_process_arguments() {
        let uri = "mongodb://user:authority-secret@host/admin?tls=true&proxyHost=127.0.0.1&proxyPort=43123&tlsCertificateKeyFilePassword=tls-secret&proxyPassword=proxy-secret";
        let secure = secure_tool_command(Path::new("/bin/echo"), uri).unwrap();
        let args = secure
            .command
            .get_args()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!args.contains("authority-secret"));
        assert!(!args.contains("tls-secret"));
        assert!(!args.contains("proxy-secret"));
        assert!(args.contains("--config"));

        let config_path = secure._config.path().to_path_buf();
        let config = std::fs::read_to_string(&config_path).unwrap();
        assert!(config.contains("authority-secret"));
        assert!(config.contains("proxyHost=127.0.0.1"));
        assert!(config.contains("proxyPort=43123"));
        assert!(config.contains("tls=true"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&config_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(secure);
        assert!(!config_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn monitor_terminates_and_waits_for_cancelled_process() {
        use std::time::Instant;

        let mut child = Command::new("sleep")
            .arg("30")
            .stderr(Stdio::piped())
            .spawn()
            .expect("sleep should start");
        let stderr = child.stderr.take().expect("stderr should be piped");
        let cancellation = CancellationToken::new();
        let cancel_from_thread = cancellation.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            cancel_from_thread.cancel();
        });

        let started = Instant::now();
        let outcome = monitor_tool_process(&mut child, stderr, &cancellation, |_| None, &|_| {})
            .expect("monitoring should succeed");

        assert!(matches!(outcome, ToolProcessResult::Cancelled { termination_succeeded: true }));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(child.try_wait().expect("wait state should be available").is_some());
    }

    #[test]
    fn bson_tool_errors_redact_credentials() {
        let uri =
            "mongodb://user:authority-secret@host/admin?tlsCertificateKeyFilePassword=tls-secret";
        let message = format!("could not connect to {}", strip_uri_database(uri));
        let sanitized = sanitize_tool_error(&message, uri);
        assert!(!sanitized.contains("authority-secret"));
        assert!(!sanitized.contains("tls-secret"));
        assert!(!sanitized.contains("user@"));
        assert!(sanitized.contains("mongodb://host"));

        let encoded = "mongodb://aws-key:sec%40ret@host/admin?proxyPassword=query%2Fsecret";
        let normalized =
            "tool failed for mongodb://aws-key@host/?proxyPassword=query%2Fsecret: sec@ret";
        let sanitized = sanitize_tool_error(normalized, encoded);
        assert!(!sanitized.contains("aws-key@"));
        assert!(!sanitized.contains("query%2Fsecret"));
        assert!(!sanitized.contains("sec@ret"));
    }
}
