// Validation helpers

pub const REDACTED_PASSWORD: &str = "*****";

/// Redact the password in a MongoDB URI.
/// e.g. "mongodb://user:secret@host" → "mongodb://user:*****@host"
pub fn redact_uri_password(uri: &str) -> String {
    let uri = uri.trim();
    let Some((scheme, rest)) = uri.split_once("://") else {
        return uri.to_string();
    };
    let Some((userinfo, after_at)) = rest.rsplit_once('@') else {
        return uri.to_string();
    };
    let Some((user, _password)) = userinfo.split_once(':') else {
        return uri.to_string();
    };
    format!("{scheme}://{user}:{REDACTED_PASSWORD}@{after_at}")
}

/// Replace the redacted password in a URI with the real password.
pub fn inject_uri_password(uri: &str, password: Option<&str>) -> String {
    let Some(password) = password else {
        return uri.to_string();
    };
    let uri = uri.trim();
    let Some((scheme, rest)) = uri.split_once("://") else {
        return uri.to_string();
    };
    let Some((userinfo, after_at)) = rest.rsplit_once('@') else {
        return uri.to_string();
    };
    let Some((user, _old_password)) = userinfo.split_once(':') else {
        return uri.to_string();
    };
    format!("{scheme}://{user}:{password}@{after_at}")
}

/// Extract the password from a MongoDB URI, if present.
pub fn extract_uri_password(uri: &str) -> Option<String> {
    let uri = uri.trim();
    let (_, rest) = uri.split_once("://")?;
    let (userinfo, _) = rest.rsplit_once('@')?;
    let (_, password) = userinfo.split_once(':')?;
    if password.is_empty() { None } else { Some(password.to_string()) }
}

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

    #[test]
    fn test_redact_uri_password() {
        assert_eq!(
            redact_uri_password("mongodb://user:secret@localhost:27017"),
            "mongodb://user:*****@localhost:27017"
        );
        assert_eq!(
            redact_uri_password(
                "mongodb+srv://admin:p%40ss@cluster0.abc.mongodb.net/db?retryWrites=true"
            ),
            "mongodb+srv://admin:*****@cluster0.abc.mongodb.net/db?retryWrites=true"
        );
        // No credentials
        assert_eq!(redact_uri_password("mongodb://localhost:27017"), "mongodb://localhost:27017");
        // Username only, no password
        assert_eq!(
            redact_uri_password("mongodb://user@localhost:27017"),
            "mongodb://user@localhost:27017"
        );
        // Already redacted
        assert_eq!(
            redact_uri_password("mongodb://user:*****@localhost:27017"),
            "mongodb://user:*****@localhost:27017"
        );
    }

    #[test]
    fn test_inject_uri_password() {
        assert_eq!(
            inject_uri_password("mongodb://user:*****@localhost:27017", Some("secret")),
            "mongodb://user:secret@localhost:27017"
        );
        assert_eq!(
            inject_uri_password(
                "mongodb+srv://admin:*****@cluster0.abc.mongodb.net/db?retryWrites=true",
                Some("p%40ss")
            ),
            "mongodb+srv://admin:p%40ss@cluster0.abc.mongodb.net/db?retryWrites=true"
        );
        // None password returns unchanged
        assert_eq!(
            inject_uri_password("mongodb://user:*****@localhost:27017", None),
            "mongodb://user:*****@localhost:27017"
        );
        // No credentials in URI
        assert_eq!(
            inject_uri_password("mongodb://localhost:27017", Some("secret")),
            "mongodb://localhost:27017"
        );
    }

    #[test]
    fn test_extract_uri_password() {
        assert_eq!(
            extract_uri_password("mongodb://user:secret@localhost:27017"),
            Some("secret".into())
        );
        assert_eq!(
            extract_uri_password("mongodb://user:*****@localhost:27017"),
            Some("*****".into())
        );
        assert_eq!(extract_uri_password("mongodb://localhost:27017"), None);
        assert_eq!(extract_uri_password("mongodb://user@localhost:27017"), None);
        assert_eq!(extract_uri_password("invalid"), None);
    }
}
