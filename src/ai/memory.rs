//! Conversation memory that survives a restart.
//!
//! rig defines the interface and the trimming policies but ships no persistent store, so this is
//! ours: one SQLite file beside the app's other local data.
//!
//! Two decisions shape it. First, what is kept: the dialogue, never the tool results. A
//! `find_documents` result is a copy of production rows, and keeping those on disk would turn a
//! chat log into a durable extract of the user's database — so tool calls and their results are
//! dropped on the way in, and the model re-runs a query when it needs the rows again.
//!
//! Second, how it is kept: the file is a SQLCipher database, encrypted whole — pages, indexes,
//! journals and WAL — with a key held in the OS keychain. That is the shape desktop applications
//! settle on (Signal Desktop is the reference), and it covers the table names, row counts and
//! timestamps that encrypting each row on its own would leave in the clear. Temporary files are
//! held in memory so no page is spilled to disk unencrypted.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use rig::completion::{AssistantContent, Message as RigMessage};
use rig::memory::{ConversationMemory, MemoryError};
use rig::message::UserContent;
use rusqlite::{Connection, OptionalExtension as _, params};

/// How long a conversation is kept by default. Thirty days is what ChatGPT's temporary chats,
/// OpenAI's abuse logs and Claude Code's local transcripts all settle on. `0` keeps them until
/// the user deletes them.
pub const DEFAULT_RETENTION_DAYS: i64 = 30;

/// The most of one message that is stored. A long answer is worth keeping; a wall of pasted data
/// is not, and OWASP's agent-memory guidance asks for a size limit on anything persisted.
const MAX_STORED_CHARS: usize = 5_000;

/// One conversation in the history list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub id: String,
    /// When it was last written to, as a Unix timestamp in milliseconds.
    pub updated_ms: i64,
    /// What it was about: the first thing the user asked, shortened.
    pub title: String,
}

/// One hit from a search over past conversations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recall {
    pub conversation_id: String,
    /// When the conversation was last written to, as a Unix timestamp in milliseconds.
    pub updated_ms: i64,
    /// The matching text, as it was said.
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

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS timelines (
         conversation_id TEXT PRIMARY KEY,
         entries         TEXT NOT NULL
     );
     CREATE TABLE IF NOT EXISTS conversations (
         id          TEXT PRIMARY KEY,
         updated_ms  INTEGER NOT NULL
     );
     CREATE TABLE IF NOT EXISTS messages (
         conversation_id TEXT NOT NULL,
         seq             INTEGER NOT NULL,
         payload         TEXT NOT NULL,
         PRIMARY KEY (conversation_id, seq)
     );";

