// Validation helpers

/// Validate a MongoDB connection URI
pub fn validate_mongodb_uri(uri: &str) -> Result<(), String> {
    let uri = uri.trim();

    if uri.is_empty() {
        return Err("URI is required".into());
    }

    if !uri.starts_with("mongodb://") && !uri.starts_with("mongodb+srv://") {
        return Err("URI must start with mongodb:// or mongodb+srv://".into());
    }

    // Basic format validation - just check it has a host
    let after_scheme =
        uri.strip_prefix("mongodb+srv://").or_else(|| uri.strip_prefix("mongodb://")).unwrap_or("");

    if after_scheme.is_empty() || after_scheme.starts_with('/') {
        return Err("URI must include a host".into());
    }

    Ok(())
}

/// Extract the host from a MongoDB URI for auto-filling connection name
/// mongodb://localhost:27017 → "localhost"
/// mongodb+srv://cluster0.abc.mongodb.net → "cluster0.abc.mongodb.net"
/// mongodb://user:pass@host:27017/db → "host"
pub fn extract_host_from_uri(uri: &str) -> Option<String> {
    let uri = uri.trim();

    // Strip the scheme
    let after_scheme =
        uri.strip_prefix("mongodb+srv://").or_else(|| uri.strip_prefix("mongodb://"))?;

    // Strip credentials if present (user:pass@)
    let after_credentials = if let Some(at_pos) = after_scheme.find('@') {
        &after_scheme[at_pos + 1..]
    } else {
        after_scheme
    };

    // Get just the host (before port, path, or query)
    let host = after_credentials.split([':', '/', '?']).next()?;

    if host.is_empty() { None } else { Some(host.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_uris() {
        assert!(validate_mongodb_uri("mongodb://localhost").is_ok());
        assert!(validate_mongodb_uri("mongodb://localhost:27017").is_ok());
        assert!(validate_mongodb_uri("mongodb://user:pass@localhost:27017").is_ok());
        assert!(validate_mongodb_uri("mongodb+srv://cluster.mongodb.net").is_ok());
    }

    #[test]
    fn test_invalid_uris() {
        assert!(validate_mongodb_uri("").is_err());
        assert!(validate_mongodb_uri("localhost:27017").is_err());
        assert!(validate_mongodb_uri("http://localhost").is_err());
        assert!(validate_mongodb_uri("mongodb://").is_err());
    }

    #[test]
    fn test_extract_host() {
        assert_eq!(extract_host_from_uri("mongodb://localhost"), Some("localhost".into()));
        assert_eq!(extract_host_from_uri("mongodb://localhost:27017"), Some("localhost".into()));
        assert_eq!(
            extract_host_from_uri("mongodb://user:pass@myhost:27017/db"),
            Some("myhost".into())
        );
        assert_eq!(
            extract_host_from_uri("mongodb+srv://cluster0.abc.mongodb.net"),
            Some("cluster0.abc.mongodb.net".into())
        );
        assert_eq!(
            extract_host_from_uri(
                "mongodb+srv://user:pass@cluster0.abc.mongodb.net/db?retryWrites=true"
            ),
            Some("cluster0.abc.mongodb.net".into())
        );
        assert_eq!(extract_host_from_uri(""), None);
        assert_eq!(extract_host_from_uri("invalid"), None);
    }
}
