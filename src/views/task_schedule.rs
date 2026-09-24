//! The schedule editor: when a task runs by itself and, for a task that writes, its safety
//! limit. The next three run times update as you edit; that is the check that the schedule means
//! what you intended.

use std::rc::Rc;
use std::sync::Arc;

use chrono::{Local, NaiveTime, Weekday};
use gpui_kit::component::button::{ButtonGroup, ButtonVariants as _};
use gpui_kit::component::calendar::{Calendar, CalendarState, Date, Matcher};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{Dialog, DialogFooter};
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, Size, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use uuid::Uuid;

use crate::components::{Button, cancel_button, open_confirm_dialog};
use crate::connection::ops::compare_database::SyncMode;
use crate::helpers::background_runner::RunnerStatus;
use crate::state::app_state::ScheduleSettings;
use crate::state::{AppState, StatusMessage, TransferMode};
use crate::tasks::model::{Task, TaskSpec};
use crate::tasks::safety::SafetyLimit;
use crate::tasks::schedule::{MIN_EVERY_MINUTES, Schedule, names_its_own_time, stamped_path};
use crate::theme::spacing;

/// How many files a scheduled export keeps when it keeps only its newest.
const KEEP_FILES: u32 = 30;
const DAYS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Repeat {
    Manual,
    Every,
    Daily,
    Weekly,
    Monthly,
}

const REPEATS: [(Repeat, &str); 5] = [
    (Repeat::Manual, "Manual"),
    (Repeat::Every, "Every…"),
    (Repeat::Daily, "Daily"),
    (Repeat::Weekly, "Weekly"),
    (Repeat::Monthly, "Monthly"),
];

/// Tabular figures, so times and counts line up.
pub(crate) fn tabular() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

/// The computer's time zone, e.g. "Asia/Tbilisi", or its offset when the name is unknown.
pub(crate) fn zone_name() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| Local::now().format("UTC%:z").to_string())
}

/// "Fri, Sep 25, 02:00".
pub(crate) fn run_time(at: chrono::DateTime<Local>) -> String {
    at.format("%a, %b %-d, %H:%M").to_string()
}