impl ChatMemory {
    pub fn open(path: PathBuf, key: [u8; 32], retention_days: i64) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Could not create {}", parent.display()))?;
        }
        let memory = match Self::open_encrypted(&path, key) {
            Ok(memory) => memory,
            Err(error) => {
                // A file written before this store was encrypted — or with a key that is gone —
                // cannot be read and is not worth keeping: it holds at most a month of chat.
                log::warn!("Starting the assistant's memory over: {error}");
                let _ = std::fs::remove_file(&path);
                for suffix in ["-wal", "-shm"] {
                    let _ = std::fs::remove_file(path.with_extension(format!("sqlite3{suffix}")));
                }
                Self::open_encrypted(&path, key)?
            }
        };
        memory.prune(retention_days)?;
        Ok(memory)
    }

    fn open_encrypted(path: &std::path::Path, key: [u8; 32]) -> Result<Self> {
        let connection =
            Connection::open(path).with_context(|| format!("Could not open {}", path.display()))?;
        // The key comes first: SQLCipher reads nothing until it is set.
        connection
            .pragma_update(None, "key", format!("x'{}'", hex(&key)))
            .context("The assistant's memory could not be unlocked")?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             -- Keep scratch pages in memory so nothing is spilled to disk in the clear.
             PRAGMA temp_store = MEMORY;",
        )?;
        // Proves the key is right; on a wrong key SQLCipher fails here rather than at open.
        connection
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get::<_, i64>(0))
            .context("The assistant's memory is not readable with this key")?;
        connection.execute_batch(&format!("{SCHEMA} PRAGMA user_version = 3;"))?;
        Ok(Self { connection: Arc::new(Mutex::new(connection)) })
    }

    /// The store used when there is nowhere to write: the conversation still works, it is just
    /// forgotten when the app closes.
    pub fn in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self { connection: Arc::new(Mutex::new(connection)) })
    }

    fn load_messages(&self, conversation_id: &str) -> Result<Vec<RigMessage>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT payload FROM messages WHERE conversation_id = ?1 ORDER BY seq")?;
        let rows = statement.query_map(params![conversation_id], |row| row.get::<_, String>(0))?;
        let mut messages = Vec::new();
        for row in rows {
            // A message this version can no longer read is skipped rather than failing the turn:
            // an old conversation is worth less than the one being had now.
            match serde_json::from_str(&row?) {
                Ok(message) => messages.push(message),
                Err(error) => log::warn!("Skipping an unreadable stored message: {error}"),
            }
        }
        Ok(messages)
    }

    fn append_messages(&self, conversation_id: &str, messages: Vec<RigMessage>) -> Result<()> {
        let messages: Vec<RigMessage> = messages.iter().filter_map(dialogue_only).collect();
        if messages.is_empty() {
            return Ok(());
        }
        let mut connection = self.lock()?;
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
            transaction.execute(
                "INSERT OR REPLACE INTO messages (conversation_id, seq, payload)
                 VALUES (?1, ?2, ?3)",
                params![conversation_id, seq, serde_json::to_string(message)?],
            )?;
        }
        transaction.execute(
            "INSERT INTO conversations (id, updated_ms) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET updated_ms = ?2",
            params![conversation_id, now],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Keep the chat as the user sees it, so reopening the app shows the conversation back.
    ///
    /// It lives here rather than in the workspace file because the workspace file is plain text:
    /// the questions and answers belong behind the same key as everything else.
    pub fn save_timeline(
        &self,
        conversation_id: &str,
        entries: &[crate::ai::AiChatEntry],
    ) -> Result<()> {
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO timelines (conversation_id, entries) VALUES (?1, ?2)
             ON CONFLICT(conversation_id) DO UPDATE SET entries = ?2",
            params![conversation_id, serde_json::to_string(entries)?],
        )?;
        connection.execute(
            "INSERT INTO conversations (id, updated_ms) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET updated_ms = ?2",
            params![conversation_id, now_ms()],
        )?;
        Ok(())
    }

    pub fn load_timeline(&self, conversation_id: &str) -> Result<Vec<crate::ai::AiChatEntry>> {
        let connection = self.lock()?;
        let entries: Option<String> = connection
            .query_row(
                "SELECT entries FROM timelines WHERE conversation_id = ?1",
                params![conversation_id],
                |row| row.get(0),
            )
            .optional()?;
        match entries {
            Some(entries) => Ok(serde_json::from_str(&entries).unwrap_or_default()),
            None => Ok(Vec::new()),
        }
    }

    /// Drop a conversation, so clearing the chat really forgets it.
    pub fn forget(&self, conversation_id: &str) -> Result<()> {
        let connection = self.lock()?;
        connection
            .execute("DELETE FROM messages WHERE conversation_id = ?1", params![conversation_id])?;
        connection.execute(
            "DELETE FROM timelines WHERE conversation_id = ?1",
            params![conversation_id],
        )?;
        connection.execute("DELETE FROM conversations WHERE id = ?1", params![conversation_id])?;
        Ok(())
    }

    /// Erase every stored conversation, for the control in Settings.
    pub fn forget_everything(&self) -> Result<()> {
        let connection = self.lock()?;
        connection.execute_batch(
            "DELETE FROM messages; DELETE FROM timelines; DELETE FROM conversations; VACUUM;",
        )?;
        Ok(())
    }

    /// Remove conversations older than `days`; `0` keeps them until the user deletes them.
    /// Runs when the store opens.
    pub fn prune(&self, days: i64) -> Result<usize> {
        if days <= 0 {
            return Ok(0);
        }
        let cutoff = now_ms() - days * 24 * 60 * 60 * 1_000;
        let connection = self.lock()?;
        for table in ["messages", "timelines"] {
            connection.execute(
                &format!(
                    "DELETE FROM {table} WHERE conversation_id IN
                         (SELECT id FROM conversations WHERE updated_ms < ?1)"
                ),
                params![cutoff],
            )?;
        }
        Ok(connection
            .execute("DELETE FROM conversations WHERE updated_ms < ?1", params![cutoff])?)
    }

    pub fn conversation_count(&self) -> Result<usize> {
        let connection = self.lock()?;
        let count: i64 =
            connection.query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// The conversations to offer, newest first.
    ///
    /// A conversation is named by the first thing that was asked in it, which is what the user
    /// will recognise; there is nowhere to put a title the model wrote, and asking it for one
    /// would cost a request per chat.
    pub fn recent(&self, exclude: &str, limit: usize) -> Result<Vec<Conversation>> {
        let rows: Vec<(String, i64)> = {
            let connection = self.lock()?;
            let mut statement = connection.prepare(
                "SELECT id, updated_ms FROM conversations
                 WHERE id <> ?1 ORDER BY updated_ms DESC, rowid DESC LIMIT ?2",
            )?;
            statement
                .query_map(params![exclude, limit as i64], |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(Result::ok)
                .collect()
        };

        let mut conversations = Vec::with_capacity(rows.len());
        for (id, updated_ms) in rows {
            let title = self
                .load_messages(&id)?
                .iter()
                .find(|message| matches!(message, RigMessage::User { .. }))
                .map(|message| snippet(&message_text(message)))
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            conversations.push(Conversation { id, updated_ms, title });
        }
        Ok(conversations)
    }

    /// Find text from earlier conversations.
    ///
    /// Nothing is indexed in the clear, so this opens each conversation and scans it, newest
    /// first. At the size a person's chat history reaches that is a few milliseconds, and it keeps
    /// the file free of the plaintext copy an index would hold.
    pub fn search(&self, query: &str, exclude: &str, limit: usize) -> Result<Vec<Recall>> {
        let needles: Vec<String> =
            query.split_whitespace().map(|word| word.to_lowercase()).collect();
        if needles.is_empty() {
            return Ok(Vec::new());
        }

        let recent: Vec<(String, i64)> = {
            let connection = self.lock()?;
            let mut statement = connection.prepare(
                "SELECT id, updated_ms FROM conversations WHERE id <> ?1 ORDER BY updated_ms DESC",
            )?;
            statement
                .query_map(params![exclude], |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(Result::ok)
                .collect()
        };

        let mut hits = Vec::new();
        for (conversation_id, updated_ms) in recent {
            if hits.len() >= limit {
                break;
            }
            for message in self.load_messages(&conversation_id)? {
                let text = message_text(&message);
                let haystack = text.to_lowercase();
                if needles.iter().all(|needle| haystack.contains(needle)) {
                    hits.push(Recall { conversation_id, updated_ms, text: snippet(&text) });
                    break;
                }
            }
        }
        Ok(hits)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| anyhow::anyhow!("the memory store is poisoned"))
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
            self.forget(conversation_id).map_err(|error| MemoryError::Internal(error.to_string()))
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// What is kept of a message: the words said, never a tool call or the rows it returned.
///
/// Dropping a tool call and its result together matters — a stored call whose result is missing is
/// a message most providers reject.
fn dialogue_only(message: &RigMessage) -> Option<RigMessage> {
    match message {
        RigMessage::User { content } => {
            let text = text_parts(content.iter().filter_map(|part| match part {
                UserContent::Text(text) => Some(text.text.clone()),
                _ => None,
            }));
            text.map(|text| RigMessage::user(clamp(text)))
        }
        RigMessage::Assistant { content, .. } => {
            let text = text_parts(content.iter().filter_map(|part| match part {
                AssistantContent::Text(text) => Some(text.text.clone()),
                _ => None,
            }));
            text.map(|text| RigMessage::assistant(clamp(text)))
        }
        // A system message is the app's own prompt, rebuilt every turn; it does not belong in a
        // stored conversation.
        RigMessage::System { .. } => None,
    }
}

/// Keeps a stored message to a sane size, so one pasted payload cannot dominate the file.
fn clamp(text: String) -> String {
    crate::helpers::truncate_chars(&text, MAX_STORED_CHARS)
}

fn text_parts(parts: impl Iterator<Item = String>) -> Option<String> {
    let joined = parts.collect::<Vec<_>>().join("\n");
    (!joined.trim().is_empty()).then_some(joined)
}

fn message_text(message: &RigMessage) -> String {
    match message {
        RigMessage::User { content } => text_parts(content.iter().filter_map(|part| match part {
            UserContent::Text(text) => Some(text.text.clone()),
            _ => None,
        }))
        .unwrap_or_default(),
        RigMessage::Assistant { content, .. } => {
            text_parts(content.iter().filter_map(|part| match part {
                AssistantContent::Text(text) => Some(text.text.clone()),
                _ => None,
            }))
            .unwrap_or_default()
        }
        RigMessage::System { content } => content.clone(),
    }
}

const SNIPPET_CHARS: usize = 160;

fn snippet(text: &str) -> String {
    crate::helpers::truncate_chars(text.trim(), SNIPPET_CHARS)
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

    /// The point of the store: what lands on disk is the dialogue, not the database.
    #[test]
    fn tool_calls_and_their_results_are_not_stored() {
        let store = memory();
        let tool_call = RigMessage::Assistant {
            id: None,
            content: vec![AssistantContent::tool_call(
                "call-1",
                "find_documents",
                serde_json::json!({ "filter": "{}" }),
            )],
        };
        let tool_result = RigMessage::tool_result(
            "call-1",
            "find_documents",
            "{\"documents\":[{\"ssn\":\"123-45-6789\"}]}",
        );

        store
            .append_messages(
                "chat-1",
                vec![RigMessage::user("who is in orders?"), tool_call, tool_result],
            )
            .expect("append");

        let loaded = store.load_messages("chat-1").expect("load");
        assert_eq!(loaded.len(), 1, "only what was said is kept");
        let stored = serde_json::to_string(&loaded).expect("json");
        assert!(!stored.contains("123-45-6789"), "no row from the database reaches the disk");
        assert!(!stored.contains("find_documents"), "no tool call is left without its result");
    }

    const KEY: [u8; 32] = [7; 32];

    /// The file is the thing an attacker gets. It must give up nothing: not the text, not even
    /// the fact that it is a SQLite database.
    #[test]
    fn the_file_on_disk_reveals_nothing() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("ai-memory.sqlite3");
        {
            let store = ChatMemory::open(path.clone(), KEY, DEFAULT_RETENTION_DAYS).expect("open");
            store
                .append_messages("chat-1", vec![RigMessage::user("secret question about payroll")])
                .expect("append");
        }

        let bytes = std::fs::read(&path).expect("read the file");
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("payroll"), "the conversation is readable on disk");
        assert!(!text.contains("chat-1"), "even the conversation id is exposed");
        assert!(!bytes.starts_with(b"SQLite format 3"), "the header names the file a database");
    }

    /// Without the keychain entry the store is unreadable. It starts over rather than failing,
    /// because a month of chat is not worth blocking the assistant on.
    #[test]
    fn another_key_does_not_open_what_was_written() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("ai-memory.sqlite3");
        {
            let store = ChatMemory::open(path.clone(), KEY, DEFAULT_RETENTION_DAYS).expect("open");
            store
                .append_messages("chat-1", vec![RigMessage::user("about payroll")])
                .expect("append");
        }

        assert!(
            ChatMemory::open_encrypted(&path, [9; 32]).is_err(),
            "the wrong key must not open the store"
        );

        let reopened = ChatMemory::open(path, [9; 32], DEFAULT_RETENTION_DAYS).expect("start over");
        assert!(reopened.load_messages("chat-1").expect("load").is_empty());
        assert_eq!(reopened.conversation_count().expect("count"), 0);
    }

    #[test]
    fn conversations_do_not_bleed_into_each_other() {
        let store = memory();
        store.append_messages("chat-1", vec![RigMessage::user("about orders")]).expect("append");
        store.append_messages("chat-2", vec![RigMessage::user("about invoices")]).expect("append");

        assert_eq!(store.load_messages("chat-1").expect("load").len(), 1);
        store.forget("chat-1").expect("forget");
        assert!(store.load_messages("chat-1").expect("load").is_empty());
        assert_eq!(store.load_messages("chat-2").expect("load").len(), 1, "the other one stands");

        store.forget_everything().expect("forget all");
        assert_eq!(store.conversation_count().expect("count"), 0);
    }

    #[test]
    fn the_history_lists_conversations_newest_first_under_what_was_asked() {
        let store = memory();
        store
            .append_messages("older", vec![RigMessage::user("how many orders shipped?")])
            .expect("write");
        store
            .append_messages("newer", vec![RigMessage::user("index the audit log")])
            .expect("write");

        let recent = store.recent("", 10).expect("recent");
        assert_eq!(
            recent.iter().map(|chat| chat.id.as_str()).collect::<Vec<_>>(),
            ["newer", "older"],
            "the one used last is the one offered first"
        );
        assert_eq!(recent[0].title, "index the audit log");

        let without_current = store.recent("newer", 10).expect("recent");
        assert_eq!(without_current.len(), 1, "the conversation in progress is not in its own list");

        // A conversation with nothing the user said has no name to show, so it is left out.
        store.append_messages("empty", vec![RigMessage::assistant("hello?")]).expect("write");
        assert!(store.recent("", 10).expect("recent").iter().all(|chat| chat.id != "empty"));
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

        let hits = store.search("auditlogs missing", "chat-now", 5).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].conversation_id, "chat-old");
        assert!(hits[0].text.contains("auditlogs"));

        // The conversation in progress stays out of its own results, and an empty query finds
        // nothing rather than everything.
        assert!(store.search("auditlogs", "chat-old", 5).expect("search").is_empty());
        assert!(store.search("   ", "none", 5).expect("search").is_empty());
    }

    #[test]
    fn a_very_long_message_is_cut_down_before_it_is_stored() {
        let store = memory();
        let wall_of_text = "x".repeat(MAX_STORED_CHARS * 3);
        store.append_messages("chat-1", vec![RigMessage::user(wall_of_text)]).expect("append");

        let loaded = store.load_messages("chat-1").expect("load");
        assert!(message_text(&loaded[0]).chars().count() <= MAX_STORED_CHARS);
    }

    #[test]
    fn keeping_conversations_for_ever_prunes_nothing() {
        let store = memory();
        store.append_messages("chat-old", vec![RigMessage::user("last year")]).expect("append");
        {
            let connection = store.connection.lock().expect("lock");
            connection.execute("UPDATE conversations SET updated_ms = 0", []).expect("age it");
        }
        assert_eq!(store.prune(0).expect("prune"), 0, "0 means keep until deleted");
        assert_eq!(store.load_messages("chat-old").expect("load").len(), 1);
    }

    #[test]
    fn conversations_are_pruned_once_they_are_old() {
        let store = memory();
        store.append_messages("chat-old", vec![RigMessage::user("last year")]).expect("append");
        {
            let connection = store.connection.lock().expect("lock");
            let long_ago = now_ms() - (DEFAULT_RETENTION_DAYS + 1) * 24 * 60 * 60 * 1_000;
            connection
                .execute("UPDATE conversations SET updated_ms = ?1", params![long_ago])
                .expect("age it");
        }
        store.append_messages("chat-new", vec![RigMessage::user("today")]).expect("append");

        assert_eq!(store.prune(DEFAULT_RETENTION_DAYS).expect("prune"), 1);
        assert!(store.load_messages("chat-old").expect("load").is_empty());
        assert_eq!(store.load_messages("chat-new").expect("load").len(), 1);
    }
}
