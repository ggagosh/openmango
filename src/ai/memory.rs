//! Conversation memory that survives a restart.
//!
//! rig defines the interface and the trimming policies but ships no persistent store, so this is
//! ours: one SQLite file beside the app's other local data, holding each conversation's messages
//! exactly as rig produced them — tool calls and results included — plus a full-text index so an
//! older conversation can be found again.
//!
//! The text is stored as the model saw it. That is the same material the workspace file already
//! keeps for the visible chat, and it never leaves the machine.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use rig::completion::Message as RigMessage;
use rig::memory::{ConversationMemory, MemoryError};
use rusqlite::{Connection, OptionalExtension as _, params};

/// One hit from a search over past conversations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recall {
    pub conversation_id: String,
    /// When the conversation was last written to, as a Unix timestamp in milliseconds.
    pub updated_ms: i64,
    /// The matching text, as the model saw it.
    pub text: String,
}

#[derive(Clone)]
pub struct ChatMemory {
    // SQLite writes here are small and local; a mutex costs less than a worker thread and keeps
    // the store usable from rig's async trait without a runtime of its own.
    connection: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for ChatMemory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ChatMemory")
    }
}

impl ChatMemory {
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Could not create {}", parent.display()))?;
        }
        let connection = Connection::open(&path)
            .with_context(|| format!("Could not open {}", path.display()))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS conversations (
                 id          TEXT PRIMARY KEY,
                 updated_ms  INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS messages (
                 conversation_id TEXT NOT NULL,
                 seq             INTEGER NOT NULL,
                 payload         TEXT NOT NULL,
                 searchable      TEXT NOT NULL,
                 PRIMARY KEY (conversation_id, seq)
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts
                 USING fts5(searchable, conversation_id UNINDEXED);
             PRAGMA user_version = 1;",
        )?;
        Ok(Self { connection: Arc::new(Mutex::new(connection)) })
    }

    /// The store used when there is nowhere to write: the conversation still works, it is just
    /// forgotten when the app closes.
    pub fn in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversations (
                 id          TEXT PRIMARY KEY,
                 updated_ms  INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS messages (
                 conversation_id TEXT NOT NULL,
                 seq             INTEGER NOT NULL,
                 payload         TEXT NOT NULL,
                 searchable      TEXT NOT NULL,
                 PRIMARY KEY (conversation_id, seq)
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts
                 USING fts5(searchable, conversation_id UNINDEXED);",
        )?;
        Ok(Self { connection: Arc::new(Mutex::new(connection)) })
    }

    fn load_messages(&self, conversation_id: &str) -> Result<Vec<RigMessage>> {
        let connection =
            self.connection.lock().map_err(|_| anyhow::anyhow!("memory is poisoned"))?;
        let mut statement = connection
            .prepare("SELECT payload FROM messages WHERE conversation_id = ?1 ORDER BY seq")?;
        let rows = statement.query_map(params![conversation_id], |row| row.get::<_, String>(0))?;
        let mut messages = Vec::new();
        for row in rows {
            // A message rig can no longer read is dropped rather than failing the turn: an old
            // conversation is worth less than the one being had now.
            match serde_json::from_str(&row?) {
                Ok(message) => messages.push(message),
                Err(error) => log::warn!("Dropping an unreadable stored message: {error}"),
            }
        }
        Ok(messages)
    }

    fn append_messages(&self, conversation_id: &str, messages: Vec<RigMessage>) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }
        let mut connection =
            self.connection.lock().map_err(|_| anyhow::anyhow!("memory is poisoned"))?;
        let now = now_ms();
        let transaction = connection.transaction()?;
        let mut seq: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(seq), -1) FROM messages WHERE conversation_id = ?1",
                params![conversation_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(-1);

        for message in &messages {
            seq += 1;
            let payload = serde_json::to_string(message)?;
            let searchable = searchable_text(message);
            transaction.execute(
                "INSERT OR REPLACE INTO messages (conversation_id, seq, payload, searchable)
                 VALUES (?1, ?2, ?3, ?4)",
                params![conversation_id, seq, payload, searchable],
            )?;
            if !searchable.trim().is_empty() {
                transaction.execute(
                    "INSERT INTO messages_fts (searchable, conversation_id) VALUES (?1, ?2)",
                    params![searchable, conversation_id],
                )?;
            }
        }
        transaction.execute(
            "INSERT INTO conversations (id, updated_ms) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET updated_ms = ?2",
            params![conversation_id, now],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn clear_conversation(&self, conversation_id: &str) -> Result<()> {
        let connection =
            self.connection.lock().map_err(|_| anyhow::anyhow!("memory is poisoned"))?;
        connection.execute(
            "DELETE FROM messages_fts WHERE conversation_id = ?1",
            params![conversation_id],
        )?;
        connection
            .execute("DELETE FROM messages WHERE conversation_id = ?1", params![conversation_id])?;
        connection.execute("DELETE FROM conversations WHERE id = ?1", params![conversation_id])?;
        Ok(())
    }

    /// Drop a conversation from the store, so clearing the chat really forgets it.
    pub fn forget(&self, conversation_id: &str) -> Result<()> {
        self.clear_conversation(conversation_id)
    }

    /// Find text from earlier conversations. `exclude` keeps the conversation in progress out of
    /// its own results.
    pub fn search(&self, query: &str, exclude: &str, limit: usize) -> Result<Vec<Recall>> {
        let query = fts_query(query);
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let connection =
            self.connection.lock().map_err(|_| anyhow::anyhow!("memory is poisoned"))?;
        let mut statement = connection.prepare(
            "SELECT fts.conversation_id, snippet(messages_fts, 0, '', '', '…', 24), c.updated_ms
             FROM messages_fts AS fts
             JOIN conversations AS c ON c.id = fts.conversation_id
             WHERE messages_fts MATCH ?1 AND fts.conversation_id <> ?2
             ORDER BY rank
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![query, exclude, limit as i64], |row| {
            Ok(Recall { conversation_id: row.get(0)?, text: row.get(1)?, updated_ms: row.get(2)? })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }
}

impl ConversationMemory for ChatMemory {
    fn load<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> rig::wasm_compat::WasmBoxedFuture<'a, Result<Vec<RigMessage>, MemoryError>> {
        Box::pin(async move {
            self.load_messages(conversation_id)
                .map_err(|error| MemoryError::Internal(error.to_string()))
        })
    }

    fn append<'a>(
        &'a self,
        conversation_id: &'a str,
        messages: Vec<RigMessage>,
    ) -> rig::wasm_compat::WasmBoxedFuture<'a, Result<(), MemoryError>> {
        Box::pin(async move {
            self.append_messages(conversation_id, messages)
                .map_err(|error| MemoryError::Internal(error.to_string()))
        })
    }

    fn clear<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> rig::wasm_compat::WasmBoxedFuture<'a, Result<(), MemoryError>> {
        Box::pin(async move {
            self.clear_conversation(conversation_id)
                .map_err(|error| MemoryError::Internal(error.to_string()))
        })
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// What a message contributes to search: its words, not its JSON envelope.
fn searchable_text(message: &RigMessage) -> String {
    let value = match serde_json::to_value(message) {
        Ok(value) => value,
        Err(_) => return String::new(),
    };
    let mut text = String::new();
    collect_text(&value, &mut text);
    text
}

fn collect_text(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::String(string) => {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(string);
        }
        serde_json::Value::Array(items) => items.iter().for_each(|item| collect_text(item, out)),
        serde_json::Value::Object(fields) => {
            fields.values().for_each(|field| collect_text(field, out))
        }
        _ => {}
    }
}

