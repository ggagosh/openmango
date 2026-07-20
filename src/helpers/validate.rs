// Validation helpers

use serde::{Deserialize, Serialize};

pub const REDACTED_PASSWORD: &str = "*****";
const TLS_PASSWORD_QUERY: &str = "tlsCertificateKeyFilePassword";
const PROXY_PASSWORD_QUERY: &str = "proxyPassword";
const AUTH_MECHANISM_PROPERTIES_QUERY: &str = "authMechanismProperties";
const AWS_SESSION_TOKEN_PROPERTY: &str = "AWS_SESSION_TOKEN";

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UriSecrets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_certificate_key_file_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_session_token: Option<String>,
}

impl UriSecrets {
    pub fn is_empty(&self) -> bool {
        self.password.is_none()
            && self.tls_certificate_key_file_password.is_none()
            && self.proxy_password.is_none()
            && self.aws_session_token.is_none()
    }
}

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

fn remove_uri_password(uri: &str) -> String {
    let uri = uri.trim();
    let Some((scheme, rest)) = uri.split_once("://") else {
        return uri.to_string();
    };
    let Some((userinfo, after_at)) = rest.rsplit_once('@') else {
        return uri.to_string();
    };
    let user = userinfo.split_once(':').map(|(user, _)| user).unwrap_or(userinfo);
    format!("{scheme}://{user}@{after_at}")
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
    let user = userinfo.split_once(':').map(|(user, _)| user).unwrap_or(userinfo);
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

/// Extract credentials from the authority and sensitive URI query options.
pub fn extract_uri_secrets(uri: &str) -> UriSecrets {
    let mut secrets = UriSecrets { password: extract_uri_password(uri), ..UriSecrets::default() };
    let Some((_, query)) = uri.split_once('?') else {
        return secrets;
    };

    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key.eq_ignore_ascii_case(TLS_PASSWORD_QUERY) {
            if !value.is_empty() {
                secrets.tls_certificate_key_file_password = Some(value.to_string());
            }
        } else if key.eq_ignore_ascii_case(PROXY_PASSWORD_QUERY) {
            if !value.is_empty() {
                secrets.proxy_password = Some(value.to_string());
            }
        } else if key.eq_ignore_ascii_case(AUTH_MECHANISM_PROPERTIES_QUERY)
            && let Some(value) = extract_auth_mechanism_secret(value)
        {
            secrets.aws_session_token = Some(value);
        }
    }
    secrets
}

/// Remove every usable credential while preserving non-secret URI options.
pub fn strip_uri_secrets(uri: &str) -> String {
    let without_password = remove_uri_password(uri);
    let Some((base, query)) = without_password.split_once('?') else {
        return without_password;
    };
    let mut kept = Vec::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key.eq_ignore_ascii_case(TLS_PASSWORD_QUERY)
            || key.eq_ignore_ascii_case(PROXY_PASSWORD_QUERY)
        {
            continue;
        }
        if key.eq_ignore_ascii_case(AUTH_MECHANISM_PROPERTIES_QUERY) {
            if let Some(non_secret) = strip_auth_mechanism_secret(value) {
                kept.push(format!("{key}={non_secret}"));
            }
        } else {
            kept.push(pair.to_string());
        }
    }
    if kept.is_empty() { base.to_string() } else { format!("{base}?{}", kept.join("&")) }
}

/// Restore credentials into a URI that was produced by [`strip_uri_secrets`].
pub fn inject_uri_secrets(uri: &str, secrets: &UriSecrets) -> String {
    let mut resolved = inject_uri_password(uri, secrets.password.as_deref());
    let (base, query) = resolved.split_once('?').unwrap_or((&resolved, ""));
    let mut pairs: Vec<String> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter(|pair| {
            let key = pair.split_once('=').map(|(key, _)| key).unwrap_or(pair);
            !key.eq_ignore_ascii_case(TLS_PASSWORD_QUERY)
                && !key.eq_ignore_ascii_case(PROXY_PASSWORD_QUERY)
                && !key.eq_ignore_ascii_case(AUTH_MECHANISM_PROPERTIES_QUERY)
        })
        .map(ToString::to_string)
        .collect();

    let existing_auth = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.eq_ignore_ascii_case(AUTH_MECHANISM_PROPERTIES_QUERY))
        .and_then(|(_, value)| strip_auth_mechanism_secret(value));
    if let Some(password) = &secrets.tls_certificate_key_file_password {
        pairs.push(format!("{TLS_PASSWORD_QUERY}={password}"));
    }
    if let Some(password) = &secrets.proxy_password {
        pairs.push(format!("{PROXY_PASSWORD_QUERY}={password}"));
    }
    let auth_properties = match (existing_auth, secrets.aws_session_token.as_deref()) {
        (Some(properties), Some(token)) => Some(format!(
            "{},{}:{}",
            percent_decode(&properties),
            AWS_SESSION_TOKEN_PROPERTY,
            token
        )),
        (Some(properties), None) => Some(percent_decode(&properties)),
        (None, Some(token)) => Some(format!("{AWS_SESSION_TOKEN_PROPERTY}:{token}")),
        (None, None) => None,
    };
    if let Some(properties) = auth_properties {
        pairs.push(format!("{AUTH_MECHANISM_PROPERTIES_QUERY}={}", percent_encode(&properties)));
    }

    resolved =
        if pairs.is_empty() { base.to_string() } else { format!("{base}?{}", pairs.join("&")) };
    resolved
}

