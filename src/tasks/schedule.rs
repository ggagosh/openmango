//! When a task runs by itself: a rule such as "daily at 02:00", stored as the rule and not as a
//! list of times, and the times it gives in the computer's time zone.

use std::path::{Path, PathBuf};

use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc,
    Weekday,
};
use serde::{Deserialize, Serialize};

/// The shortest gap an Every schedule allows.
pub const MIN_EVERY_MINUTES: u32 = 15;
/// A due time found later than this passed while nothing could run it: the computer slept, or
/// OpenMango was closed.
const LATE: Duration = Duration::minutes(2);
/// Days a rule is searched over. A month covers every rule, a monthly day in a short month too.
const SEARCH_DAYS: usize = 62;
/// How a scheduled export's file name carries its run's time: sorts by time, and has no colons,
/// which Windows doesn't allow in file names.
const STAMP: &str = "%Y-%m-%dT%H%M";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "repeat", rename_all = "snake_case")]
pub enum Schedule {
    /// Runs only when someone chooses Run now.
    #[default]
    Manual,
    /// Every `minutes`, counted from midnight each day: every 2 hours runs at 00:00, 02:00 and so
    /// on, whenever the schedule was set.
    Every {
        minutes: u32,
    },
    Daily {
        at: NaiveTime,
        #[serde(default)]
        weekdays_only: bool,
    },
    Weekly {
        days: Vec<Weekday>,
        at: NaiveTime,
    },
    /// On `day` of each month, or on the last day of a month too short for it.
    Monthly {
        day: u32,
        at: NaiveTime,
    },
}

impl Schedule {
    pub fn is_manual(&self) -> bool {
        *self == Self::Manual
    }

    /// "Daily at 02:00", "Every 2 hours", "Weekly on Mon, Thu at 07:30".
    pub fn label(&self) -> String {
        let time = |at: &NaiveTime| at.format("%H:%M").to_string();
        match self {
            Self::Manual => "Manual".into(),
            Self::Every { minutes } if minutes % 60 == 0 => match minutes / 60 {
                1 => "Every hour".into(),
                hours => format!("Every {hours} hours"),
            },
            Self::Every { minutes } => format!("Every {minutes} minutes"),
            Self::Daily { at, weekdays_only: false } => format!("Daily at {}", time(at)),
            Self::Daily { at, weekdays_only: true } => format!("Weekdays at {}", time(at)),
            Self::Weekly { days, at } => {
                let mut days = days.clone();
                days.sort_by_key(|day| day.num_days_from_monday());
                let names: Vec<String> = days.iter().map(|day| day.to_string()).collect();
                format!("Weekly on {} at {}", names.join(", "), time(at))
            }
            Self::Monthly { day, at } => format!("Monthly on day {day} at {}", time(at)),
        }
    }

    /// Whether the schedule runs at all on `date`.
    pub fn runs_on(&self, date: NaiveDate) -> bool {
        !self.times_on(date).is_empty()
    }

    /// The first run time after `after`, in its time zone.
    pub fn next_after<Tz: TimeZone>(&self, after: &DateTime<Tz>) -> Option<DateTime<Tz>> {
        let zone = after.timezone();
        let mut date = after.date_naive();
        for _ in 0..SEARCH_DAYS {
            for time in self.times_on(date) {
                if let Some(at) = resolve(&zone, date.and_time(time))
                    && at > *after
                {
                    return Some(at);
                }
            }
            date = date.succ_opt()?;
        }
        None
    }

    /// The last run time at or before `now`, if it is after `from`.
    fn last_between<Tz: TimeZone>(
        &self,
        from: &DateTime<Tz>,
        now: &DateTime<Tz>,
    ) -> Option<DateTime<Tz>> {
        let zone = now.timezone();
        let mut date = now.date_naive();
        for _ in 0..SEARCH_DAYS {
            for time in self.times_on(date).into_iter().rev() {
                if let Some(at) = resolve(&zone, date.and_time(time))
                    && at <= *now
                {
                    return (at > *from).then_some(at);
                }
            }
            date = date.pred_opt()?;
        }
        None
    }

