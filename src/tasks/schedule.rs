//! When a task runs by itself: a rule such as "daily at 02:00", stored as the rule and not as a
//! list of times, and the times it gives in the computer's time zone.

use std::path::{Path, PathBuf};

use jiff::civil::{Date, DateTime, Time, Weekday};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{SignedDuration, Timestamp, ZonedDateTime};
use serde::{Deserialize, Serialize};

/// The shortest gap an Every schedule allows.
pub const MIN_EVERY_MINUTES: u32 = 15;
/// A due time found later than this passed while nothing could run it: the computer slept, or
/// OpenMango was closed.
const LATE: SignedDuration = SignedDuration::from_mins(2);
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
    Every { minutes: u32 },
    Daily {
        at: Time,
        #[serde(default)]
        weekdays_only: bool,
    },
    Weekly {
        #[serde(with = "weekday_names")]
        days: Vec<Weekday>,
        at: Time,
    },
    /// On `day` of each month, or on the last day of a month too short for it.
    Monthly { day: u32, at: Time },
}

impl Schedule {
    pub fn is_manual(&self) -> bool {
        *self == Self::Manual
    }

    /// "Daily at 02:00", "Every 2 hours", "Weekly on Mon, Thu at 07:30".
    pub fn label(&self) -> String {
        let time = |at: &Time| at.strftime("%H:%M").to_string();
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
                days.sort_by_key(|day| day.to_monday_zero_offset());
                let names: Vec<&str> = days.iter().map(|day| weekday_names::name(*day)).collect();
                format!("Weekly on {} at {}", names.join(", "), time(at))
            }
            Self::Monthly { day, at } => format!("Monthly on day {day} at {}", time(at)),
        }
    }

    /// Whether the schedule runs at all on `date`.
    pub fn runs_on(&self, date: Date) -> bool {
        !self.times_on(date).is_empty()
    }

    /// The first run time after `after`, in its time zone.
    pub fn next_after(&self, after: &ZonedDateTime) -> Option<ZonedDateTime> {
        let mut date = after.date();
        for _ in 0..SEARCH_DAYS {
            for time in self.times_on(date) {
                if let Some(at) = resolve(after.time_zone(), date.to_datetime(time))
                    && at > *after
                {
                    return Some(at);
                }
            }
            date = date.tomorrow().ok()?;
        }
        None
    }

    /// The last run time at or before `now`, if it is after `from`.
    fn last_between(&self, from: &ZonedDateTime, now: &ZonedDateTime) -> Option<ZonedDateTime> {
        let mut date = now.date();
        for _ in 0..SEARCH_DAYS {
            for time in self.times_on(date).into_iter().rev() {
                if let Some(at) = resolve(now.time_zone(), date.to_datetime(time))
                    && at <= *now
                {
                    return (at > *from).then_some(at);
                }
            }
            date = date.yesterday().ok()?;
        }
        None
    }

    /// The local times the schedule runs on `date`, in order.
    fn times_on(&self, date: Date) -> Vec<Time> {
        let on = |runs: bool, at: &Time| if runs { vec![*at] } else { Vec::new() };
        match self {
            Self::Manual => Vec::new(),
            Self::Every { minutes } => {
                let step = (*minutes).max(MIN_EVERY_MINUTES) as usize;
                (0..24 * 60)
                    .step_by(step)
                    .filter_map(|minute| {
                        Time::new((minute / 60) as i8, (minute % 60) as i8, 0, 0).ok()
                    })
                    .collect()
            }
            Self::Daily { at, weekdays_only } => {
                on(!weekdays_only || date.weekday().to_monday_one_offset() <= 5, at)
            }
            Self::Weekly { days, at } => on(days.contains(&date.weekday()), at),
            Self::Monthly { day, at } => {
                on(date.day() as u32 == (*day).clamp(1, date.days_in_month() as u32), at)
            }
        }
    }
}

/// The moment a local time names. A time skipped when clocks go forward runs at the first
/// minute after it; a time that happens twice when clocks go back runs at its first occurrence.
fn resolve(zone: &TimeZone, mut local: DateTime) -> Option<ZonedDateTime> {
    for _ in 0..=24 * 60 {
        let at = zone.to_ambiguous_zoned_datetime(local);
        match at.offset() {
            AmbiguousOffset::Gap { .. } => {
                local = local.checked_add(SignedDuration::from_mins(1)).ok()?
            }
            AmbiguousOffset::Unambiguous { .. } | AmbiguousOffset::Fold { .. } => {
                return at.earlier().ok();
            }
        }
    }
    None
}

/// What a scheduled task should do at `now`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Due {
    /// Nothing is due; the next run is at this time, if the schedule has one.
    Wait(Option<Timestamp>),
    /// Run for `due`, the latest time that has passed. A catch-up when that time passed while
    /// nothing could run it; earlier missed times are not run as well.
    Run { due: Timestamp, catch_up: bool },
    /// `due` was missed, and the next regular run at `next` is less than half an interval away,
    /// so it waits for that instead of running twice in a short time.
    Skip { due: Timestamp, next: Timestamp },
}

