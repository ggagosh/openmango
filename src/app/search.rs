use uuid::Uuid;

use crate::models::TreeNodeId;

#[derive(Clone, Debug)]
pub(crate) struct SidebarEntry {
    pub(crate) id: TreeNodeId,
    pub(crate) label: String,
    pub(crate) search_label: String,
    pub(crate) depth: usize,
    pub(crate) is_folder: bool,
    pub(crate) is_expanded: bool,
}

impl SidebarEntry {
    pub(crate) fn new(
        id: TreeNodeId,
        label: impl Into<String>,
        depth: usize,
        is_folder: bool,
        is_expanded: bool,
    ) -> Self {
        let label = label.into();
        let search_label = label.to_lowercase();
        Self { id, label, search_label, depth, is_folder, is_expanded }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SidebarSearchKind {
    Connection,
    Database,
    Collection,
}

impl SidebarSearchKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Connection => "Connection",
            Self::Database => "Database",
            Self::Collection => "Collection",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SidebarSearchCandidate {
    pub(crate) node_id: TreeNodeId,
    pub(crate) connection_id: Uuid,
    pub(crate) kind: SidebarSearchKind,
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) database: Option<String>,
    pub(crate) collection: Option<String>,
}

impl SidebarSearchCandidate {
    pub(crate) fn connection(connection_id: Uuid, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            node_id: TreeNodeId::connection(connection_id),
            connection_id,
            kind: SidebarSearchKind::Connection,
            title: name,
            subtitle: "Connection".to_string(),
            database: None,
            collection: None,
        }
    }

    pub(crate) fn database(
        connection_id: Uuid,
        connection_name: impl Into<String>,
        database: impl Into<String>,
    ) -> Self {
        let connection_name = connection_name.into();
        let database = database.into();
        Self {
            node_id: TreeNodeId::database(connection_id, database.clone()),
            connection_id,
            kind: SidebarSearchKind::Database,
            title: database.clone(),
            subtitle: connection_name,
            database: Some(database),
            collection: None,
        }
    }

    pub(crate) fn collection(
        connection_id: Uuid,
        connection_name: impl Into<String>,
        database: impl Into<String>,
        collection: impl Into<String>,
    ) -> Self {
        let connection_name = connection_name.into();
        let database = database.into();
        let collection = collection.into();
        Self {
            node_id: TreeNodeId::collection(connection_id, database.clone(), collection.clone()),
            connection_id,
            kind: SidebarSearchKind::Collection,
            title: collection.clone(),
            subtitle: format!("{connection_name} / {database}"),
            database: Some(database),
            collection: Some(collection),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SidebarSearchResult {
    pub(crate) node_id: TreeNodeId,
    pub(crate) connection_id: Uuid,
    pub(crate) kind: SidebarSearchKind,
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) database: Option<String>,
    pub(crate) collection: Option<String>,
    pub(crate) score: usize,
}

pub(crate) fn search_results(
    query: &str,
    candidates: impl IntoIterator<Item = SidebarSearchCandidate>,
) -> Vec<SidebarSearchResult> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::new();
    for candidate in candidates {
        let title = candidate.title.to_lowercase();
        let path = candidate.subtitle.to_lowercase();
        let Some(score) = best_match_score(&query, &title, &path) else {
            continue;
        };
        results.push(SidebarSearchResult {
            node_id: candidate.node_id,
            connection_id: candidate.connection_id,
            kind: candidate.kind,
            title: candidate.title,
            subtitle: candidate.subtitle,
            database: candidate.database,
            collection: candidate.collection,
            score,
        });
    }

    results.sort_by(|a, b| {
        a.score
            .cmp(&b.score)
            .then_with(|| a.title.len().cmp(&b.title.len()))
            .then_with(|| a.title.cmp(&b.title))
    });
    results
}

fn best_match_score(query: &str, title: &str, path: &str) -> Option<usize> {
    let title_score = ranked_match_score(query, title);
    let path_score = ranked_match_score(query, path).map(|score| score + 80);
    match (title_score, path_score) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(score), None) | (None, Some(score)) => Some(score),
        (None, None) => None,
    }
}

fn ranked_match_score(query: &str, text: &str) -> Option<usize> {
    if query.is_empty() {
        return None;
    }
    if text == query {
        return Some(0);
    }
    if text.starts_with(query) {
        return Some(1 + text.len().saturating_sub(query.len()));
    }
    if let Some(pos) = text.find(query) {
        return Some(25 + pos);
    }
    fuzzy_match_score(query, text).map(|score| 100 + score)
}

pub(crate) fn fuzzy_match_score(query: &str, text: &str) -> Option<usize> {
    if query.is_empty() {
        return None;
    }
    let mut score = 0usize;
    let mut last_index = 0usize;
    let chars: Vec<char> = text.chars().collect();
    for ch in query.chars() {
        let mut found = None;
        for (offset, tc) in chars.iter().enumerate().skip(last_index) {
            if *tc == ch {
                found = Some(offset);
                break;
            }
        }
        let pos = found?;
        score += pos.saturating_sub(last_index);
        last_index = pos + 1;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ranks_prefix_before_fuzzy_match() {
        let connection_id = Uuid::new_v4();
        let results = search_results(
            "app",
            [
                SidebarSearchCandidate::database(connection_id, "Local", "snapshots"),
                SidebarSearchCandidate::database(connection_id, "Local", "app_data"),
            ],
        );

        assert_eq!(results[0].title, "app_data");
    }

    #[test]
    fn search_returns_connection_database_and_collection_candidates() {
        let connection_id = Uuid::new_v4();
        let results = search_results(
            "prod",
            [
                SidebarSearchCandidate::connection(connection_id, "Production"),
                SidebarSearchCandidate::database(connection_id, "Production", "analytics"),
                SidebarSearchCandidate::collection(
                    connection_id,
                    "Production",
                    "analytics",
                    "events",
                ),
            ],
        );

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].kind, SidebarSearchKind::Connection);
    }
}