    /// The local times the schedule runs on `date`, in order.
    fn times_on(&self, date: NaiveDate) -> Vec<NaiveTime> {
        let on = |runs: bool, at: &NaiveTime| if runs { vec![*at] } else { Vec::new() };
        match self {
            Self::Manual => Vec::new(),
            Self::Every { minutes } => {
                let step = (*minutes).max(MIN_EVERY_MINUTES) as usize;
                (0..24 * 60)
                    .step_by(step)
                    .filter_map(|minute| NaiveTime::from_hms_opt(minute / 60, minute % 60, 0))
                    .collect()
            }
            Self::Daily { at, weekdays_only } => {
                on(!weekdays_only || date.weekday().number_from_monday() <= 5, at)
            }
            Self::Weekly { days, at } => on(days.contains(&date.weekday()), at),
            Self::Monthly { day, at } => on(date.day() == (*day).clamp(1, last_day(date)), at),
        }
    }
}

fn last_day(date: NaiveDate) -> u32 {
    let (year, month) =
        if date.month() == 12 { (date.year() + 1, 1) } else { (date.year(), date.month() + 1) };
    NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|first| first.pred_opt())
        .map_or(28, |last| last.day())
}

/// The moment a local time names. A time skipped when clocks go forward runs at the first
/// minute after it; a time that happens twice when clocks go back runs at its first occurrence.
fn resolve<Tz: TimeZone>(zone: &Tz, mut local: NaiveDateTime) -> Option<DateTime<Tz>> {
    for _ in 0..=24 * 60 {
        match zone.from_local_datetime(&local) {
            LocalResult::Single(at) => return Some(at),
            LocalResult::Ambiguous(first, _) => return Some(first),
            LocalResult::None => local += Duration::minutes(1),
        }
    }
    None
}

/// What a scheduled task should do at `now`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Due {
    /// Nothing is due; the next run is at this time, if the schedule has one.
    Wait(Option<DateTime<Utc>>),
    /// Run for `due`, the latest time that has passed. A catch-up when that time passed while
    /// nothing could run it; earlier missed times are not run as well.
    Run { due: DateTime<Utc>, catch_up: bool },
    /// `due` was missed, and the next regular run at `next` is less than half an interval away,
    /// so it waits for that instead of running twice in a short time.
    Skip { due: DateTime<Utc>, next: DateTime<Utc> },
}

