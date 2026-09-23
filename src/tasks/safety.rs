//! When a run that writes should stop before writing: the safety limit.
//!
//! It catches mistakes nobody is there to see, such as a source that was emptied or half
//! restored, or a task pointing at the wrong database. Inserts never count: they destroy nothing.

use serde::{Deserialize, Serialize};

use super::model::{Run, RunStatus, RunTrigger};
use crate::helpers::format_number;

/// The safety limit's three numbers, kept per task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SafetyLimit {
    /// Stop above this share of the target collection, in percent.
    pub percent: u32,
    /// Stop above this multiple of the most the task changed in its recent successful runs.
    pub jump: u32,
    /// Changes smaller than this never stop a run.
    pub floor: u64,
}

impl Default for SafetyLimit {
    fn default() -> Self {
        Self { percent: 10, jump: 3, floor: 100 }
    }
}

/// Successful runs the jump rule looks back over, and how many it needs first.
const HISTORY_RUNS: usize = 10;
const HISTORY_NEEDED: usize = 3;

/// What a run is about to do to one target collection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Planned {
    pub name: String,
    pub inserts: u64,
    pub replaces: u64,
    pub deletes: u64,
    /// Documents the target collection holds now.
    pub target_documents: u64,
    /// Documents the source holds, when known. Zero means an empty source.
    pub source_documents: Option<u64>,
    /// Replacing the whole target is the point (Clear or Drop target first), so the share of the
    /// target isn't a sign of a mistake.
    pub replaces_whole_target: bool,
}

