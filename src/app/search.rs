use std::collections::HashMap;

use uuid::Uuid;

use crate::components::TreeNodeId;

#[derive(Clone, Debug)]
pub(crate) struct SidebarEntry {
    pub(crate) id: TreeNodeId,
    pub(crate) label: String,
    pub(crate) depth: usize,
    pub(crate) is_folder: bool,
    pub(crate) is_expanded: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct SidebarSearchResult {
    pub(crate) index: usize,
    pub(crate) node_id: TreeNodeId,
    pub(crate) connection_id: Uuid,
    pub(crate) connection_name: String,
    pub(crate) database: String,
    pub(crate) score: usize,
}

pub(crate) fn search_results(
    query: &str,
    entries: &[SidebarEntry],
    connection_names: &HashMap<Uuid, String>,
) -> Vec<SidebarSearchResult> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let TreeNodeId::Database { connection, database } = &entry.id else {
            continue;
        };
        let label = entry.label.to_lowercase();
        let Some(score) = fuzzy_match_score(&query, &label) else {
            continue;
        };
        let connection_name =
            connection_names.get(connection).cloned().unwrap_or_else(|| "Connection".to_string());
        results.push(SidebarSearchResult {
            index,
            node_id: entry.id.clone(),
            connection_id: *connection,
            connection_name,
            database: database.clone(),
            score,
        });
    }

    results.sort_by(|a, b| {
        a.score
            .cmp(&b.score)
            .then_with(|| a.database.len().cmp(&b.database.len()))
            .then_with(|| a.database.cmp(&b.database))
    });
    results
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
