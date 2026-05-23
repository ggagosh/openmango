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

pub(crate) fn ranked_match_score(query: &str, text: &str) -> Option<usize> {
    let query = normalize_search_text(query);
    let text = normalize_search_text(text);
    if query.is_empty() {
        return None;
    }
    if text == query {
        return Some(0);
    }

    let query_tokens = tokenize(&query);
    let text_tokens = tokenize(&text);
    let multi_token_score = ordered_token_match_score(&query_tokens, &text_tokens);

    if text.starts_with(&query) {
        return Some(1 + text.len().saturating_sub(query.len()));
    }
    let token_score = best_token_match_score(&query, &text_tokens).map(|score| 15 + score);
    if let Some(pos) = text.find(&query) {
        return Some(
            [
                multi_token_score,
                token_score,
                Some(if is_word_boundary(&text, pos) { 30 } else { 45 } + pos),
            ]
            .into_iter()
            .flatten()
            .min()
            .unwrap(),
        );
    }

    [
        multi_token_score,
        token_score,
        acronym_match_score(&query, &text_tokens).map(|score| 55 + score),
        typo_match_score(&query, &text_tokens).map(|score| 75 + score),
        subsequence_match_score(&query, &text).map(|score| 120 + score),
    ]
    .into_iter()
    .flatten()
    .min()
}

pub(crate) fn fuzzy_match_score(query: &str, text: &str) -> Option<usize> {
    ranked_match_score(query, text)
}

fn normalize_search_text(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

#[derive(Clone, Copy)]
struct SearchToken<'a> {
    text: &'a str,
    start: usize,
}

fn tokenize(text: &str) -> Vec<SearchToken<'_>> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (idx, ch) in text.char_indices() {
        if ch.is_alphanumeric() {
            start.get_or_insert(idx);
        } else if let Some(token_start) = start.take() {
            tokens.push(SearchToken { text: &text[token_start..idx], start: token_start });
        }
    }
    if let Some(token_start) = start {
        tokens.push(SearchToken { text: &text[token_start..], start: token_start });
    }
    tokens
}

