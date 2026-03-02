//! Dynamic AI context builder — injects MongoDB state into the system prompt.

use std::collections::HashSet;
use std::fmt::Write;

use mongodb::bson::Bson;

use crate::bson::document_to_shell_string;
use crate::state::app_state::CollectionSubview;
use crate::state::commands::schema_to_summary;
use crate::state::{AppState, SessionKey};

const BUDGET: usize = 25 * 1024; // 25 KB

const BASE_PROMPT: &str = "\
You are an AI assistant for OpenMango, a MongoDB GUI.
Help with MongoDB queries, schema design, aggregation pipelines, and database operations.
When writing queries, use MongoDB shell syntax.
Format your responses using Markdown.";

// ---------------------------------------------------------------------------
// BudgetWriter
// ---------------------------------------------------------------------------

struct BudgetWriter {
    buf: String,
    budget: usize,
}

impl BudgetWriter {
    fn new(budget: usize) -> Self {
        Self { buf: String::with_capacity(budget.min(32768)), budget }
    }

    fn remaining(&self) -> usize {
        self.budget.saturating_sub(self.buf.len())
    }

    /// Append a section with a markdown header. Returns `false` if the section
    /// was skipped because the header alone would not fit.
    fn section(&mut self, header: &str, body: &str) -> bool {
        let needed = header.len() + body.len() + 3; // "\n\n" before header + "\n" before body
        if needed > self.remaining() {
            return false;
        }
        let _ = write!(self.buf, "\n\n{header}\n{body}");
        true
    }

    /// Write raw text (no header). Returns `false` if it would exceed budget.
    fn raw(&mut self, text: &str) -> bool {
        if text.len() > self.remaining() {
            return false;
        }
        self.buf.push_str(text);
        true
    }

