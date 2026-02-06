use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::assets::EmbeddedAssets;
use crate::connection::tools::node_path;
use crate::error::{Error, Result};

const CREATE_SESSION_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone)]
struct SessionInfo {
    uri: String,
    database: String,
}

#[derive(Debug, Deserialize)]
struct BridgeResponse {
    id: u64,
    ok: bool,
    result: Option<serde_json::Value>,
    error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "event")]
pub enum MongoshEvent {
    #[serde(rename = "print")]
    Print {
        session_id: String,
        run_id: Option<u64>,
        lines: Vec<String>,
        #[serde(default)]
        payload: Option<Vec<serde_json::Value>>,
    },
    #[serde(rename = "clear")]
    Clear { session_id: String },
}

#[derive(Debug, Deserialize)]
struct CompletionItem {
    completion: String,
}

#[derive(Debug, Deserialize)]
pub struct RuntimeEvaluationResult {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub result_type: Option<String>,
    pub printable: serde_json::Value,
    #[allow(dead_code)]
    pub source: Option<serde_json::Value>,
}

pub struct MongoshBridge {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<u64, std::sync::mpsc::Sender<BridgeResponse>>>>,
    next_id: AtomicU64,
    alive: Arc<AtomicBool>,
    sessions: Mutex<HashMap<Uuid, SessionInfo>>,
    events: broadcast::Sender<MongoshEvent>,
}