pub struct ScheduleEditor {
    state: Entity<AppState>,
    task: Task,
    repeat: Repeat,
    hours: bool,
    days: Vec<Weekday>,
    weekdays_only: bool,
    keep_files: bool,
    notify_success: bool,
    run_when_closed: bool,
    /// The system entry that starts tasks while OpenMango is closed, as last seen.
    runner: RunnerStatus,
    protected_writes: bool,
    /// The Production or protected connection the task writes to, by name.
    protected_target: Option<String>,
    every: Entity<InputState>,
    time: Entity<InputState>,
    day: Entity<InputState>,
    percent: Entity<InputState>,
    jump: Entity<InputState>,
    floor: Entity<InputState>,
    /// Shows the days the schedule runs, with the next run selected.
    calendar: Entity<CalendarState>,
    save_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ScheduleEditor {
    fn new(
        state: Entity<AppState>,
        task: Task,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (mut repeat, mut hours, mut every) = (Repeat::Manual, true, 1);
        let (mut at, mut days, mut weekdays_only, mut day) =
            (NaiveTime::from_hms_opt(2, 0, 0).unwrap_or_default(), vec![Weekday::Mon], false, 1);
        match &task.schedule {
            Schedule::Manual => {}
            Schedule::Every { minutes } => {
                repeat = Repeat::Every;
                hours = minutes % 60 == 0;
                every = if hours { minutes / 60 } else { *minutes };
            }
            Schedule::Daily { at: time, weekdays_only: only } => {
                (repeat, at, weekdays_only) = (Repeat::Daily, *time, *only);
            }
            Schedule::Weekly { days: chosen, at: time } => {
                (repeat, at, days) = (Repeat::Weekly, *time, chosen.clone());
            }
            Schedule::Monthly { day: chosen, at: time } => {
                (repeat, at, day) = (Repeat::Monthly, *time, *chosen);
            }
        }
        let mut input = |value: String, placeholder: &str| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value))
        };
        let every = input(every.to_string(), "1");
        let time = input(at.format("%H:%M").to_string(), "02:00");
        let day = input(day.to_string(), "1");
        let percent = input(task.safety.percent.to_string(), "10");
        let jump = input(task.safety.jump.to_string(), "3");
        let floor = input(task.safety.floor.to_string(), "100");
        let subscriptions = [&every, &time, &day, &percent, &jump, &floor]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |editor: &mut Self, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        editor.save_error = None;
                        cx.notify();
                    }
                })
            })
            .collect();
        let calendar = cx.new(|cx| CalendarState::new(window, cx));
        let protected_target = state.read(cx).task_protected_target(&task);
        Self {
            repeat,
            hours,
            days,
            weekdays_only,
            keep_files: task.keep_files.is_some(),
            notify_success: task.notify_success,
            run_when_closed: task.run_when_closed,
            // Asked here, since the app only asks at launch when a task uses it.
            runner: crate::helpers::background_runner::status(),
            protected_writes: task
                .approval
                .as_ref()
                .is_some_and(|approval| approval.protected_writes),
            protected_target,
            every,
            time,
            day,
            percent,
            jump,
            floor,
            calendar,
            save_error: None,
            _subscriptions: subscriptions,
            state,
            task,
        }
    }

    fn number(input: &Entity<InputState>, cx: &App) -> Option<u64> {
        input.read(cx).value().trim().parse().ok()
    }

    fn at(&self, cx: &App) -> Result<NaiveTime, String> {
        NaiveTime::parse_from_str(self.time.read(cx).value().trim(), "%H:%M")
            .map_err(|_| "Enter a time like 02:00, on a 24-hour clock.".into())
    }

    fn every_minutes(&self, cx: &App) -> Result<u32, String> {
        let count = Self::number(&self.every, cx).ok_or("Enter a whole number.")?;
        let minutes = if self.hours { count.saturating_mul(60) } else { count };
        if minutes < MIN_EVERY_MINUTES as u64 {
            return Err(format!("Use {MIN_EVERY_MINUTES} minutes or more."));
        }
        if minutes > 24 * 60 {
            return Err("Use 24 hours or less.".into());
        }
        Ok(minutes as u32)
    }

    fn month_day(&self, cx: &App) -> Result<u32, String> {
        Self::number(&self.day, cx)
            .filter(|day| (1..=31).contains(day))
            .map(|day| day as u32)
            .ok_or_else(|| "Use a day from 1 to 31.".into())
    }

    /// The schedule the fields describe, or the first thing to fix.
    fn schedule(&self, cx: &App) -> Result<Schedule, String> {
        Ok(match self.repeat {
            Repeat::Manual => Schedule::Manual,
            Repeat::Every => Schedule::Every { minutes: self.every_minutes(cx)? },
            Repeat::Daily => {
                Schedule::Daily { at: self.at(cx)?, weekdays_only: self.weekdays_only }
            }
            Repeat::Weekly if self.days.is_empty() => return Err("Choose at least one day.".into()),
            Repeat::Weekly => Schedule::Weekly { days: self.days.clone(), at: self.at(cx)? },
            Repeat::Monthly => Schedule::Monthly { day: self.month_day(cx)?, at: self.at(cx)? },
        })
    }

    fn safety(&self, cx: &App) -> Result<SafetyLimit, String> {
        let percent = Self::number(&self.percent, cx)
            .filter(|percent| (1..=100).contains(percent))
            .ok_or("Use a percentage from 1 to 100.")?;
        let jump = Self::number(&self.jump, cx)
            .filter(|jump| *jump >= 1)
            .ok_or("Use 1 or more for how many times the usual.")?;
        let floor = Self::number(&self.floor, cx).ok_or("Use a whole number of documents.")?;
        Ok(SafetyLimit { percent: percent as u32, jump: jump as u32, floor })
    }

    fn writes(&self) -> bool {
        self.task.spec.write_connection().is_some()
    }

    fn mirror(&self) -> bool {
        matches!(self.task.spec, TaskSpec::Sync { mode: SyncMode::Mirror, .. })
    }

    /// The export's path, when each scheduled run names its file by its time.
    fn stamped_export(&self) -> Option<&str> {
        match &self.task.spec {
            TaskSpec::Transfer { config, .. }
                if config.mode == TransferMode::Export
                    && !config.file_path.is_empty()
                    && !names_its_own_time(&config.file_path) =>
            {
                Some(&config.file_path)
            }
            _ => None,
        }
    }

    /// Saves, after asking when the schedule lets the task write to Production on its own or
    /// delete as a Mirror.
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Ok(schedule), Ok(safety)) = (self.schedule(cx), self.safety(cx)) else {
            return;
        };
        let scheduled = !schedule.is_manual();
        if scheduled && self.protected_target.is_some() && !self.protected_writes {
            return;
        }
        let keep_files = (self.keep_files && self.stamped_export().is_some()).then_some(KEEP_FILES);
        let protected_writes = self.protected_writes;
        let notify_success = self.notify_success;
        let run_when_closed = self.run_when_closed;
        let apply = {
            let (state, id, name) = (self.state.clone(), self.task.id, self.task.name.clone());
            move |window: &mut Window, cx: &mut App| -> Result<(), String> {
                let next = schedule.next_after(&Local::now());
                let label = schedule.label();
                let result = state.update(cx, |app, cx| {
                    let settings = ScheduleSettings {
                        schedule,
                        safety,
                        keep_files,
                        protected_writes,
                        notify_success,
                        run_when_closed,
                    };
                    let result = app.set_task_schedule(id, settings);
                    if result.is_ok() {
                        let message = match next {
                            Some(next) => format!(
                                "“{name}” is scheduled: {}. Next run {}.",
                                label.to_lowercase(),
                                run_time(next)
                            ),
                            None => format!("“{name}” runs only when you choose Run now."),
                        };
                        app.set_status_message(Some(StatusMessage::info(message)));
                    }
                    cx.notify();
                    result
                });
                if result.is_ok() {
                    window.close_dialog(cx);
                }
                result.map_err(|error| format!("Couldn't save the schedule: {error}"))
            }
        };
        let was_manual = self.task.schedule.is_manual();
        let was_allowed =
            self.task.approval.as_ref().is_some_and(|approval| approval.protected_writes);
        let mut reasons = Vec::new();
        if scheduled
            && protected_writes
            && !was_allowed
            && let Some(target) = &self.protected_target
        {
            reasons.push(format!(
                "{target} is a Production or protected connection. Each scheduled run of “{}” \
                 writes to it without asking.",
                self.task.name
            ));
        }
        if scheduled && was_manual && self.mirror() {
            reasons.push(
                "Mirror deletes what the target has and the source doesn't, and a scheduled run \
                 does it without asking."
                    .to_string(),
            );
        }
        if reasons.is_empty() {
            self.save_error = apply(window, cx).err();
            return cx.notify();
        }
        let (title, confirm) = match &self.protected_target {
            Some(target) if protected_writes && !was_allowed => {
                (format!("Allow scheduled writes to {target}?"), "Allow scheduled writes")
            }
            _ => (format!("Run “{}” on a schedule?", self.task.name), "Schedule Mirror"),
        };
        let message = format!(
            "{}\n\nThe safety limit still stops a run that would delete or replace more than {}% \
             of a collection.",
            reasons.join("\n\n"),
            safety.percent
        );
        let editor = cx.entity();
        open_confirm_dialog(window, cx, title, message, confirm, true, move |window, cx| {
            // The question closes itself after this; the editor closes once it has.
            window.defer(cx, move |window, cx| {
                if let Err(error) = apply(window, cx) {
                    editor.update(cx, |editor, cx| {
                        editor.save_error = Some(error);
                        cx.notify();
                    });
                }
            });
        });
    }

    /// A number field of a fixed width. On its own a number input stretches or shrinks with its
    /// row, and a shrunk one hides its value between its buttons.
    fn number_field(input: &Entity<InputState>) -> Div {
        div().flex().w(px(120.0)).flex_none().child(NumberInput::new(input).small())
    }

    fn label(text: &'static str) -> Div {
        div().text_sm().child(text)
    }

    fn error(message: Option<String>, cx: &App) -> Option<Div> {
        message.map(|message| div().text_xs().text_color(cx.theme().danger).child(message))
    }

    fn time_field(&self, cx: &App) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .child(Self::label("Time"))
            .child(div().flex().w(px(120.0)).flex_none().child(Input::new(&self.time).small()))
            .children(Self::error(self.at(cx).err(), cx))
    }

    fn render_rule(&mut self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let rule = div().flex().flex_col().gap(spacing::md());
        match self.repeat {
            Repeat::Manual => rule.child(
                div().text_sm().text_color(muted).child("Runs only when you choose Run now."),
            ),
            Repeat::Every => {
                rule.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .child(Self::label("Every"))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(spacing::sm())
                                .child(Self::number_field(&self.every))
                                .child(
                                    ButtonGroup::new("schedule-every-unit")
                                        .compact()
                                        .child(Self::choice("Minutes", !self.hours))
                                        .child(Self::choice("Hours", self.hours))
                                        .on_click(cx.listener(
                                            |editor, selection: &Vec<usize>, _, cx| {
                                                editor.hours = selection.first() == Some(&1);
                                                cx.notify();
                                            },
                                        )),
                                ),
                        )
                        .child(div().text_xs().text_color(muted).child(
                            "Counted from midnight: every 2 hours runs at 00:00, 02:00, 04:00…",
                        ))
                        .children(Self::error(self.every_minutes(cx).err(), cx)),
                )
            }
            Repeat::Daily => rule.child(self.time_field(cx)).child(
                Checkbox::new("schedule-weekdays-only")
                    .label("Weekdays only, Monday to Friday")
                    .checked(self.weekdays_only)
                    .on_click(cx.listener(|editor, checked: &bool, _, cx| {
                        editor.weekdays_only = *checked;
                        cx.notify();
                    })),
            ),
            Repeat::Weekly => rule
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .child(Self::label("Days"))
                        .child(
                            ButtonGroup::new("schedule-days")
                                .compact()
                                .multiple(true)
                                .children(DAYS.iter().map(|day| {
                                    Self::choice(&day.to_string(), self.days.contains(day))
                                }))
                                .on_click(cx.listener(|editor, selection: &Vec<usize>, _, cx| {
                                    editor.days =
                                        selection.iter().map(|index| DAYS[*index]).collect();
                                    cx.notify();
                                })),
                        )
                        .children(Self::error(
                            self.days.is_empty().then(|| "Choose at least one day.".to_string()),
                            cx,
                        )),
                )
                .child(self.time_field(cx)),
            Repeat::Monthly => rule
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .child(Self::label("Day of the month"))
                        .child(Self::number_field(&self.day))
                        .when(self.month_day(cx).is_ok_and(|day| day > 28), |field| {
                            field.child(div().text_xs().text_color(muted).child(
                                "In a month without this day, it runs on the month's last day.",
                            ))
                        })
                        .children(Self::error(self.month_day(cx).err(), cx)),
                )
                .child(self.time_field(cx)),
        }
    }

    fn choice(label: &str, selected: bool) -> Button {
        Button::new(SharedString::from(label.to_string()))
            .label(label.to_string())
            .selected(selected)
            .with_size(Size::Small)
    }

    fn render_next(&self, schedule: &Schedule, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut times = Vec::new();
        let mut after = Local::now();
        while times.len() < 3
            && let Some(next) = schedule.next_after(&after)
        {
            times.push(next);
            after = next;
        }
        let list = div().flex().flex_col().gap(spacing::xs()).children(
            times.iter().enumerate().map(|(index, at)| {
                div()
                    .text_sm()
                    .font_features(tabular())
                    .when(index > 0, |line| line.text_color(muted))
                    .child(run_time(*at))
            }),
        );
        // Every so many minutes runs every day, so a calendar would say nothing the times don't.
        let calendar = (!matches!(schedule, Schedule::Every { .. })).then(|| {
            // The days it doesn't run are dimmed, and the next run is selected. The date is set
            // again only when it changed, so paging to another month stays there.
            let runs = schedule.clone();
            let next = Date::Single(times.first().map(|at| at.date_naive()));
            self.calendar.update(cx, |calendar, _| {
                calendar.set_disabled_matcher_shared(Some(Rc::new(Matcher::from(
                    move |date: &chrono::NaiveDate| !runs.runs_on(*date),
                ))));
                if calendar.date() != next {
                    calendar.apply_date(next);
                }
            });
            Calendar::new(&self.calendar).first_day_of_week(Weekday::Mon).small()
        });
        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .child(Self::label("Next runs"))
            .child(div().flex().items_start().gap(spacing::lg()).children(calendar).child(list))
            .child(
                div().text_xs().text_color(muted).child(format!(
                    "Local time, {}. Runs start while OpenMango is open.",
                    zone_name()
                )),
            )
    }

    /// "Run even when OpenMango is closed", or why this OpenMango can't.
    fn render_closed(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let (available, note) = match self.runner {
            // So the runner can be tried in development: nothing starts it by itself there.
            RunnerStatus::Unavailable(_) if cfg!(debug_assertions) => (
                true,
                "Development build: nothing starts the runner by itself. Run `cargo run -- \
                 --run-due-tasks` once the task is due, with OpenMango closed."
                    .to_string(),
            ),
            RunnerStatus::Unavailable(why) => (false, why.to_string()),
            _ => (
                true,
                "OpenMango looks for due tasks about every 15 minutes, so a run can start up to \
                 15 minutes late. It's listed in System Settings under Login Items."
                    .to_string(),
            ),
        };
        div()
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .child(
                Checkbox::new("schedule-run-when-closed")
                    .label("Run even when OpenMango is closed")
                    .checked(available && self.run_when_closed)
                    .disabled(!available)
                    .on_click(cx.listener(|editor, checked: &bool, _, cx| {
                        editor.run_when_closed = *checked;
                        cx.notify();
                    })),
            )
            .child(div().pl(px(24.0)).text_xs().text_color(muted).child(note))
    }

    fn render_export(&self, path: &str, schedule: &Schedule, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let example = schedule
            .next_after(&Local::now())
            .map(|next| stamped_path(path, next.naive_local()))
            .and_then(|path| {
                std::path::Path::new(&path)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .child(
                div().text_xs().text_color(muted).child(format!(
                    "Each run writes a new file named by its time, like {example}."
                )),
            )
            .child(
                Checkbox::new("schedule-keep-files")
                    .label(format!("Keep only the newest {KEEP_FILES} files"))
                    .checked(self.keep_files)
                    .on_click(cx.listener(|editor, checked: &bool, _, cx| {
                        editor.keep_files = *checked;
                        cx.notify();
                    })),
            )
    }

    fn render_safety(&self, scheduled: bool, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        // One label column, one field width, then the unit: the three rows share their edges.
        let row = |label: &'static str, input: &Entity<InputState>, unit: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .child(div().w(px(176.0)).flex_none().text_sm().child(label))
                .child(Self::number_field(input))
                .child(div().text_sm().text_color(muted).child(unit))
        };
        let mut section = div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child("Safety limit"))
                    .child(div().text_xs().text_color(muted).child(
                        "A run stops before it writes when, in any collection, it would delete or \
                         replace more than either limit.",
                    )),
            )
            .child(row("Share of collection", &self.percent, "%"))
            .child(row("Times the usual", &self.jump, "×"))
            .child(row("Ignore changes under", &self.floor, "documents"))
            .child(div().text_xs().text_color(muted).child(
                "The usual is the most it changed in one of its last 10 runs, counted from its \
                 third run.",
            ))
            .children(Self::error(self.safety(cx).err(), cx));
        if let Some(target) = self.protected_target.clone() {
            let missing = scheduled && !self.protected_writes;
            section = section.child(
                div()
                    .pt(spacing::xs())
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        Checkbox::new("schedule-protected-writes")
                            .label(format!("Allow scheduled writes to {target}"))
                            .checked(self.protected_writes)
                            .on_click(cx.listener(|editor, checked: &bool, _, cx| {
                                editor.protected_writes = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if missing { cx.theme().danger } else { muted })
                            .child(if missing {
                                format!(
                                    "{target} is Production or protected. Turn this on to schedule \
                                     this task, or keep it Manual."
                                )
                            } else {
                                format!(
                                    "{target} is Production or protected, so scheduled runs write \
                                     to it only when this is on."
                                )
                            }),
                    ),
            );
        }
        section
    }
}