/// What a task whose schedule counts from `from` (the time last handled, or when the schedule was
/// set) should do at `now`, with times in `zone`.
pub fn due<Tz: TimeZone>(
    schedule: &Schedule,
    from: DateTime<Utc>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Due {
    let (from_local, now_local) = (from.with_timezone(zone), now.with_timezone(zone));
    let next = schedule.next_after(&now_local.clone().max(from_local.clone()));
    let next = next.map(|at| at.with_timezone(&Utc));
    let Some(due) = schedule.last_between(&from_local, &now_local) else {
        return Due::Wait(next);
    };
    let due = due.with_timezone(&Utc);
    if now - due <= LATE {
        return Due::Run { due, catch_up: false };
    }
    match next {
        Some(next) if (next - now) * 2 < next - due => Due::Skip { due, next },
        _ => Due::Run { due, catch_up: true },
    }
}

/// A scheduled export's file name: the run's date and time go before the extension, as in
/// `orders-2026-09-24T0200.jsonl`. A name that already has a date or time placeholder is left
/// as it is.
pub fn stamped_path(path: &str, at: NaiveDateTime) -> String {
    if names_its_own_time(path) {
        return path.to_string();
    }
    let path = Path::new(path);
    let stem = path.file_stem().map(|stem| stem.to_string_lossy()).unwrap_or_default();
    let stamp = at.format(STAMP);
    let name = match path.extension() {
        Some(extension) => format!("{stem}-{stamp}.{}", extension.to_string_lossy()),
        None => format!("{stem}-{stamp}"),
    };
    path.with_file_name(name).display().to_string()
}

/// The path has a date or time placeholder, so each run's file is already named by its time.
pub fn names_its_own_time(path: &str) -> bool {
    path.contains("${date") || path.contains("${time}")
}

/// Deletes the oldest files a scheduled export wrote for `path` until `keep` are left, and says
/// which it deleted. Only files named the way `stamped_path` names them are touched.
pub fn prune_exports(path: &Path, keep: usize) -> std::io::Result<Vec<PathBuf>> {
    let (Some(directory), Some(stem)) = (path.parent(), path.file_stem()) else {
        return Ok(Vec::new());
    };
    let prefix = format!("{}-", stem.to_string_lossy());
    let suffix = path.extension().map(|extension| format!(".{}", extension.to_string_lossy()));
    let mut stamped: Vec<(NaiveDateTime, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix(&prefix) else {
            continue;
        };
        let stamp = match &suffix {
            Some(suffix) => rest.strip_suffix(suffix.as_str()),
            None => Some(rest),
        };
        if let Some(at) = stamp.and_then(|stamp| NaiveDateTime::parse_from_str(stamp, STAMP).ok())
            && entry.file_type()?.is_file()
        {
            stamped.push((at, entry.path()));
        }
    }
    stamped.sort();
    let excess = stamped.len().saturating_sub(keep);
    let mut deleted = Vec::new();
    for (_, file) in stamped.into_iter().take(excess) {
        std::fs::remove_file(&file)?;
        deleted.push(file);
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Tz;

    const TBILISI: Tz = chrono_tz::Asia::Tbilisi;
    const BERLIN: Tz = chrono_tz::Europe::Berlin;

    fn at(hour: u32, minute: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hour, minute, 0).unwrap()
    }

    fn local(zone: Tz, text: &str) -> DateTime<Tz> {
        let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap();
        zone.from_local_datetime(&naive).earliest().unwrap()
    }

    /// The next `count` run times after `start`, as local "YYYY-MM-DD HH:MM" text.
    fn times(schedule: &Schedule, start: DateTime<Tz>, count: usize) -> Vec<String> {
        let mut times = Vec::new();
        let mut after = start;
        for _ in 0..count {
            after = schedule.next_after(&after).unwrap();
            times.push(after.format("%Y-%m-%d %H:%M %Z").to_string());
        }
        times
    }

    #[test]
    fn every_preset_gives_its_times() {
        // Thursday 24 September 2026.
        let start = local(TBILISI, "2026-09-24 10:05");
        let every = Schedule::Every { minutes: 90 };
        assert_eq!(
            times(&every, start, 2),
            ["2026-09-24 10:30 +04", "2026-09-24 12:00 +04"],
            "counted from midnight"
        );
        let daily = Schedule::Daily { at: at(2, 0), weekdays_only: false };
        assert_eq!(times(&daily, start, 1), ["2026-09-25 02:00 +04"]);
        let weekdays = Schedule::Daily { at: at(7, 30), weekdays_only: true };
        assert_eq!(
            times(&weekdays, local(TBILISI, "2026-09-25 08:00"), 2),
            ["2026-09-28 07:30 +04", "2026-09-29 07:30 +04"],
            "Friday's is over, so Monday"
        );
        let weekly = Schedule::Weekly { days: vec![Weekday::Thu, Weekday::Mon], at: at(2, 0) };
        assert_eq!(
            times(&weekly, start, 3),
            ["2026-09-28 02:00 +04", "2026-10-01 02:00 +04", "2026-10-05 02:00 +04"]
        );
        assert_eq!(Schedule::Manual.next_after(&start), None);
        assert_eq!(Schedule::Weekly { days: vec![], at: at(2, 0) }.next_after(&start), None);
    }

    #[test]
    fn a_monthly_day_past_the_months_end_runs_on_its_last_day() {
        let monthly = Schedule::Monthly { day: 31, at: at(2, 0) };
        assert_eq!(
            times(&monthly, local(TBILISI, "2027-01-31 03:00"), 3),
            ["2027-02-28 02:00 +04", "2027-03-31 02:00 +04", "2027-04-30 02:00 +04"]
        );
        let leap = Schedule::Monthly { day: 30, at: at(2, 0) };
        assert_eq!(times(&leap, local(TBILISI, "2028-02-01 00:00"), 1), ["2028-02-29 02:00 +04"]);
    }

    #[test]
    fn clocks_going_forward_run_a_skipped_time_at_the_first_valid_minute() {
        // Berlin skips 02:00-03:00 on 29 March 2026.
        let daily = Schedule::Daily { at: at(2, 30), weekdays_only: false };
        assert_eq!(
            times(&daily, local(BERLIN, "2026-03-28 12:00"), 2),
            ["2026-03-29 03:00 CEST", "2026-03-30 02:30 CEST"]
        );
        let every = Schedule::Every { minutes: 30 };
        assert_eq!(
            times(&every, local(BERLIN, "2026-03-29 01:40"), 3),
            ["2026-03-29 03:00 CEST", "2026-03-29 03:30 CEST", "2026-03-29 04:00 CEST"],
            "02:00 and 02:30 don't exist; each runs once, at 03:00"
        );
    }

    #[test]
    fn clocks_going_back_run_a_repeated_time_once() {
        // Berlin goes from 03:00 CEST back to 02:00 CET on 25 October 2026.
        let daily = Schedule::Daily { at: at(2, 30), weekdays_only: false };
        assert_eq!(
            times(&daily, local(BERLIN, "2026-10-25 00:00"), 2),
            ["2026-10-25 02:30 CEST", "2026-10-26 02:30 CET"]
        );
        let every = Schedule::Every { minutes: 60 };
        assert_eq!(
            times(&every, local(BERLIN, "2026-10-25 01:30"), 3),
            ["2026-10-25 02:00 CEST", "2026-10-25 03:00 CET", "2026-10-25 04:00 CET"],
            "the hour that happens twice runs once"
        );
    }

    #[test]
    fn a_time_that_just_passed_runs_and_a_missed_one_catches_up_once() {
        let daily = Schedule::Daily { at: at(2, 0), weekdays_only: false };
        let utc = |text: &str| local(TBILISI, text).with_timezone(&Utc);
        let from = utc("2026-09-20 12:00");
        let due_at = |now: &str| due(&daily, from, utc(now), &TBILISI);

        assert_eq!(due_at("2026-09-21 01:59"), Due::Wait(Some(utc("2026-09-21 02:00"))));
        assert_eq!(
            due_at("2026-09-21 02:00"),
            Due::Run { due: utc("2026-09-21 02:00"), catch_up: false }
        );
        // Asleep from Monday night until Thursday morning: one catch-up, for Thursday's 02:00.
        assert_eq!(
            due_at("2026-09-24 09:00"),
            Due::Run { due: utc("2026-09-24 02:00"), catch_up: true }
        );
        // Woken at 23:00, three hours before the next 02:00: that run is waited for instead.
        assert_eq!(
            due_at("2026-09-21 23:00"),
            Due::Skip { due: utc("2026-09-21 02:00"), next: utc("2026-09-22 02:00") }
        );
        // Nothing before `from` is due.
        assert_eq!(
            due(&daily, utc("2026-09-21 02:00"), utc("2026-09-21 03:00"), &TBILISI),
            Due::Wait(Some(utc("2026-09-22 02:00")))
        );
        assert_eq!(
            due(&Schedule::Manual, from, utc("2026-09-24 09:00"), &TBILISI),
            Due::Wait(None)
        );
    }

    #[test]
    fn labels_say_the_rule() {
        assert_eq!(Schedule::Every { minutes: 120 }.label(), "Every 2 hours");
        assert_eq!(Schedule::Every { minutes: 60 }.label(), "Every hour");
        assert_eq!(Schedule::Every { minutes: 45 }.label(), "Every 45 minutes");
        assert_eq!(
            Schedule::Daily { at: at(7, 30), weekdays_only: true }.label(),
            "Weekdays at 07:30"
        );
        assert_eq!(
            Schedule::Weekly { days: vec![Weekday::Thu, Weekday::Mon], at: at(2, 0) }.label(),
            "Weekly on Mon, Thu at 02:00"
        );
    }

    #[test]
    fn scheduled_exports_are_named_by_time_and_only_their_oldest_are_pruned() {
        let when = NaiveDateTime::parse_from_str("2026-09-24 02:00", "%Y-%m-%d %H:%M").unwrap();
        // Joined the way the system joins paths: with `\` on Windows.
        let data = |name: &str| std::path::Path::new("/data").join(name).display().to_string();
        assert_eq!(stamped_path("/data/orders.jsonl", when), data("orders-2026-09-24T0200.jsonl"));
        assert_eq!(stamped_path("/data/dump", when), data("dump-2026-09-24T0200"));
        assert_eq!(stamped_path("/data/orders-${date}.csv", when), "/data/orders-${date}.csv");

        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().join("orders.jsonl");
        for day in 20..25 {
            let at = when.with_day(day).unwrap();
            std::fs::write(stamped_path(&base.display().to_string(), at), "{}").unwrap();
        }
        for other in ["orders.jsonl", "orders-notes.jsonl", "orders-2026-09-19T0200.csv"] {
            std::fs::write(directory.path().join(other), "{}").unwrap();
        }
        let deleted = prune_exports(&base, 3).unwrap();
        let names: Vec<String> =
            deleted.iter().map(|path| path.file_name().unwrap().to_string_lossy().into()).collect();
        assert_eq!(names, ["orders-2026-09-20T0200.jsonl", "orders-2026-09-21T0200.jsonl"]);
        let left = std::fs::read_dir(directory.path()).unwrap().count();
        assert_eq!(left, 6, "three stamped files and the three others");
    }
}