fn extract_auth_mechanism_secret(value: &str) -> Option<String> {
    percent_decode(value).split(',').find_map(|property| {
        let (key, value) = property.split_once(':')?;
        key.eq_ignore_ascii_case(AWS_SESSION_TOKEN_PROPERTY).then(|| value.to_string())
    })
}

fn strip_auth_mechanism_secret(value: &str) -> Option<String> {
    let decoded = percent_decode(value);
    let kept: Vec<&str> = decoded
        .split(',')
        .filter(|property| {
            property
                .split_once(':')
                .map(|(key, _)| !key.eq_ignore_ascii_case(AWS_SESSION_TOKEN_PROPERTY))
                .unwrap_or(true)
        })
        .filter(|property| !property.is_empty())
        .collect();
    (!kept.is_empty()).then(|| percent_encode(&kept.join(",")))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            decoded.push((high << 4) | low);
            index += 3;
            continue;
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn percent_encode(value: &str) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(&mut encoded, "%{byte:02X}");
        }
    }
    encoded
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
    fn uri_secrets_round_trip_without_persisting_credentials() {
        let uri = "mongodb://user:authority-secret@host/db?retryWrites=true&TLSCertificateKeyFilePassword=tls%20secret&proxyPassword=proxy%2Fsecret&authMechanismProperties=SERVICE_NAME%3Amongodb%2CAWS_SESSION_TOKEN%3Aaws%2Bsecret";

        let secrets = extract_uri_secrets(uri);
        assert_eq!(secrets.password.as_deref(), Some("authority-secret"));
        assert_eq!(secrets.tls_certificate_key_file_password.as_deref(), Some("tls%20secret"));
        assert_eq!(secrets.proxy_password.as_deref(), Some("proxy%2Fsecret"));
        assert_eq!(secrets.aws_session_token.as_deref(), Some("aws+secret"));

        let stripped = strip_uri_secrets(uri);
        for secret in ["authority-secret", "tls%20secret", "proxy%2Fsecret", "aws%2Bsecret"] {
            assert!(!stripped.contains(secret));
        }
        assert!(stripped.contains("retryWrites=true"));
        assert!(stripped.contains("SERVICE_NAME%3Amongodb"));

        let restored = inject_uri_secrets(&stripped, &secrets);
        let restored_secrets = extract_uri_secrets(&restored);
        assert!(restored_secrets == secrets);
        assert!(restored.contains("retryWrites=true"));
        assert!(restored.contains("SERVICE_NAME%3Amongodb"));
    }

    #[test]
    fn five_asterisks_is_treated_as_a_real_password_but_never_persisted() {
        let uri =
            "mongodb://user:*****@host/db?tlsCertificateKeyFilePassword=*****&proxyPassword=*****";
        let secrets = extract_uri_secrets(uri);
        assert_eq!(secrets.password.as_deref(), Some("*****"));
        assert_eq!(secrets.tls_certificate_key_file_password.as_deref(), Some("*****"));
        assert_eq!(secrets.proxy_password.as_deref(), Some("*****"));
        let stripped = strip_uri_secrets(uri);
        assert_eq!(stripped, "mongodb://user@host/db");
        assert_eq!(inject_uri_secrets(&stripped, &secrets), uri);
    }

    #[test]
    fn uri_secret_stripping_removes_empty_secret_only_auth_properties() {
        let uri = "mongodb://host/?authMechanismProperties=AWS_SESSION_TOKEN%3Atoken";
        assert_eq!(strip_uri_secrets(uri), "mongodb://host/");
    }

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
