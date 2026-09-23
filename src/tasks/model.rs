use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::connection::ops::compare::{CompareCounts, Side};
use crate::connection::ops::compare_database::SyncMode;
use crate::connection::ops::compare_sync::SyncSummary;
use crate::state::app_state::{TransferConfig, TransferMode, TransferOptions};
use crate::state::compare::{CompareConfig, CompareScope};

pub const TASK_FORMAT_VERSION: u32 = 1;
/// Log lines kept per run. Later lines are counted, not stored.
pub const RUN_LOG_LIMIT: usize = 1_000;

/// A saved Transfer or Compare setup that can be run again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub version: u32,
    pub id: Uuid,
    pub name: String,
    pub spec: TaskSpec,
    #[serde(default)]
    pub safety: super::safety::SafetyLimit,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    pub fn new(name: String, spec: TaskSpec) -> Self {
        let now = Utc::now();
        Self {
            version: TASK_FORMAT_VERSION,
            id: Uuid::new_v4(),
            name,
            spec,
            safety: Default::default(),
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskSpec {
    /// Export, Import or Copy, with the settings the Transfer tab holds.
    Transfer { config: TransferConfig, options: TransferOptions },
    /// A comparison that only reports, in either scope.
    Compare { config: CompareConfig },
    /// Writes the differences between two databases into `target`, collection by collection.
    /// Every collection the source has is synced except `excluded`, so collections added later
    /// are included.
    Sync { config: CompareConfig, target: Side, mode: SyncMode, excluded: Vec<String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    Export,
    Import,
    Copy,
    Compare,
    Sync,
}

impl TaskKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Export => "Export",
            Self::Import => "Import",
            Self::Copy => "Copy",
            Self::Compare => "Compare",
            Self::Sync => "Sync",
        }
    }
}

impl TaskSpec {
    pub fn kind(&self) -> TaskKind {
        match self {
            Self::Transfer { config, .. } => match config.mode {
                TransferMode::Export => TaskKind::Export,
                TransferMode::Import => TaskKind::Import,
                TransferMode::Copy => TaskKind::Copy,
            },
            Self::Compare { .. } => TaskKind::Compare,
            Self::Sync { .. } => TaskKind::Sync,
        }
    }

    /// The connection a run writes to, if it writes to MongoDB at all.
    pub fn write_connection(&self) -> Option<Uuid> {
        match self {
            Self::Transfer { config, .. } => match config.mode {
                TransferMode::Export => None,
                TransferMode::Import => config.source_connection_id,
                TransferMode::Copy => config.destination_connection_id,
            },
            Self::Compare { .. } => None,
            Self::Sync { config, target, .. } => config.sides[side_index(*target)].connection_id,
        }
    }

    /// Every saved connection the task uses.
    pub fn connections(&self) -> Vec<Uuid> {
        let ids = match self {
            Self::Transfer { config, .. } => match config.mode {
                TransferMode::Copy => {
                    vec![config.source_connection_id, config.destination_connection_id]
                }
                _ => vec![config.source_connection_id],
            },
            Self::Compare { config } | Self::Sync { config, .. } => {
                config.sides.iter().map(|side| side.connection_id).collect()
            }
        };
        let mut unique = Vec::new();
        for id in ids.into_iter().flatten() {
            if !unique.contains(&id) {
                unique.push(id);
            }
        }
        unique
    }

    /// A starting name for the Save as task dialog, e.g. "Export orders" or "Mirror shop to local".
    pub fn default_name(&self) -> String {
        let pick = |collection: &str, database: &str| {
            if collection.is_empty() { database.to_string() } else { collection.to_string() }
        };
        match self {
            Self::Transfer { config, .. } => {
                let subject = pick(&config.source_collection, &config.source_database);
                match config.mode {
                    TransferMode::Export => format!("Export {subject}"),
                    TransferMode::Import => format!("Import into {subject}"),
                    TransferMode::Copy => format!("Copy {subject}"),
                }
            }
            Self::Compare { config } => {
                let side = &config.sides[0];
                match config.scope {
                    CompareScope::Collections => {
                        format!("Compare {}", pick(&side.collection, &side.database))
                    }
                    CompareScope::Databases => format!("Compare {}", side.database),
                }
            }
            Self::Sync { config, target, mode, .. } => {
                let [left, right] = config.sides.each_ref().map(|side| side.database.as_str());
                let (source, destination) =
                    if *target == Side::Right { (left, right) } else { (right, left) };
                format!("{} {source} to {destination}", mode.label())
            }
        }
    }