impl MongoshBridge {
    pub fn new() -> Result<Arc<Self>> {
        let node = node_path().ok_or_else(|| {
            Error::ToolNotFound(
                "Node runtime not found. Run 'just download-node' or install Node.js.".into(),
            )
        })?;

        let sidecar_path = write_sidecar_asset("forge/mongosh-sidecar.js")?;

        let mut cmd = Command::new(node);
        cmd.arg(sidecar_path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Parse("Failed to open sidecar stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Parse("Failed to open sidecar stdout".into()))?;
        let stderr = child.stderr.take();

        let pending: Arc<Mutex<HashMap<u64, std::sync::mpsc::Sender<BridgeResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let (event_tx, _) = broadcast::channel(1024);

        let pending_for_reader = pending.clone();
        let event_tx_reader = event_tx.clone();
        let alive_for_reader = alive.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(std::result::Result::ok) {
                let payload: serde_json::Value = match serde_json::from_str(&line) {
                    Ok(value) => value,
                    Err(err) => {
                        log::warn!("Forge sidecar JSON parse error: {}", err);
                        continue;
                    }
                };

                if payload.get("event").is_some() {
                    match serde_json::from_value::<MongoshEvent>(payload) {
                        Ok(event) => {
                            let _ = event_tx_reader.send(event);
                        }
                        Err(err) => {
                            log::warn!("Forge sidecar event parse error: {}", err);
                        }
                    }
                    continue;
                }

                let response = match serde_json::from_value::<BridgeResponse>(payload) {
                    Ok(resp) => resp,
                    Err(err) => {
                        log::warn!("Forge sidecar response parse error: {}", err);
                        continue;
                    }
                };

                let tx = pending_for_reader
                    .lock()
                    .ok()
                    .and_then(|mut pending| pending.remove(&response.id));
                if let Some(tx) = tx {
                    let _ = tx.send(response);
                }
            }
            alive_for_reader.store(false, Ordering::Release);
        });

        if let Some(stderr) = stderr {
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(std::result::Result::ok) {
                    log::warn!("Forge sidecar stderr: {}", line);
                }
            });
        }

        Ok(Arc::new(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            pending,
            next_id: AtomicU64::new(1),
            alive,
            sessions: Mutex::new(HashMap::new()),
            events: event_tx,
        }))
    }

    pub fn is_alive(&self) -> bool {
        if !self.alive.load(Ordering::Acquire) {
            return false;
        }

        match self.child.lock() {
            Ok(mut child) => match child.try_wait() {
                Ok(None) => true,
                Ok(Some(_)) => {
                    self.alive.store(false, Ordering::Release);
                    false
                }
                Err(_) => false,
            },
            Err(_) => false,
        }
    }

    pub fn ensure_session(&self, session_id: Uuid, uri: &str, database: &str) -> Result<()> {
        let existing =
            self.sessions.lock().ok().and_then(|sessions| sessions.get(&session_id).cloned());

        if let Some(info) = existing
            && info.uri == uri
            && info.database == database
        {
            return Ok(());
        }

        self.send_request(
            "create_session",
            json!({
                "session_id": session_id,
                "uri": uri,
                "database": database,
            }),
            CREATE_SESSION_TIMEOUT,
        )
        .map_err(|err| match err {
            Error::Timeout(_) => Error::Timeout(format!(
                "Timed out waiting for sidecar response (create_session, {}s). \
                 Check connection reachability/auth and sidecar logs.",
                CREATE_SESSION_TIMEOUT.as_secs()
            )),
            other => other,
        })?;

        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(
                session_id,
                SessionInfo { uri: uri.to_string(), database: database.to_string() },
            );
        }

        Ok(())
    }

    pub fn complete(&self, session_id: Uuid, code: &str, timeout: Duration) -> Result<Vec<String>> {
        let value = self.send_request(
            "complete",
            json!({
                "session_id": session_id,
                "code": code,
            }),
            timeout,
        )?;

        let items: Vec<CompletionItem> = serde_json::from_value(value)?;
        Ok(items.into_iter().map(|item| item.completion).collect())
    }

    pub fn evaluate(
        &self,
        session_id: Uuid,
        code: &str,
        run_id: Option<u64>,
        timeout: Duration,
    ) -> Result<RuntimeEvaluationResult> {
        let value = self.send_request(
            "evaluate",
            json!({
                "session_id": session_id,
                "code": code,
                "run_id": run_id,
            }),
            timeout,
        )?;

        serde_json::from_value(value).map_err(Error::from)
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<MongoshEvent> {
        self.events.subscribe()
    }

    pub fn dispose_session(&self, session_id: Uuid) -> Result<()> {
        let _ = self.send_request(
            "dispose_session",
            json!({ "session_id": session_id }),
            Duration::from_secs(8),
        );

        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&session_id);
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub fn prune_sessions(&self, keep: &HashSet<Uuid>) {
        let session_ids: Vec<Uuid> = match self.sessions.lock() {
            Ok(sessions) => sessions.keys().cloned().collect(),
            Err(_) => return,
        };

        for session_id in session_ids {
            if !keep.contains(&session_id) {
                let _ = self.dispose_session(session_id);
            }
        }
    }

    fn send_request(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = std::sync::mpsc::channel();

        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, tx);
        }

        let request = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        {
            let mut stdin = self
                .stdin
                .lock()
                .map_err(|_| Error::Parse("Failed to lock sidecar stdin for write".into()))?;
            writeln!(stdin, "{}", request)?;
        }

        let response = match rx.recv_timeout(timeout) {
            Ok(resp) => resp,
            Err(_) => {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&id);
                }
                return Err(Error::Timeout(format!(
                    "Timed out waiting for sidecar response ({})",
                    method
                )));
            }
        };

        if !response.ok {
            return Err(Error::Parse(
                response.error.unwrap_or_else(|| "Unknown sidecar error".into()),
            ));
        }

        Ok(response.result.unwrap_or(serde_json::Value::Null))
    }
}

impl Drop for MongoshBridge {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn write_sidecar_asset(path: &str) -> Result<std::path::PathBuf> {
    let data = EmbeddedAssets::get(path)
        .ok_or_else(|| Error::Parse(format!("Missing embedded asset: {}", path)))?;

    let mut target_dir = std::env::temp_dir();
    target_dir.push("openmango");
    std::fs::create_dir_all(&target_dir)?;

    let target_path = target_dir.join("mongosh-sidecar.js");
    std::fs::write(&target_path, data.data)?;

    Ok(target_path)
}