/// FTS5 reads bare input as syntax, so each word is quoted and the last one is a prefix match —
/// which is what someone typing half a collection name expects.
fn fts_query(query: &str) -> String {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}\""))
        .collect();
    match words.split_last() {
        None => String::new(),
        Some((last, rest)) => {
            let mut parts: Vec<String> = rest.to_vec();
            parts.push(format!("{last}*"));
            parts.join(" AND ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> ChatMemory {
        ChatMemory::in_memory().expect("memory store")
    }

    #[test]
    fn a_conversation_comes_back_in_the_order_it_was_written() {
        let store = memory();
        store
            .append_messages(
                "chat-1",
                vec![RigMessage::user("how many orders?"), RigMessage::assistant("3,229")],
            )
            .expect("append");
        store.append_messages("chat-1", vec![RigMessage::user("and customers?")]).expect("append");

        let loaded = store.load_messages("chat-1").expect("load");
        assert_eq!(loaded.len(), 3);
        assert!(matches!(loaded.first(), Some(RigMessage::User { .. })));
        assert!(matches!(loaded.get(1), Some(RigMessage::Assistant { .. })));
    }

    #[test]
    fn conversations_do_not_bleed_into_each_other() {
        let store = memory();
        store.append_messages("chat-1", vec![RigMessage::user("about orders")]).expect("append");
        store.append_messages("chat-2", vec![RigMessage::user("about invoices")]).expect("append");

        assert_eq!(store.load_messages("chat-1").expect("load").len(), 1);
        assert_eq!(store.load_messages("chat-2").expect("load").len(), 1);

        store.clear_conversation("chat-1").expect("clear");
        assert!(store.load_messages("chat-1").expect("load").is_empty());
        assert_eq!(store.load_messages("chat-2").expect("load").len(), 1, "the other one stands");
    }

    #[test]
    fn an_earlier_conversation_can_be_found_by_what_was_said_in_it() {
        let store = memory();
        store
            .append_messages(
                "chat-old",
                vec![RigMessage::user("which auditlogs are missing a userId?")],
            )
            .expect("append");
        store.append_messages("chat-now", vec![RigMessage::user("unrelated")]).expect("append");

        let hits = store.search("auditlogs", "chat-now", 5).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].conversation_id, "chat-old");
        assert!(hits[0].text.contains("auditlogs"));

        // Half a word still finds it, and the conversation in progress stays out of its own hits.
        assert_eq!(store.search("auditl", "chat-now", 5).expect("search").len(), 1);
        assert!(store.search("auditlogs", "chat-old", 5).expect("search").is_empty());
    }

    #[test]
    fn punctuation_in_a_question_does_not_break_the_search() {
        let store = memory();
        store
            .append_messages("chat-old", vec![RigMessage::user("orders.status values?")])
            .expect("append");

        assert_eq!(store.search("orders.status", "none", 5).expect("search").len(), 1);
        assert!(store.search("   ", "none", 5).expect("search").is_empty());
    }
}
