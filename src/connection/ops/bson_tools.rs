//! BSON export/import using mongodump/mongorestore tools.

use std::path::Path;
use std::process::Command;

use crate::connection::ConnectionManager;
use crate::connection::tools::{mongodump_path, mongorestore_path};
use crate::connection::types::{BsonOutputFormat, BsonToolProgress};
use crate::error::{Error, Result};

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

        let mut cmd = Command::new(&mongodump);
        cmd.arg("--uri").arg(connection_string).arg("--db").arg(database);

        if gzip {
            cmd.arg("--gzip");
        }

        // Add exclude collection flags
        for collection in exclude_collections {
            cmd.arg("--excludeCollection").arg(collection);
        }

        match output_format {
            BsonOutputFormat::Folder => {
                cmd.arg("--out").arg(path);
            }
            BsonOutputFormat::Archive => {
                let archive_path = if path.extension().map(|e| e == "archive").unwrap_or(false) {
                    path.to_path_buf()
                } else {
                    path.with_extension("archive")
                };
                // --archive requires = format: --archive=/path/to/file
                cmd.arg(format!("--archive={}", archive_path.display()));
            }
        }

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Parse(format!("mongodump failed: {}", stderr)));
        }

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

        let mut cmd = Command::new(&mongorestore);
        cmd.arg("--uri").arg(connection_string).arg("--db").arg(database);

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
            return Err(Error::Parse(format!("mongorestore failed: {}", stderr)));
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
        on_progress: F,
    ) -> Result<()>
    where
        F: Fn(BsonToolProgress) + Send + 'static,
    {
        use std::process::Stdio;

        let mongodump = mongodump_path().ok_or_else(|| {
            Error::ToolNotFound(
                "mongodump not found. Run 'just download-tools' or install MongoDB Database Tools."
                    .into(),
            )
        })?;

        let mut cmd = Command::new(&mongodump);
        cmd.arg("--uri")
            .arg(connection_string)
            .arg("--db")
            .arg(database)
            .arg("-v") // Enable verbose output for progress
            .stderr(Stdio::piped());

        if gzip {
            cmd.arg("--gzip");
        }

        for collection in exclude_collections {
            cmd.arg("--excludeCollection").arg(collection);
        }

        match output_format {
            BsonOutputFormat::Folder => {
                cmd.arg("--out").arg(path);
            }
            BsonOutputFormat::Archive => {
                let archive_path = if path.extension().map(|e| e == "archive").unwrap_or(false) {
                    path.to_path_buf()
                } else {
                    path.with_extension("archive")
                };
                // --archive requires = format: --archive=/path/to/file
                cmd.arg(format!("--archive={}", archive_path.display()));
            }
        }

        let mut child = cmd.spawn()?;
        let stderr = child.stderr.take().expect("stderr was piped");

        // Read stderr in a separate thread to avoid blocking
        // mongodump outputs progress to stderr, and we need to parse it in real-time
        let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(std::result::Result::ok) {
                if line_tx.send(line).is_err() {
                    break;
                }
            }
        });

        // Process lines as they come in, collecting non-progress lines for error reporting
        let mut error_lines: Vec<String> = Vec::new();
        while let Ok(line) = line_rx.recv() {
            if let Some(progress) = parse_mongodump_line(&line) {
                on_progress(progress);
            } else if line.contains("error") || line.contains("Error") || line.contains("failed") {
                // Collect lines that look like errors
                error_lines.push(line);
            }
        }

        let status = child.wait()?;
        if !status.success() {
            let error_msg = if error_lines.is_empty() {
                "mongodump failed".to_string()
            } else {
                format!("mongodump failed: {}", error_lines.join("\n"))
            };
            return Err(Error::Parse(error_msg));
        }

        Ok(())
    }

    /// Import a database from BSON format with progress tracking.
    /// The callback receives (collection_name, bytes_processed, bytes_total, is_complete).
    pub fn import_database_bson_with_progress<F>(
        &self,
        connection_string: &str,
        database: &str,
        path: &Path,
        drop_before: bool,
        on_progress: F,
    ) -> Result<()>
    where
        F: Fn(BsonToolProgress) + Send + 'static,
    {
        use std::process::Stdio;

        let mongorestore = mongorestore_path().ok_or_else(|| {
            Error::ToolNotFound(
                "mongorestore not found. Run 'just download-tools' or install MongoDB Database Tools."
                    .into(),
            )
        })?;

        let mut cmd = Command::new(&mongorestore);
        cmd.arg("--uri")
            .arg(connection_string)
            .arg("--db")
            .arg(database)
            .arg("-v") // Enable verbose output for progress
            .stderr(Stdio::piped());

        if drop_before {
            cmd.arg("--drop");
        }

        // Detect if path is archive or folder
        if path.extension().map(|e| e == "archive").unwrap_or(false) {
            // --archive requires = format: --archive=/path/to/file
            cmd.arg(format!("--archive={}", path.display()));
        } else {
            let db_path = path.join(database);
            if db_path.exists() {
                cmd.arg("--dir").arg(&db_path);
            } else {
                cmd.arg("--dir").arg(path);
            }
        }

        let mut child = cmd.spawn()?;
        let stderr = child.stderr.take().expect("stderr was piped");

        // Read stderr in a separate thread to avoid blocking
        let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(std::result::Result::ok) {
                if line_tx.send(line).is_err() {
                    break;
                }
            }
        });

        // Process lines as they come in, collecting non-progress lines for error reporting
        let mut error_lines: Vec<String> = Vec::new();
        while let Ok(line) = line_rx.recv() {
            if let Some(progress) = parse_mongorestore_line(&line) {
                on_progress(progress);
            } else if line.contains("error") || line.contains("Error") || line.contains("failed") {
                // Collect lines that look like errors
                error_lines.push(line);
            }
        }

        let status = child.wait()?;
        if !status.success() {
            let error_msg = if error_lines.is_empty() {
                "mongorestore failed".to_string()
            } else {
                format!("mongorestore failed: {}", error_lines.join("\n"))
            };
            return Err(Error::Parse(error_msg));
        }

        Ok(())
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
}