fn ordered_token_match_score(
    query_tokens: &[SearchToken<'_>],
    text_tokens: &[SearchToken<'_>],
) -> Option<usize> {
    if query_tokens.len() < 2 || text_tokens.is_empty() {
        return None;
    }

    let mut score = 8usize;
    let mut start_at = 0usize;
    for query_token in query_tokens {
        let mut best = None;
        for (offset, text_token) in text_tokens.iter().enumerate().skip(start_at) {
            let Some(token_score) = single_token_match_score(query_token.text, text_token.text)
            else {
                continue;
            };
            let candidate =
                token_score + text_token.start / 2 + offset.saturating_sub(start_at) * 8;
            if best.is_none_or(|(_, current_score)| candidate < current_score) {
                best = Some((offset, candidate));
            }
        }
        let (offset, token_score) = best?;
        score += token_score;
        start_at = offset + 1;
    }
    Some(score)
}

fn best_token_match_score(query: &str, text_tokens: &[SearchToken<'_>]) -> Option<usize> {
    text_tokens
        .iter()
        .filter_map(|token| {
            single_token_match_score(query, token.text)
                .map(|score| score + if token.start == 0 { 0 } else { token.start / 2 + 4 })
        })
        .min()
}

fn single_token_match_score(query: &str, token: &str) -> Option<usize> {
    if token == query {
        return Some(0);
    }
    if token.starts_with(query) {
        return Some(5 + token.len().saturating_sub(query.len()));
    }
    if let Some(pos) = token.find(query) {
        return Some(24 + pos);
    }
    if let Some(score) = typo_token_score(query, token) {
        return Some(36 + score);
    }
    subsequence_match_score(query, token).map(|score| 60 + score)
}

fn acronym_match_score(query: &str, text_tokens: &[SearchToken<'_>]) -> Option<usize> {
    if query.chars().count() < 2 || text_tokens.len() < 2 {
        return None;
    }

    let mut score = 0usize;
    let mut token_index = 0usize;
    for query_char in query.chars() {
        let mut found = None;
        for (idx, token) in text_tokens.iter().enumerate().skip(token_index) {
            if token.text.starts_with(query_char) {
                found = Some((idx, token.start));
                break;
            }
        }
        let (idx, start) = found?;
        score += idx.saturating_sub(token_index) * 10 + start / 2;
        token_index = idx + 1;
    }
    Some(score)
}

fn typo_match_score(query: &str, text_tokens: &[SearchToken<'_>]) -> Option<usize> {
    text_tokens.iter().filter_map(|token| typo_token_score(query, token.text)).min()
}

fn typo_token_score(query: &str, token: &str) -> Option<usize> {
    let query_len = query.chars().count();
    let token_len = token.chars().count();
    if query_len < 3 || token_len < 3 {
        return None;
    }

    let max_edits = allowed_typo_edits(query_len);
    let mut best = None;

    if query_len.abs_diff(token_len) <= max_edits
        && let Some(distance) = bounded_damerau_levenshtein(query, token, max_edits)
    {
        best = Some(distance * 18 + query_len.abs_diff(token_len));
    }

    if token_len >= query_len {
        let prefix = first_chars(token, query_len);
        if let Some(distance) = bounded_damerau_levenshtein(query, &prefix, max_edits) {
            let prefix_score = distance * 16 + token_len.saturating_sub(query_len).min(8);
            best = Some(best.map_or(prefix_score, |score: usize| score.min(prefix_score)));
        }
    }

    best
}

fn allowed_typo_edits(query_len: usize) -> usize {
    match query_len {
        0..=2 => 0,
        3..=5 => 1,
        6..=10 => 2,
        _ => 3,
    }
}

fn first_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

fn bounded_damerau_levenshtein(a: &str, b: &str, max_distance: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max_distance {
        return None;
    }

    let mut distances = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in distances.iter_mut().enumerate().take(a.len() + 1) {
        row[0] = i;
    }
    for (j, distance) in distances[0].iter_mut().enumerate() {
        *distance = j;
    }

    for i in 1..=a.len() {
        let mut row_min = usize::MAX;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut distance = (distances[i - 1][j] + 1)
                .min(distances[i][j - 1] + 1)
                .min(distances[i - 1][j - 1] + cost);

            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                distance = distance.min(distances[i - 2][j - 2] + 1);
            }

            distances[i][j] = distance;
            row_min = row_min.min(distance);
        }
        if row_min > max_distance {
            return None;
        }
    }

    let distance = distances[a.len()][b.len()];
    (distance <= max_distance).then_some(distance)
}

fn is_word_boundary(text: &str, pos: usize) -> bool {
    pos == 0 || text[..pos].chars().next_back().is_none_or(|ch| !ch.is_alphanumeric())
}

fn subsequence_match_score(query: &str, text: &str) -> Option<usize> {
    if query.is_empty() {
        return None;
    }
    let mut score = 0usize;
    let mut last_index = 0usize;
    let mut first_index = None;
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
        first_index.get_or_insert(pos);
        let gap = pos.saturating_sub(last_index);
        score += if gap == 0 { 0 } else { 4 + gap * 3 };
        last_index = pos + 1;
    }
    Some(score + first_index.unwrap_or(0) * 2)
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

    #[test]
    fn search_tolerates_missing_extra_and_swapped_letters() {
        let connection_id = Uuid::new_v4();
        let candidates = || {
            [
                SidebarSearchCandidate::connection(connection_id, "Production"),
                SidebarSearchCandidate::database(connection_id, "Production", "analytics"),
                SidebarSearchCandidate::collection(
                    connection_id,
                    "Production",
                    "analytics",
                    "user_profiles",
                ),
            ]
        };

        assert_eq!(search_results("prodction", candidates())[0].title, "Production");
        assert_eq!(search_results("anlytics", candidates())[0].title, "analytics");
        assert_eq!(search_results("porfiles", candidates())[0].title, "user_profiles");
    }

    #[test]
    fn search_understands_acronyms_and_word_tokens() {
        let connection_id = Uuid::new_v4();
        let results = search_results(
            "up",
            [
                SidebarSearchCandidate::collection(connection_id, "Local", "app", "user_profiles"),
                SidebarSearchCandidate::collection(connection_id, "Local", "app", "accounts"),
            ],
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "user_profiles");
    }
}
