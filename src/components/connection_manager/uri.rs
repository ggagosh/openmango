use gpui::{Context, Entity};
use gpui_component::input::InputState;

use super::ConnectionManager;

#[derive(Clone, Debug)]
pub(super) struct UriParts {
    scheme: String,
    user: Option<String>,
    password: Option<String>,
    hosts: String,
    database: Option<String>,
    query: Vec<(String, String)>,
}

impl UriParts {
    pub(super) fn get_query(&self, key: &str) -> String {
        self.query
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }

    pub(super) fn set_query(&mut self, key: &str, value: Option<String>) {
        self.query.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
        if let Some(value) = value {
            self.query.push((key.to_string(), value));
        }
    }

    pub(super) fn set_userinfo(&mut self, user: Option<String>, password: Option<String>) {
        self.user = user;
        self.password = password;
    }

    pub(super) fn userinfo(&self) -> (Option<String>, Option<String>) {
        (self.user.clone(), self.password.clone())
    }

    pub(super) fn to_uri(&self) -> String {
        let mut output = format!("{}://", self.scheme);
        if let Some(user) = &self.user {
            output.push_str(user);
            if let Some(password) = &self.password {
                output.push(':');
                output.push_str(password);
            }
            output.push('@');
        }
        output.push_str(&self.hosts);
        if let Some(database) = &self.database
            && !database.is_empty()
        {
            output.push('/');
            output.push_str(database);
        }
        if !self.query.is_empty() {
            output.push('?');
            for (idx, (key, value)) in self.query.iter().enumerate() {
                output.push_str(key);
                output.push('=');
                output.push_str(value);
                if idx + 1 < self.query.len() {
                    output.push('&');
                }
            }
        }
        output
    }
}

pub(super) fn parse_uri(input: &str) -> Result<UriParts, String> {
    let raw = input.trim();
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| "URI must include a scheme (mongodb:// or mongodb+srv://)".to_string())?;
    if rest.is_empty() {
        return Err("URI is missing host".to_string());
    }
    let (base, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host_part, database) = base.split_once('/').unwrap_or((base, ""));
    if host_part.is_empty() {
        return Err("URI is missing host".to_string());
    }

    let (userinfo, hosts) =
        host_part.rsplit_once('@').map(|(u, h)| (Some(u), h)).unwrap_or((None, host_part));

    if hosts.trim().is_empty() {
        return Err("URI is missing host".to_string());
    }

    let (user, password) = if let Some(userinfo) = userinfo {
        if let Some((user, pass)) = userinfo.split_once(':') {
            (Some(user.to_string()), Some(pass.to_string()))
        } else {
            (Some(userinfo.to_string()), None)
        }
    } else {
        (None, None)
    };

    let database = if database.trim().is_empty() { None } else { Some(database.to_string()) };

    let mut query_pairs = Vec::new();
    if !query.trim().is_empty() {
        for pair in query.split('&') {
            if pair.trim().is_empty() {
                continue;
            }
            if let Some((key, value)) = pair.split_once('=') {
                query_pairs.push((key.to_string(), value.to_string()));
            } else {
                query_pairs.push((pair.to_string(), String::new()));
            }
        }
    }

    Ok(UriParts {
        scheme: scheme.to_string(),
        user,
        password,
        hosts: hosts.to_string(),
        database,
        query: query_pairs,
    })
}

pub(super) fn parse_bool(value: String) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on")
}

pub(super) fn bool_to_query(value: bool) -> Option<String> {
    if value { Some("true".to_string()) } else { None }
}

pub(super) fn value_or_none(
    state: &Entity<InputState>,
    cx: &mut Context<ConnectionManager>,
) -> Option<String> {
    let value = state.read(cx).value().to_string();
    if value.trim().is_empty() { None } else { Some(value.trim().to_string()) }
}