    fn finish(self) -> String {
        self.buf
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build a dynamic system prompt from the current application state.
pub fn build_ai_context(state: &AppState) -> String {
    let mut w = BudgetWriter::new(BUDGET);

    // ── 1. Base prompt + identity ──────────────────────────────────────────
    w.raw(BASE_PROMPT);

    let conn_id = match state.selected_connection_id() {
        Some(id) => id,
        None => return w.finish(),
    };
    let active = match state.active_connection_by_id(conn_id) {
        Some(c) => c,
        None => return w.finish(),
    };

    // Tools section — when connected, the AI has MongoDB tools available.
    w.section(
        "## Tool Usage Guide",
        "You have 8 MongoDB tools. Choose the minimal set needed — prefer one powerful call \
         over many small ones.\n\n\
         ### Querying\n\
         - **aggregate**: Your most powerful tool. Use for any multi-step operation: filtering + \
         grouping, joins ($lookup), faceted results, stats computation. Prefer this when the \
         answer needs transformation beyond simple filtering. Pipeline stages are JSON objects. \
         Max 50 docs returned.\n\
         - **find_documents**: Simple queries with filter, sort, projection, limit. Use when you \
         just need to fetch documents matching a condition. Max 50 docs.\n\
         - **count_documents**: Count documents matching a filter. Use for \"how many?\" questions. \
         Fast (uses estimated count when no filter).\n\n\
         ### Introspection\n\
         - **collection_stats**: Get document count, data size, storage size, index count. Use for \
         \"how big/large is this collection?\" questions.\n\
         - **collection_schema**: Sample 100 docs and analyze field types, presence %, cardinality. \
         Use when the user asks about structure/fields.\n\
         - **list_indexes**: List all indexes with key definitions. Use for performance analysis.\n\
         - **explain_query**: Explain a find query's execution plan. Use to diagnose slow queries.\n\
         - **list_collections**: List all collections in the database.\n\n\
         ### Cross-Collection Access\n\
         All tools except list_collections accept an optional `collection` parameter. Pass it to \
         query or inspect any collection in the current database — not just the selected one. \
         For example, call `collection_schema` with `collection: \"orders\"` while viewing `users`.\n\n\
         ### Strategy\n\
         - When a question can be answered with a single aggregation pipeline, use `aggregate` \
         instead of calling multiple tools. For example: \"What's the average order value by \
         status?\" → one `aggregate` call with $group, not find + manual calculation.\n\
         - Use $lookup in aggregate to join across collections rather than querying each \
         separately.\n\
         - Use $facet to compute multiple aggregations in one pipeline.\n\
         - The context below already includes schema, indexes, and stats when available — check \
         it before calling introspection tools redundantly.\n\
         - If you need schema or structure for a collection not shown in the context below, \
         call `collection_schema` with the `collection` parameter to fetch it.\n\
         - When the user asks about data, always use tools — never guess or fabricate data.",
    );

    let conn_name = state.connection_name(conn_id).unwrap_or_default();
    let db_name = state.selected_database_name();
    let col_name = state.selected_collection_name();

    let mut identity = String::new();
    let _ = write!(identity, "Connection: {conn_name}");
    if let Some(db) = &db_name {
        let _ = write!(identity, "\nDatabase: {db}");
    }
    if let Some(col) = &col_name {
        let _ = write!(identity, "\nCollection: {col}");
    }
    w.section("## Current Context", &identity);

    let session_key = state.current_session_key();
    let data = session_key.as_ref().and_then(|k| state.session_data(k));
    let view = session_key.as_ref().and_then(|k| state.session_view(k));

    // ── 2. Current subview + active query ──────────────────────────────────
    if let (Some(data), Some(view)) = (data, view) {
        let subview_label = match view.subview {
            CollectionSubview::Documents => "Documents",
            CollectionSubview::Indexes => "Indexes",
            CollectionSubview::Stats => "Stats",
            CollectionSubview::Aggregation => "Aggregation",
            CollectionSubview::Schema => "Schema",
        };
        let mut query_buf = String::new();
        let _ = write!(query_buf, "Active subview: {subview_label}");
        if !data.filter_raw.is_empty() {
            let _ = write!(query_buf, "\nFilter: {}", data.filter_raw);
        }
        if !data.sort_raw.is_empty() {
            let _ = write!(query_buf, "\nSort: {}", data.sort_raw);
        }
        if !data.projection_raw.is_empty() {
            let _ = write!(query_buf, "\nProjection: {}", data.projection_raw);
        }
        let _ = write!(query_buf, "\nMatched: {} documents", data.total);
        w.section("## Active Query", &query_buf);
    }

    // ── 3. Collection stats ────────────────────────────────────────────────
    if let Some(stats) = data.and_then(|d| d.stats.as_ref()) {
        let mut buf = String::new();
        let _ = write!(buf, "Documents: {}", stats.document_count);
        let _ = write!(buf, "\nAvg document size: {} bytes", stats.avg_obj_size);
        let _ = write!(buf, "\nData size: {} bytes", stats.data_size);
        let _ = write!(buf, "\nStorage size: {} bytes", stats.storage_size);
        let _ = write!(buf, "\nIndexes: {}", stats.index_count);
        let _ = write!(buf, "\nTotal index size: {} bytes", stats.total_index_size);
        if stats.capped {
            buf.push_str("\nCapped: yes");
            if let Some(max) = stats.max_size {
                let _ = write!(buf, " (max size: {max} bytes)");
            }
        }
        w.section("## Collection Stats", &buf);
    }

    // ── 4. Schema summary ──────────────────────────────────────────────────
    if let Some(schema) = data.and_then(|d| d.schema.as_ref()) {
        let summary = schema_to_summary(schema);
        let cap = w.remaining().min(4000);
        let truncated = truncate_str(&summary, cap);
        let header =
            format!("## Schema (sampled {} docs, {} fields)", schema.sampled, schema.total_fields);
        w.section(&header, truncated);
    }

    // ── 5. Indexes ─────────────────────────────────────────────────────────
    if let Some(indexes) = data.and_then(|d| d.indexes.as_ref()) {
        let mut buf = String::new();
        for index in indexes {
            let name =
                index.options.as_ref().and_then(|o| o.name.as_deref()).unwrap_or("(unnamed)");
            let keys = format_index_keys(&index.keys);
            let mut flags = Vec::new();
            if let Some(opts) = &index.options {
                if opts.unique == Some(true) {
                    flags.push("unique");
                }
                if opts.sparse == Some(true) {
                    flags.push("sparse");
                }
                if opts.expire_after.is_some() {
                    flags.push("TTL");
                }
            }
            if flags.is_empty() {
                let _ = writeln!(buf, "- {name} ({keys})");
            } else {
                let _ = writeln!(buf, "- {name} ({keys}, {})", flags.join(", "));
            }
        }
        if !buf.is_empty() {
            w.section("## Indexes", buf.trim_end());
        }
    }

    // ── 6. Selected documents ──────────────────────────────────────────────
    if let (Some(data), Some(view)) = (data, view)
        && !view.selected_docs.is_empty()
    {
        let mut buf = String::new();
        let mut count = 0;
        for key in &view.selected_docs {
            if count >= 3 {
                break;
            }
            if let Some(&idx) = data.index_by_key.get(key)
                && let Some(item) = data.items.get(idx)
            {
                let shell = document_to_shell_string(&item.doc);
                let truncated = truncate_str(&shell, 1500);
                let _ = writeln!(buf, "{truncated}");
                count += 1;
            }
        }
        if !buf.is_empty() {
            let cap = w.remaining().min(4000);
            let body = truncate_str(&buf, cap);
            w.section("## Selected Documents", body.trim_end());
        }
    }

    let Some(data) = data else {
        return w.finish();
    };

    // ── 7. Diversity-sampled documents ─────────────────────────────────────
    if !data.items.is_empty() {
        let selected =
            select_diverse_docs(&data.items.iter().map(|d| &d.doc).collect::<Vec<_>>(), 5);
        if !selected.is_empty() {
            let mut buf = String::new();
            for doc in selected {
                let shell = document_to_shell_string(doc);
                let truncated = truncate_str(&shell, 1000);
                let _ = writeln!(buf, "{truncated}");
            }
            let cap = w.remaining().min(5000);
            let body = truncate_str(&buf, cap);
            w.section("## Sample Documents", body.trim_end());
        }
    }

    // ── 8. Aggregation pipeline + timing ───────────────────────────────────
    let enabled_stages: Vec<(usize, _)> =
        data.aggregation.stages.iter().enumerate().filter(|(_, s)| s.enabled).collect();
    if !enabled_stages.is_empty() {
        let mut buf = String::new();
        for (display_idx, (orig_idx, stage)) in enabled_stages.iter().enumerate() {
            let _ = write!(buf, "{}. {}: {}", display_idx + 1, stage.operator, stage.body.trim());
            // Stage doc counts if available (indexed by original position)
            if let Some(counts) = data.aggregation.stage_doc_counts.get(*orig_idx)
                && let Some(time_ms) = counts.time_ms
            {
                let _ = write!(buf, "  [{time_ms}ms");
                if let Some(out) = counts.output {
                    let _ = write!(buf, ", {out} docs out");
                }
                buf.push(']');
            }
            buf.push('\n');
        }
        if let Some(analysis) = &data.aggregation.analysis {
            let _ = write!(buf, "Total execution time: {}ms", analysis.total_time_ms);
        }
        let header = format!("## Aggregation Pipeline ({} stages)", enabled_stages.len());
        w.section(&header, buf.trim_end());
    }

    // ── 9. Enriched explain ────────────────────────────────────────────────
    if let Some(summary) = &data.explain.summary {
        let mut buf = String::new();
        if let Some(docs) = summary.docs_examined {
            let _ = write!(buf, "Docs examined: {docs}");
        }
        if let Some(keys) = summary.keys_examined {
            let _ = write!(buf, "\nKeys examined: {keys}");
        }
        if let Some(ret) = summary.n_returned {
            let _ = write!(buf, "\nReturned: {ret}");
        }
        if let Some(ms) = summary.execution_time_ms {
            let _ = write!(buf, "\nExecution time: {ms}ms");
        }
        if summary.has_collscan {
            buf.push_str("\nWarning: COLLSCAN (no index used)");
        }
        if summary.has_sort_stage {
            buf.push_str("\nNote: In-memory sort stage present");
        }
        if summary.is_covered_query {
            buf.push_str("\nNote: Covered query (index-only)");
        }
        if !summary.covered_indexes.is_empty() {
            let _ = write!(buf, "\nIndexes used: {}", summary.covered_indexes.join(", "));
        }
        // Bottleneck hints from explain
        for bottleneck in &data.explain.bottlenecks {
            let _ =
                write!(buf, "\nBottleneck: {} — {}", bottleneck.stage, bottleneck.recommendation);
        }
        if !buf.is_empty() {
            w.section("## Last Explain", buf.trim_end());
        }
    }

    // ── 10. Sibling collection schemas ─────────────────────────────────────
    if let Some(db) = &db_name
        && let Some(cols) = active.collections.get(db.as_str())
    {
        let mut buf = String::new();
        let mut count = 0;
        for col in cols {
            if count >= 10 {
                break;
            }
            if col_name.as_deref() == Some(col.as_str()) {
                continue;
            }
            let key = SessionKey::new(conn_id, db, col);
            if let Some(meta) = state.collection_meta(&key) {
                let compact = compact_schema_line(col, &meta.schema);
                let truncated = truncate_str(&compact, 400);
                let _ = writeln!(buf, "{truncated}");
                count += 1;
            }
        }
        if !buf.is_empty() {
            let cap = w.remaining().min(4000);
            let body = truncate_str(&buf, cap);
            w.section("## Sibling Collection Schemas", body.trim_end());
        }
    }

    // ── 11. Database-level context ─────────────────────────────────────────
    {
        let mut buf = String::new();
        // All database names
        let databases = &active.databases;
        if !databases.is_empty() {
            let _ = write!(buf, "Databases: {}", databases.join(", "));
        }
        // Sibling collection names (quick list)
        if let Some(db) = &db_name {
            if let Some(cols) = active.collections.get(db.as_str()) {
                let siblings: Vec<&str> = cols
                    .iter()
                    .filter(|c| col_name.as_deref() != Some(c.as_str()))
                    .map(|c| c.as_str())
                    .collect();
                if !siblings.is_empty() {
                    let _ = write!(
                        buf,
                        "\nOther collections in this database: {}",
                        siblings.join(", ")
                    );
                }
            }

            // Database-level stats from DatabaseSessionData
            let db_key = crate::state::DatabaseKey::new(conn_id, db);
            if let Some(db_session) = state.database_session(&db_key) {
                if let Some(stats) = &db_session.data.stats {
                    let _ = write!(buf, "\nDB stats: {} collections", stats.collections);
                    let _ = write!(buf, ", {} objects", stats.objects);
                    let _ = write!(buf, ", data size {} bytes", stats.data_size);
                    let _ = write!(buf, ", index size {} bytes", stats.index_size);
                }
                // Special collection types
                let specials: Vec<String> = db_session
                    .data
                    .collections
                    .iter()
                    .filter(|c| c.capped || c.collection_type != "collection")
                    .map(|c| {
                        if c.capped {
                            format!("{} (capped)", c.name)
                        } else {
                            format!("{} ({})", c.name, c.collection_type)
                        }
                    })
                    .collect();
                if !specials.is_empty() {
                    let _ = write!(buf, "\nSpecial collections: {}", specials.join(", "));
                }
            }
        }
        if !buf.is_empty() {
            w.section("## Database Context", buf.trim_end());
        }
    }

    w.finish()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Produce a one-line compact schema representation for a sibling collection.
fn compact_schema_line(collection: &str, schema: &crate::state::SchemaAnalysis) -> String {
    let field_summary: Vec<String> = schema
        .fields
        .iter()
        .map(|f| {
            let primary_type = f.types.first().map(|t| t.bson_type.as_str()).unwrap_or("?");
            format!("{}: {primary_type}", f.name)
        })
        .collect();
    format!("### {} ({} fields): {}", collection, schema.total_fields, field_summary.join(", "))
}

/// Greedily select documents that maximize field-name diversity.
fn select_diverse_docs<'a>(
    docs: &[&'a mongodb::bson::Document],
    max: usize,
) -> Vec<&'a mongodb::bson::Document> {
    if docs.len() <= max {
        return docs.to_vec();
    }

    let field_sets: Vec<HashSet<&str>> =
        docs.iter().map(|d| d.keys().map(|k| k.as_str()).collect::<HashSet<&str>>()).collect();

    let mut selected = Vec::with_capacity(max);
    let mut seen_fields: HashSet<&str> = HashSet::new();
    let mut used: HashSet<usize> = HashSet::new();

    for _ in 0..max {
        let best = (0..docs.len())
            .filter(|i| !used.contains(i))
            .max_by_key(|&i| field_sets[i].iter().filter(|f| !seen_fields.contains(*f)).count());
        let Some(idx) = best else { break };
        seen_fields.extend(&field_sets[idx]);
        used.insert(idx);
        selected.push(docs[idx]);
    }

    selected
}

fn format_index_keys(keys: &mongodb::bson::Document) -> String {
    let parts: Vec<String> = keys
        .iter()
        .map(|(k, v)| {
            let dir = match v {
                Bson::Int32(n) => n.to_string(),
                Bson::Int64(n) => n.to_string(),
                Bson::String(s) => format!("\"{s}\""),
                _ => format!("{v}"),
            };
            format!("{k}: {dir}")
        })
        .collect();
    format!("{{ {} }}", parts.join(", "))
}

fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let end = s.floor_char_boundary(max_bytes);
    &s[..end]
}