    /// One line saying what the task works on, e.g. "shop.orders → local/shop.orders".
    pub fn subject(&self) -> String {
        match self {
            Self::Transfer { config, .. } => {
                let source = namespace(&config.source_database, &config.source_collection);
                match config.mode {
                    TransferMode::Export => format!("{source} → {}", config.file_path),
                    TransferMode::Import => format!("{} → {source}", config.file_path),
                    TransferMode::Copy => format!(
                        "{source} → {}",
                        namespace(&config.destination_database, &config.destination_collection)
                    ),
                }
            }
            Self::Compare { config } => {
                let [left, right] = compare_sides(config);
                format!("{left} ↔ {right}")
            }
            Self::Sync { config, target, mode, .. } => {
                let [left, right] = compare_sides(config);
                let (source, destination) = match target {
                    Side::Right => (left, right),
                    Side::Left => (right, left),
                };
                format!("{} · {source} → {destination}", mode.label())
            }
        }
    }
}

pub fn side_index(side: Side) -> usize {
    match side {
        Side::Left => 0,
        Side::Right => 1,
    }
}

fn namespace(database: &str, collection: &str) -> String {
    if collection.is_empty() { database.to_string() } else { format!("{database}.{collection}") }
}

fn compare_sides(config: &CompareConfig) -> [String; 2] {
    config.sides.clone().map(|side| match config.scope {
        CompareScope::Collections => namespace(&side.database, &side.collection),
        CompareScope::Databases => side.database,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTrigger {
    /// Run now, from the Tasks tab.
    Manual,
    /// Works out what a run would change, and writes nothing.
    Preview,
    /// Reverts the task's last sync run.
    Undo,
}

impl RunTrigger {
    pub fn label(self) -> &'static str {
        match self {
            Self::Manual => "Run now",
            Self::Preview => "Preview",
            Self::Undo => "Undo",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    /// Some collections finished and some failed.
    PartlyDone,
    Failed,
    Cancelled,
    /// Still marked running when the app started: it was cut short by a crash or a forced quit.
    Interrupted,
}

impl RunStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Succeeded => "Succeeded",
            Self::PartlyDone => "Partly done",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
            Self::Interrupted => "Interrupted",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub at: DateTime<Utc>,
    pub level: LogLevel,
    pub message: String,
}

/// What happened to one collection during a run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectionRun {
    pub name: String,
    /// Documents exported, imported or copied.
    pub documents: u64,
    pub differences: Option<CompareCounts>,
    pub writes: Option<SyncSummary>,
    /// What the run worked out it would insert, replace and delete, before writing.
    pub planned: Option<[u64; 3]>,
    /// Why the collection was left alone, e.g. "Exists only on the left".
    pub note: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: Uuid,
    pub task_id: Uuid,
    pub trigger: RunTrigger,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub collections: Vec<CollectionRun>,
    /// Why the run as a whole failed, when it failed before or outside any one collection.
    pub error: Option<String>,
    /// Why the safety limit stopped, or would stop, this run.
    #[serde(default)]
    pub stops: Vec<String>,
    pub log: Vec<LogLine>,
    pub log_dropped: u64,
}

impl Run {
    pub fn start(task_id: Uuid, trigger: RunTrigger) -> Self {
        Self {
            id: Uuid::new_v4(),
            task_id,
            trigger,
            status: RunStatus::Running,
            started_at: Utc::now(),
            finished_at: None,
            collections: Vec::new(),
            error: None,
            stops: Vec::new(),
            log: Vec::new(),
            log_dropped: 0,
        }
    }

    pub fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        if self.log.len() < RUN_LOG_LIMIT {
            self.log.push(LogLine { at: Utc::now(), level, message: message.into() });
        } else {
            self.log_dropped += 1;
        }
    }

    pub fn collection_mut(&mut self, name: &str) -> &mut CollectionRun {
        if let Some(index) = self.collections.iter().position(|run| run.name == name) {
            return &mut self.collections[index];
        }
        self.collections.push(CollectionRun { name: name.to_string(), ..Default::default() });
        self.collections.last_mut().expect("just pushed")
    }

    /// Ends the run. The status follows from what happened unless it was cancelled.
    pub fn finish(&mut self, cancelled: bool) {
        let failed = self.collections.iter().filter(|run| run.error.is_some()).count();
        self.status = if cancelled {
            RunStatus::Cancelled
        } else if self.error.is_some() || (failed > 0 && failed == self.collections.len()) {
            RunStatus::Failed
        } else if failed > 0 {
            RunStatus::PartlyDone
        } else {
            RunStatus::Succeeded
        };
        self.finished_at = Some(Utc::now());
    }

    pub fn documents(&self) -> u64 {
        self.collections.iter().map(|run| run.documents).sum()
    }

    pub fn differences(&self) -> CompareCounts {
        let mut total = CompareCounts::default();
        for counts in self.collections.iter().filter_map(|run| run.differences) {
            total.identical += counts.identical;
            total.only_left += counts.only_left;
            total.only_right += counts.only_right;
            total.different += counts.different;
            total.minor += counts.minor;
        }
        total
    }

    pub fn writes(&self) -> SyncSummary {
        let mut total = SyncSummary::default();
        for summary in self.collections.iter().filter_map(|run| run.writes.as_ref()) {
            total.absorb(summary);
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with(errors: &[bool]) -> Run {
        let mut run = Run::start(Uuid::new_v4(), RunTrigger::Manual);
        for (index, failed) in errors.iter().enumerate() {
            let collection = run.collection_mut(&format!("c{index}"));
            if *failed {
                collection.error = Some("boom".into());
            }
        }
        run
    }

    #[test]
    fn a_run_is_partly_done_only_when_some_collections_failed() {
        let mut all_good = run_with(&[false, false]);
        all_good.finish(false);
        assert_eq!(all_good.status, RunStatus::Succeeded);

        let mut some = run_with(&[false, true]);
        some.finish(false);
        assert_eq!(some.status, RunStatus::PartlyDone);

        let mut all = run_with(&[true, true]);
        all.finish(false);
        assert_eq!(all.status, RunStatus::Failed);

        let mut before_any = run_with(&[]);
        before_any.error = Some("Could not connect".into());
        before_any.finish(false);
        assert_eq!(before_any.status, RunStatus::Failed);

        let mut cancelled = run_with(&[true]);
        cancelled.finish(true);
        assert_eq!(cancelled.status, RunStatus::Cancelled);
    }

    #[test]
    fn the_log_keeps_its_first_lines_and_counts_the_rest() {
        let mut run = run_with(&[]);
        for line in 0..RUN_LOG_LIMIT + 5 {
            run.log(LogLevel::Info, format!("line {line}"));
        }
        assert_eq!(run.log.len(), RUN_LOG_LIMIT);
        assert_eq!(run.log_dropped, 5);
        assert_eq!(run.log[0].message, "line 0");
    }

    #[test]
    fn a_task_round_trips_and_names_its_write_connection() {
        let target = Uuid::new_v4();
        let mut config = CompareConfig { scope: CompareScope::Databases, ..Default::default() };
        config.sides[0].connection_id = Some(Uuid::new_v4());
        config.sides[1].connection_id = Some(target);
        let task = Task::new(
            "Nightly mirror".into(),
            TaskSpec::Sync {
                config,
                target: Side::Right,
                mode: SyncMode::Mirror,
                excluded: vec!["audit_log".into()],
            },
        );
        let json = serde_json::to_string(&task).unwrap();
        let back: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(back, task);
        assert_eq!(task.spec.kind(), TaskKind::Sync);
        assert_eq!(task.spec.write_connection(), Some(target));
        assert_eq!(task.spec.connections().len(), 2);
    }
}