impl Render for ScheduleEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let schedule = self.schedule(cx).ok();
        let scheduled = schedule.as_ref().is_some_and(|schedule| !schedule.is_manual());
        let repeat = ButtonGroup::new("schedule-repeat")
            .compact()
            .children(
                REPEATS.iter().map(|(repeat, label)| Self::choice(label, self.repeat == *repeat)),
            )
            .on_click(cx.listener(|editor, selection: &Vec<usize>, _, cx| {
                if let Some((repeat, _)) = selection.first().and_then(|index| REPEATS.get(*index)) {
                    editor.repeat = *repeat;
                    editor.save_error = None;
                    cx.notify();
                }
            }));
        let rule = self.render_rule(cx);
        let export = self.stamped_export().map(str::to_string);
        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .p(spacing::md())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(Self::label("Repeat"))
                    .child(repeat),
            )
            .child(rule)
            .when_some(schedule.filter(|schedule| !schedule.is_manual()), |editor, schedule| {
                let editor = editor
                    .child(self.render_next(&schedule, cx))
                    .child(
                        Checkbox::new("schedule-notify-success")
                            .label("Also notify when a scheduled run succeeds")
                            .checked(self.notify_success)
                            .on_click(cx.listener(|editor, checked: &bool, _, cx| {
                                editor.notify_success = *checked;
                                cx.notify();
                            })),
                    )
                    .when(cfg!(target_os = "macos"), |editor| editor.child(self.render_closed(cx)));
                match &export {
                    Some(path) => editor.child(self.render_export(path, &schedule, cx)),
                    None => editor,
                }
            })
            .when(self.writes(), |editor| editor.child(self.render_safety(scheduled, cx)))
            .children(Self::error(self.save_error.clone(), cx))
    }
}