impl Planned {
    pub fn destructive(&self) -> u64 {
        self.replaces + self.deletes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopReason {
    TooLarge { name: String, count: u64, target: u64, percent: u32 },
    Jump { name: String, count: u64, usual: u64 },
    EmptySource { name: String, target: u64 },
}

impl StopReason {
    pub fn describe(&self) -> String {
        match self {
            Self::TooLarge { name, count, target, percent } => format!(
                "Would delete or replace {} of the {} documents in {name}, more than {percent}%.",
                format_number(*count),
                format_number(*target)
            ),
            Self::Jump { name, count, usual } => format!(
                "Would delete or replace {} documents in {name}; this task usually changes at most {}.",
                format_number(*count),
                format_number(*usual)
            ),
            Self::EmptySource { name, target } => format!(
                "The source for {name} is empty, so this would remove all {} documents it holds.",
                format_number(*target)
            ),
        }
    }
}

/// Why the run should stop before writing; empty when it may go ahead. `history` is the
/// task's runs, newest first.
pub fn check(limit: &SafetyLimit, planned: &[Planned], history: &[Run]) -> Vec<StopReason> {
    let successful: Vec<&Run> = history
        .iter()
        .filter(|run| run.trigger == RunTrigger::Manual && run.status == RunStatus::Succeeded)
        .take(HISTORY_RUNS)
        .collect();
    let mut reasons = Vec::new();
    for plan in planned {
        let count = plan.destructive();
        if plan.source_documents == Some(0) && plan.target_documents > 0 && count > 0 {
            reasons.push(StopReason::EmptySource {
                name: plan.name.clone(),
                target: plan.target_documents,
            });
            continue;
        }
        if count < limit.floor.max(1) {
            continue;
        }
        let share = count.saturating_mul(100);
        if !plan.replaces_whole_target
            && plan.target_documents > 0
            && share > plan.target_documents.saturating_mul(limit.percent as u64)
        {
            reasons.push(StopReason::TooLarge {
                name: plan.name.clone(),
                count,
                target: plan.target_documents,
                percent: limit.percent,
            });
            continue;
        }
        if successful.len() >= HISTORY_NEEDED {
            let usual = successful
                .iter()
                .filter_map(|run| run.collections.iter().find(|c| c.name == plan.name))
                .filter_map(|c| c.planned)
                .map(|[_, replaces, deletes]| replaces + deletes)
                .max()
                .unwrap_or(0);
            if count > usual.saturating_mul(limit.jump as u64) {
                reasons.push(StopReason::Jump { name: plan.name.clone(), count, usual });
            }
        }
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn plan(name: &str, replaces: u64, deletes: u64, target: u64) -> Planned {
        Planned {
            name: name.into(),
            replaces,
            deletes,
            target_documents: target,
            source_documents: Some(target),
            ..Default::default()
        }
    }

    fn succeeded(name: &str, changed: u64) -> Run {
        let mut run = Run::start(Uuid::nil(), RunTrigger::Manual);
        run.collection_mut(name).planned = Some([0, changed, 0]);
        run.finish(false);
        run
    }

    #[test]
    fn more_than_ten_percent_stops_but_small_changes_never_do() {
        let limit = SafetyLimit::default();
        let over = check(&limit, &[plan("orders", 0, 1_001, 10_000)], &[]);
        assert!(matches!(over.as_slice(), [StopReason::TooLarge { count: 1_001, .. }]));
        assert!(check(&limit, &[plan("orders", 0, 1_000, 10_000)], &[]).is_empty());
        // 99 of 100 is 99%, but under the floor.
        assert!(check(&limit, &[plan("tiny", 99, 0, 100)], &[]).is_empty());
        // Inserts never count.
        let inserts = Planned { inserts: 1_000_000, ..plan("orders", 0, 0, 10) };
        assert!(check(&limit, &[inserts], &[]).is_empty());
    }

    #[test]
    fn an_unusual_jump_stops_once_the_task_has_history() {
        let limit = SafetyLimit::default();
        // 300,000 of 50 million is under 10%; 5 million is exactly 10% and doesn't trip it.
        let usual = plan("orders", 300_000, 0, 50_000_000);
        let accident = plan("orders", 0, 5_000_000, 50_000_000);
        assert!(check(&limit, std::slice::from_ref(&accident), &[]).is_empty(), "no history yet");

        let two = vec![succeeded("orders", 310_000), succeeded("orders", 290_000)];
        assert!(
            check(&limit, std::slice::from_ref(&accident), &two).is_empty(),
            "needs three runs"
        );

        let mut three = two.clone();
        three.push(succeeded("orders", 305_000));
        assert!(check(&limit, &[usual], &three).is_empty(), "its usual volume passes");
        let reasons = check(&limit, &[accident], &three);
        assert!(matches!(
            reasons.as_slice(),
            [StopReason::Jump { count: 5_000_000, usual: 310_000, .. }]
        ));
        assert!(reasons[0].describe().contains("usually changes at most 310,000"));
    }

    #[test]
    fn previews_failures_and_stopped_runs_are_not_history() {
        let mut preview = succeeded("orders", 1_000_000);
        preview.trigger = RunTrigger::Preview;
        let mut failed = succeeded("orders", 1_000_000);
        failed.status = RunStatus::Failed;
        let history = vec![
            preview,
            failed,
            succeeded("orders", 10),
            succeeded("orders", 10),
            succeeded("orders", 10),
        ];
        let reasons =
            check(&SafetyLimit::default(), &[plan("orders", 500, 0, 1_000_000)], &history);
        assert!(matches!(reasons.as_slice(), [StopReason::Jump { usual: 10, .. }]));
    }

    #[test]
    fn an_empty_source_stops_even_below_the_floor_and_replacing_the_target_is_allowed() {
        let limit = SafetyLimit::default();
        let empty = Planned { source_documents: Some(0), ..plan("orders", 0, 20, 20) };
        assert!(matches!(
            check(&limit, &[empty], &[]).as_slice(),
            [StopReason::EmptySource { .. }]
        ));
        let refresh = Planned { replaces_whole_target: true, ..plan("orders", 0, 5_000, 5_000) };
        assert!(check(&limit, &[refresh], &[]).is_empty());
    }
}
