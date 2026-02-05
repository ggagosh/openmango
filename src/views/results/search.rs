use std::collections::HashSet;

use crate::state::SessionDocument;
use crate::views::documents::tree::lazy_row::compute_row_meta;
use crate::views::documents::tree::lazy_tree::VisibleRow;

pub fn filter_visible_rows(
    documents: &[SessionDocument],
    rows: Vec<VisibleRow>,
    query: &str,
) -> Vec<VisibleRow> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return rows;
    }

    let mut keep_ids = HashSet::new();
    for row in rows.iter() {
        let meta = compute_row_meta(row, documents);
        let haystack =
            format!("{} {} {}", meta.key_label, meta.value_label, meta.type_label).to_lowercase();
        if !haystack.contains(&needle) {
            continue;
        }

        let doc_key = &documents[row.doc_index].key;
        keep_ids.insert(crate::bson::doc_root_id(doc_key));
        for depth in 1..=row.path.len() {
            keep_ids.insert(crate::bson::path_to_id(doc_key, &row.path[..depth]));
        }
        keep_ids.insert(row.node_id.clone());
    }

    rows.into_iter().filter(|row| keep_ids.contains(&row.node_id)).collect()
}
