//! Integration tests for URI validation helpers (`openmango::helpers::validate`).

use openmango::helpers::validate::{
    REDACTED_PASSWORD, extract_host_from_uri, extract_uri_password, inject_uri_password,
    redact_uri_password, validate_mongodb_uri,
};

// =============================================================================
// validate_mongodb_uri
// =============================================================================

#[test]
fn test_validate_mongodb_uri_valid() {
    // Standard URI
    assert!(validate_mongodb_uri("mongodb://localhost").is_ok());
    // With port
    assert!(validate_mongodb_uri("mongodb://localhost:27017").is_ok());
    // SRV
    assert!(validate_mongodb_uri("mongodb+srv://cluster0.abc.mongodb.net").is_ok());
    // With credentials
    assert!(validate_mongodb_uri("mongodb://admin:pass@localhost:27017").is_ok());
    // With path and query params
    assert!(
        validate_mongodb_uri(
            "mongodb+srv://user:pass@cluster0.abc.mongodb.net/mydb?retryWrites=true&w=majority"
        )
        .is_ok()
    );
    // Leading/trailing whitespace is trimmed
    assert!(validate_mongodb_uri("  mongodb://localhost  ").is_ok());
    // Replica set with multiple hosts
    assert!(validate_mongodb_uri("mongodb://host1:27017,host2:27017,host3:27017").is_ok());
}

#[test]
fn test_validate_mongodb_uri_invalid() {
    // Empty
    assert!(validate_mongodb_uri("").is_err());
    // Whitespace-only
    assert!(validate_mongodb_uri("   ").is_err());
    // Wrong scheme
    assert!(validate_mongodb_uri("http://localhost").is_err());
    assert!(validate_mongodb_uri("postgres://localhost").is_err());
    // No host (scheme only)
    assert!(validate_mongodb_uri("mongodb://").is_err());
    assert!(validate_mongodb_uri("mongodb+srv://").is_err());
    // Scheme followed by slash but no host
    assert!(validate_mongodb_uri("mongodb:///dbname").is_err());
    // Plain hostname without scheme
    assert!(validate_mongodb_uri("localhost:27017").is_err());
}

// =============================================================================
// redact_uri_password
// =============================================================================

#[test]
fn test_redact_uri_password() {
    // Basic
    assert_eq!(
        redact_uri_password("mongodb://user:secret@localhost:27017"),
        format!("mongodb://user:{REDACTED_PASSWORD}@localhost:27017")
    );
    // SRV with encoded characters in password
    assert_eq!(
        redact_uri_password(
            "mongodb+srv://admin:p%40ss%21@cluster0.abc.mongodb.net/db?retryWrites=true"
        ),
        format!(
            "mongodb+srv://admin:{REDACTED_PASSWORD}@cluster0.abc.mongodb.net/db?retryWrites=true"
        )
    );
    // No credentials — returned unchanged
    assert_eq!(redact_uri_password("mongodb://localhost:27017"), "mongodb://localhost:27017");
    // Username only (no colon) — returned unchanged
    assert_eq!(
        redact_uri_password("mongodb://user@localhost:27017"),
        "mongodb://user@localhost:27017"
    );
    // Already redacted — idempotent
    assert_eq!(
        redact_uri_password(&format!("mongodb://user:{REDACTED_PASSWORD}@localhost:27017")),
        format!("mongodb://user:{REDACTED_PASSWORD}@localhost:27017")
    );
    // Non-mongodb scheme — returned unchanged
    assert_eq!(redact_uri_password("invalid-string"), "invalid-string");
}

// =============================================================================
// inject_uri_password
// =============================================================================

#[test]
fn test_inject_uri_password() {
    // Basic inject
    assert_eq!(
        inject_uri_password(
            &format!("mongodb://user:{REDACTED_PASSWORD}@localhost:27017"),
            Some("secret")
        ),
        "mongodb://user:secret@localhost:27017"
    );
    // SRV inject with encoded password
    assert_eq!(
        inject_uri_password(
            &format!(
                "mongodb+srv://admin:{REDACTED_PASSWORD}@cluster0.abc.mongodb.net/db?retryWrites=true"
            ),
            Some("p%40ss")
        ),
        "mongodb+srv://admin:p%40ss@cluster0.abc.mongodb.net/db?retryWrites=true"
    );
    // None password — returned unchanged
    assert_eq!(
        inject_uri_password(&format!("mongodb://user:{REDACTED_PASSWORD}@localhost:27017"), None),
        format!("mongodb://user:{REDACTED_PASSWORD}@localhost:27017")
    );
    // No credentials in URI — returned unchanged
    assert_eq!(
        inject_uri_password("mongodb://localhost:27017", Some("secret")),
        "mongodb://localhost:27017"
    );
}

// =============================================================================
// extract_uri_password
// =============================================================================

#[test]
fn test_extract_uri_password() {
    // Basic
    assert_eq!(
        extract_uri_password("mongodb://user:secret@localhost:27017"),
        Some("secret".to_string())
    );
    // No password (no credentials) → None
    assert_eq!(extract_uri_password("mongodb://localhost:27017"), None);
    // No credentials at all
    assert_eq!(extract_uri_password("invalid"), None);
    // Empty password → None (empty string returns None)
    assert_eq!(extract_uri_password("mongodb://user:@localhost:27017"), None);
    // Encoded password
    assert_eq!(
        extract_uri_password("mongodb://user:p%40ss@localhost:27017"),
        Some("p%40ss".to_string())
    );
}

// =============================================================================
// extract_host_from_uri
// =============================================================================

#[test]
fn test_extract_host_from_uri() {
    // Localhost
    assert_eq!(extract_host_from_uri("mongodb://localhost"), Some("localhost".to_string()));
    // With port
    assert_eq!(extract_host_from_uri("mongodb://localhost:27017"), Some("localhost".to_string()));
    // With credentials
    assert_eq!(
        extract_host_from_uri("mongodb://user:pass@myhost:27017/db"),
        Some("myhost".to_string())
    );
    // SRV
    assert_eq!(
        extract_host_from_uri("mongodb+srv://cluster0.abc.mongodb.net"),
        Some("cluster0.abc.mongodb.net".to_string())
    );
    // SRV with credentials, path, and query
    assert_eq!(
        extract_host_from_uri(
            "mongodb+srv://user:pass@cluster0.abc.mongodb.net/db?retryWrites=true"
        ),
        Some("cluster0.abc.mongodb.net".to_string())
    );
    // Empty string
    assert_eq!(extract_host_from_uri(""), None);
    // Invalid (no scheme)
    assert_eq!(extract_host_from_uri("invalid"), None);
    // Whitespace-only
    assert_eq!(extract_host_from_uri("   "), None);
}

// =============================================================================
// Password roundtrip: extract → redact → inject
// =============================================================================

#[test]
fn test_password_roundtrip() {
    let original = "mongodb://admin:s3cret%21@db.example.com:27017/mydb?authSource=admin";

    // Extract the real password
    let password = extract_uri_password(original).expect("should extract password");
    assert_eq!(password, "s3cret%21");

    // Redact
    let redacted = redact_uri_password(original);
    assert!(redacted.contains(REDACTED_PASSWORD));
    assert!(!redacted.contains("s3cret%21"));

    // Inject back
    let restored = inject_uri_password(&redacted, Some(&password));
    assert_eq!(restored, original);
}