/// What a task whose schedule counts from `from` (the time last handled, or when the schedule was
/// set) should do at `now`, with times in `zone`.
pub fn due(schedule: &Schedule, from: Timestamp, now: Timestamp, zone: &TimeZone) -> Due {
    let (from_local, now_local) =
        (from.to_zoned_datetime(zone.clone()), now.to_zoned_datetime(zone.clone()));
    let next = schedule.next_after(&now_local.clone().max(from_local.clone()));
    let next = next.map(|at| at.timestamp());
    let Some(due) = schedule.last_between(&from_local, &now_local) else {
        return Due::Wait(next);
    };
    let due = due.timestamp();
    if now.duration_since(due) <= LATE {
        return Due::Run { due, catch_up: false };
    }
    match next {
        Some(next) if next.duration_since(now) * 2 < next.duration_since(due) => {
            Due::Skip { due, next }
        }
        _ => Due::Run { due, catch_up: true },
    }
}

/// A scheduled export's file name: the run's date and time go before the extension, as in
/// `orders-2026-09-24T0200.jsonl`. A name that already has a date or time placeholder is left
/// as it is.
pub fn stamped_path(path: &str, at: DateTime) -> String {
    if names_its_own_time(path) {
        return path.to_string();
    }
    let path = Path::new(path);
    let stem = path.file_stem().map(|stem| stem.to_string_lossy()).unwrap_or_default();
    let stamp = at.strftime(STAMP);
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
    let mut stamped: Vec<(DateTime, PathBuf)> = Vec::new();
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
        if let Some(at) = stamp.and_then(|stamp| DateTime::strptime(STAMP, stamp).ok())
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

/// Weekdays are saved by their short English names, "Mon" to "Sun". Reading also takes full
/// names, in any case.
pub mod weekday_names {
    use jiff::civil::Weekday;
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    const NAMES: [(Weekday, &str); 7] = [
        (Weekday::Monday, "Mon"),
        (Weekday::Tuesday, "Tue"),
        (Weekday::Wednesday, "Wed"),
        (Weekday::Thursday, "Thu"),
        (Weekday::Friday, "Fri"),
        (Weekday::Saturday, "Sat"),
        (Weekday::Sunday, "Sun"),
    ];

    pub fn name(day: Weekday) -> &'static str {
        NAMES[day.to_monday_zero_offset() as usize].1
    }

    pub fn parse(text: &str) -> Option<Weekday> {
        let text = text.to_ascii_lowercase();
        NAMES
            .iter()
            .find(|(_, short)| text.starts_with(&short.to_ascii_lowercase()))
            .map(|(day, _)| *day)
    }

    pub fn serialize<S: Serializer>(days: &[Weekday], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(days.iter().map(|day| name(*day)))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Weekday>, D::Error> {
        Vec::<String>::deserialize(deserializer)?
            .iter()
            .map(|text| {
                parse(text).ok_or_else(|| D::Error::custom(format!("unknown weekday `{text}`")))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tbilisi() -> TimeZone {
        TimeZone::get("Asia/Tbilisi").unwrap()
    }

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    fn at(hour: i8, minute: i8) -> Time {
        Time::new(hour, minute, 0, 0).unwrap()
    }

    fn local(zone: TimeZone, text: &str) -> ZonedDateTime {
        let local = DateTime::strptime("%Y-%m-%d %H:%M", text).unwrap();
        zone.to_ambiguous_zoned_datetime(local).earlier().unwrap()
    }

    /// The next `count` run times after `start`, as local "YYYY-MM-DD HH:MM" text.
    fn times(schedule: &Schedule, start: &ZonedDateTime, count: usize) -> Vec<String> {
        let mut times = Vec::new();
        let mut after = start.clone();
        for _ in 0..count {
            after = schedule.next_after(&after).unwrap();
            times.push(after.strftime("%Y-%m-%d %H:%M %Z").to_string());
        }
        times
    }

    #[test]
    fn every_preset_gives_its_times() {
        // Thursday 24 September 2026.
        let start = local(tbilisi(), "2026-09-24 10:05");
        let every = Schedule::Every { minutes: 90 };
        assert_eq!(
            times(&every, &start, 2),
            ["2026-09-24 10:30 +04", "2026-09-24 12:00 +04"],
            "counted from midnight"
        );
        let daily = Schedule::Daily { at: at(2, 0), weekdays_only: false };
        assert_eq!(times(&daily, &start, 1), ["2026-09-25 02:00 +04"]);
        let weekdays = Schedule::Daily { at: at(7, 30), weekdays_only: true };
        assert_eq!(
            times(&weekdays, &local(tbilisi(), "2026-09-25 08:00"), 2),
            ["2026-09-28 07:30 +04", "2026-09-29 07:30 +04"],
            "Friday's is over, so Monday"
        );
        let weekly =
            Schedule::Weekly { days: vec![Weekday::Thursday, Weekday::Monday], at: at(2, 0) };
        assert_eq!(
            times(&weekly, &start, 3),
            ["2026-09-28 02:00 +04", "2026-10-01 02:00 +04", "2026-10-05 02:00 +04"]
        );
        assert_eq!(Schedule::Manual.next_after(&start), None);
        assert_eq!(Schedule::Weekly { days: vec![], at: at(2, 0) }.next_after(&start), None);
    }

    #[test]
    fn a_monthly_day_past_the_months_end_runs_on_its_last_day() {
        let monthly = Schedule::Monthly { day: 31, at: at(2, 0) };
        assert_eq!(
            times(&monthly, &local(tbilisi(), "2027-01-31 03:00"), 3),
            ["2027-02-28 02:00 +04", "2027-03-31 02:00 +04", "2027-04-30 02:00 +04"]
        );
        let leap = Schedule::Monthly { day: 30, at: at(2, 0) };
        assert_eq!(
            times(&leap, &local(tbilisi(), "2028-02-01 00:00"), 1),
            ["2028-02-29 02:00 +04"]
        );
    }

    #[test]
    fn clocks_going_forward_run_a_skipped_time_at_the_first_valid_minute() {
        // Berlin skips 02:00-03:00 on 29 March 2026.
        let daily = Schedule::Daily { at: at(2, 30), weekdays_only: false };
        assert_eq!(
            times(&daily, &local(berlin(), "2026-03-28 12:00"), 2),
            ["2026-03-29 03:00 CEST", "2026-03-30 02:30 CEST"]
        );
        let every = Schedule::Every { minutes: 30 };
        assert_eq!(
            times(&every, &local(berlin(), "2026-03-29 01:40"), 3),
            ["2026-03-29 03:00 CEST", "2026-03-29 03:30 CEST", "2026-03-29 04:00 CEST"],
            "02:00 and 02:30 don't exist; each runs once, at 03:00"
        );
    }

    #[test]
    fn clocks_going_back_run_a_repeated_time_once() {
        // Berlin goes from 03:00 CEST back to 02:00 CET on 25 October 2026.
        let daily = Schedule::Daily { at: at(2, 30), weekdays_only: false };
        assert_eq!(
            times(&daily, &local(berlin(), "2026-10-25 00:00"), 2),
            ["2026-10-25 02:30 CEST", "2026-10-26 02:30 CET"]
        );
        let every = Schedule::Every { minutes: 60 };
        assert_eq!(
            times(&every, &local(berlin(), "2026-10-25 01:30"), 3),
            ["2026-10-25 02:00 CEST", "2026-10-25 03:00 CET", "2026-10-25 04:00 CET"],
            "the hour that happens twice runs once"
        );
    }

    #[test]
    fn a_time_that_just_passed_runs_and_a_missed_one_catches_up_once() {
        let daily = Schedule::Daily { at: at(2, 0), weekdays_only: false };
        let utc = |text: &str| local(tbilisi(), text).timestamp();
        let from = utc("2026-09-20 12:00");
        let due_at = |now: &str| due(&daily, from, utc(now), &tbilisi());

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
            due(&daily, utc("2026-09-21 02:00"), utc("2026-09-21 03:00"), &tbilisi()),
            Due::Wait(Some(utc("2026-09-22 02:00")))
        );
        assert_eq!(
            due(&Schedule::Manual, from, utc("2026-09-24 09:00"), &tbilisi()),
            Due::Wait(None)
        );
    }

    /// Saved schedules from before jiff keep reading, and are written the same way.
    #[test]
    fn schedules_keep_their_saved_form() {
        let saved = r#"{"repeat":"weekly","days":["Thu","Mon"],"at":"02:00:00"}"#;
        let schedule: Schedule = serde_json::from_str(saved).unwrap();
        assert_eq!(
            schedule,
            Schedule::Weekly { days: vec![Weekday::Thursday, Weekday::Monday], at: at(2, 0) }
        );
        assert_eq!(serde_json::to_string(&schedule).unwrap(), saved);
        let full: Schedule =
            serde_json::from_str(r#"{"repeat":"weekly","days":["monday"],"at":"02:00:00"}"#)
                .unwrap();
        assert_eq!(full, Schedule::Weekly { days: vec![Weekday::Monday], at: at(2, 0) });
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
            Schedule::Weekly { days: vec![Weekday::Thursday, Weekday::Monday], at: at(2, 0) }
                .label(),
            "Weekly on Mon, Thu at 02:00"
        );
    }

    #[test]
    fn scheduled_exports_are_named_by_time_and_only_their_oldest_are_pruned() {
        let when = DateTime::strptime("%Y-%m-%d %H:%M", "2026-09-24 02:00").unwrap();
        // Joined the way the system joins paths: with `\` on Windows.
        let data = |name: &str| std::path::Path::new("/data").join(name).display().to_string();
        assert_eq!(stamped_path("/data/orders.jsonl", when), data("orders-2026-09-24T0200.jsonl"));
        assert_eq!(stamped_path("/data/dump", when), data("dump-2026-09-24T0200"));
        assert_eq!(stamped_path("/data/orders-${date}.csv", when), "/data/orders-${date}.csv");

        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().join("orders.jsonl");
        for day in 20..25 {
            let at = when.with().day(day).build().unwrap();
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