/// Opens the schedule editor for the task, and returns it.
pub fn open_schedule_dialog(
    state: Entity<AppState>,
    task_id: Uuid,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<ScheduleEditor>> {
    let task = state.read(cx).task(task_id).cloned()?;
    let title = if task.spec.write_connection().is_some() {
        format!("Schedule and safety for “{}”", task.name)
    } else {
        format!("Schedule “{}”", task.name)
    };
    let editor = cx.new(|cx| ScheduleEditor::new(state, task, window, cx));
    let opened = editor.clone();
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
        let save = editor.clone();
        dialog.title(title.clone()).w(px(500.0)).child(editor.clone()).footer(
            DialogFooter::new().children(vec![
                cancel_button("cancel-schedule"),
                Button::new("save-schedule")
                    .primary()
                    .label("Save schedule")
                    .on_click(move |_, window, cx| {
                        save.update(cx, |editor, cx| editor.save(window, cx))
                    })
                    .into_any_element(),
            ]),
        )
    });
    Some(opened)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{Datelike as _, NaiveTime};
    use gpui_kit::component::{Root, WindowExt as _};
    use gpui_kit::{
        AppContext as _, Entity, ParentElement as _, Styled as _, TestAppContext,
        VisualTestContext, px, size,
    };

    use super::{Repeat, open_schedule_dialog};
    use crate::connection::ops::compare::Side;
    use crate::connection::ops::compare_database::SyncMode;
    use crate::models::{ConnectionEnvironment, SavedConnection};
    use crate::state::AppState;
    use crate::state::ConfigManager;
    use crate::state::compare::CompareConfig;
    use crate::tasks::model::{Task, TaskSpec};
    use crate::tasks::schedule::Schedule;
    use gpui_kit::component::input::InputState;

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }

    fn type_into(input: &Entity<InputState>, text: &str, cx: &mut VisualTestContext) {
        cx.update(|window, cx| input.update(cx, |input, cx| input.set_value(text, window, cx)));
    }

    /// The room each number field on screen leaves for its value, between its − and + buttons.
    fn number_rooms(cx: &mut VisualTestContext) -> Vec<gpui_kit::Pixels> {
        let nodes = cx.update(|window, _| gpui_kit::base::test_support::snapshots(window));
        let buttons = |label: &str| -> Vec<gpui_kit::Bounds<gpui_kit::Pixels>> {
            nodes
                .iter()
                .filter(|node| node.label() == Some(label))
                .map(|node| node.bounds())
                .collect()
        };
        let increments = buttons("Increment");
        buttons("Decrement")
            .into_iter()
            .filter_map(|decrement| {
                let increment = increments
                    .iter()
                    .find(|increment| increment.center().y == decrement.center().y)?;
                Some(increment.left() - decrement.right())
            })
            .collect()
    }

    #[gpui_kit::test]
    fn the_editor_saves_a_schedule_and_asks_before_scheduled_production_writes(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply_design_tokens(cx);
            crate::keyboard::bind_keymap(cx, &Default::default());
        });
        let directory = tempfile::tempdir().unwrap();
        let source = SavedConnection::new("Staging".into(), "mongodb://localhost:1".into());
        let mut target = SavedConnection::new("Live".into(), "mongodb://localhost:2".into());
        target.environment = Some(ConnectionEnvironment::Production);
        let state = cx.new(|_| {
            let mut app = AppState::with_config(
                Arc::new(crate::connection::ConnectionManager::new()),
                ConfigManager::with_config_dir(directory.path().into()),
            );
            app.connections = vec![source.clone(), target.clone()];
            app
        });
        let compare =
            Task::new("Check".into(), TaskSpec::Compare { config: CompareConfig::default() });
        let mut config = CompareConfig::default();
        config.sides[0].connection_id = Some(source.id);
        config.sides[1].connection_id = Some(target.id);
        let mirror = Task::new(
            "Mirror to live".into(),
            TaskSpec::Sync {
                config,
                target: Side::Right,
                mode: SyncMode::Mirror,
                excluded: vec![],
            },
        );
        state.update(cx, |app, _| {
            app.upsert_task(compare.clone()).unwrap();
            app.upsert_task(mirror.clone()).unwrap();
        });
        struct DialogHost;
        impl gpui_kit::Render for DialogHost {
            fn render(
                &mut self,
                window: &mut gpui_kit::Window,
                cx: &mut gpui_kit::Context<Self>,
            ) -> impl gpui_kit::IntoElement {
                gpui_kit::div().size_full().children(Root::render_dialog_layer(window, cx))
            }
        }
        let (_, cx) = cx.add_window_view(|window, cx| {
            let host = cx.new(|_| DialogHost);
            Root::new(host, window, cx).bordered(false)
        });
        cx.simulate_resize(size(px(1200.0), px(900.0)));

        // Every kind of rule draws; a daily time saves.
        let editor = cx
            .update(|window, cx| open_schedule_dialog(state.clone(), compare.id, window, cx))
            .unwrap();
        for repeat in [Repeat::Every, Repeat::Weekly, Repeat::Monthly, Repeat::Daily] {
            editor.update(cx, |editor, _| editor.repeat = repeat);
            draw(cx);
            if repeat == Repeat::Weekly {
                // Weekly on Mondays: the calendar dims the other days and selects the next Monday.
                let calendar = editor.read_with(cx, |editor, _| editor.calendar.clone());
                let (next_monday, dims_tuesday, dims_monday) =
                    calendar.read_with(cx, |calendar, _| {
                        let next = calendar.date().start().unwrap();
                        let dims = |date| {
                            calendar
                                .disabled_matcher_ref()
                                .is_some_and(|matcher| matcher.matched(&date))
                        };
                        (next, dims(next.succ_opt().unwrap()), dims(next))
                    });
                assert_eq!(next_monday.weekday(), chrono::Weekday::Mon);
                assert!(next_monday >= chrono::Local::now().date_naive());
                assert!(dims_tuesday && !dims_monday);
            }
        }
        type_into(&editor.read_with(cx, |editor, _| editor.time.clone()), "7:30", cx);
        cx.update(|window, cx| editor.update(cx, |editor, cx| editor.save(window, cx)));
        draw(cx);
        let saved = state.read_with(cx, |app, _| app.task(compare.id).unwrap().clone());
        assert_eq!(
            saved.schedule,
            Schedule::Daily {
                at: NaiveTime::from_hms_opt(7, 30, 0).unwrap(),
                weekdays_only: false
            }
        );
        assert!(saved.approval.is_none(), "a comparison writes nothing to approve");
        assert!(!cx.update(|window, cx| window.has_active_dialog(cx)), "saving closes it");

        // A time that isn't one doesn't save.
        let editor = cx
            .update(|window, cx| open_schedule_dialog(state.clone(), compare.id, window, cx))
            .unwrap();
        type_into(&editor.read_with(cx, |editor, _| editor.time.clone()), "25:00", cx);
        draw(cx);
        cx.update(|window, cx| editor.update(cx, |editor, cx| editor.save(window, cx)));
        assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
        cx.update(|window, cx| window.close_dialog(cx));

        // A Mirror into Production: refused until allowed, then asked about before saving.
        let editor = cx
            .update(|window, cx| open_schedule_dialog(state.clone(), mirror.id, window, cx))
            .unwrap();
        editor.update(cx, |editor, _| editor.repeat = Repeat::Every);
        type_into(&editor.read_with(cx, |editor, _| editor.every.clone()), "2", cx);
        draw(cx);
        // Every… and the three safety numbers: each keeps the same room for its value, however
        // long the text beside it.
        let rooms = number_rooms(cx);
        assert_eq!(rooms.len(), 4, "{rooms:?}");
        assert!(rooms.iter().all(|room| *room >= px(60.0) && *room == rooms[0]), "{rooms:?}");
        cx.update(|window, cx| editor.update(cx, |editor, cx| editor.save(window, cx)));
        assert!(state.read_with(cx, |app, _| app.task(mirror.id).unwrap().schedule.is_manual()));

        editor.update(cx, |editor, _| editor.protected_writes = true);
        cx.update(|window, cx| editor.update(cx, |editor, cx| editor.save(window, cx)));
        draw(cx);
        assert!(
            state.read_with(cx, |app, _| app.task(mirror.id).unwrap().schedule.is_manual()),
            "nothing is saved before the question is answered"
        );
        let snapshots = cx.update(|window, _| gpui_kit::base::test_support::snapshots(window));
        let answer = snapshots
            .iter()
            .find(|node| node.path().last() == Some(&gpui_kit::ElementId::from("confirm-action")))
            .expect("the question's answer button");
        assert_eq!(answer.label(), Some("Allow scheduled writes"));
        cx.simulate_click(answer.bounds().center(), Default::default());
        draw(cx);
        let saved = state.read_with(cx, |app, _| app.task(mirror.id).unwrap().clone());
        assert_eq!(saved.schedule, Schedule::Every { minutes: 120 });
        assert!(saved.approval.as_ref().is_some_and(|approval| approval.protected_writes));
        assert!(state.read_with(cx, |app, _| app.task_approval_problem(&saved).is_none()));
        assert!(!cx.update(|window, cx| window.has_active_dialog(cx)), "both dialogs closed");
    }
}
