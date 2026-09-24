//! The Tasks tab: saved Transfer and Compare setups, run again with one click, with the result
//! of every run.

use gpui_kit::component::button::{ButtonGroup, ButtonVariants as _};
use gpui_kit::component::description_list::{DescriptionItem, DescriptionList};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use uuid::Uuid;

use crate::components::{Button, open_confirm_dialog};
use crate::helpers::format_number;
use crate::keyboard::{EditSelectedTask, TaskNext, TaskPrevious};
use crate::state::app_state::Fix;
use crate::state::compare::CompareScope;
use crate::state::{AppCommands, AppState, TabKey, TransferMode};
use crate::tasks::model::{LogLevel, Run, RunStatus, RunTrigger, Task, TaskKind};
use crate::theme::{islands, spacing};
use crate::views::compare::app_icon;
use crate::views::task_schedule::{open_schedule_dialog, run_time, tabular};

pub struct TasksView {
    state: Entity<AppState>,
    selected: Option<Uuid>,
    /// The run whose details replace the history list.
    selected_run: Option<Uuid>,
    /// The list shows only the tasks that need attention.
    attention_only: bool,
    focus: FocusHandle,
    _subscription: Subscription,
}

impl TasksView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        // A notification's Open task asks for a task through the app state.
        let subscription = cx.observe(&state, |view, state, cx| {
            if let Some(id) = state.update(cx, |app, _| app.tasks.focus.take()) {
                view.selected = Some(id);
                view.selected_run = None;
                view.attention_only = false;
            }
            cx.notify()
        });
        let selected = state.update(cx, |app, _| app.tasks.focus.take());
        Self {
            state,
            selected,
            selected_run: None,
            attention_only: false,
            focus: cx.focus_handle(),
            _subscription: subscription,
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// Tasks by name, the order the list shows them in.
    fn sorted(app: &AppState) -> Vec<Task> {
        let mut tasks = app.tasks.tasks.clone();
        tasks.sort_by_key(|task| task.name.to_lowercase());
        tasks
    }

    /// The selected task, or the first one when nothing (or a deleted task) is selected.
    fn current(&self, tasks: &[Task]) -> Option<Uuid> {
        self.selected
            .filter(|id| tasks.iter().any(|task| task.id == *id))
            .or_else(|| tasks.first().map(|task| task.id))
    }

    fn select(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.selected != Some(id) {
            self.selected = Some(id);
            self.selected_run = None;
            cx.notify();
        }
    }

    /// The tasks the list shows: all of them, or those that need attention.
    fn shown(&self, app: &AppState) -> Vec<Task> {
        let tasks = Self::sorted(app);
        if !self.attention_only || app.tasks_needing_attention() == 0 {
            return tasks;
        }
        tasks.into_iter().filter(|task| app.task_attention(task).is_some()).collect()
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let tasks = self.shown(self.state.read(cx));
        let Some(current) = self.current(&tasks) else {
            return;
        };
        let index = tasks.iter().position(|task| task.id == current).unwrap_or(0) as isize;
        let next = (index + delta).clamp(0, tasks.len() as isize - 1) as usize;
        self.select(tasks[next].id, cx);
    }
}

fn status_icon(status: RunStatus, cx: &App) -> AnyElement {
    let theme = cx.theme();
    match status {
        RunStatus::Running => Spinner::new().xsmall().into_any_element(),
        RunStatus::Succeeded => {
            Icon::new(IconName::CircleCheck).xsmall().text_color(theme.success).into_any_element()
        }
        RunStatus::Failed => {
            Icon::new(IconName::TriangleAlert).xsmall().text_color(theme.danger).into_any_element()
        }
        RunStatus::PartlyDone => {
            app_icon("contrast").xsmall().text_color(theme.warning).into_any_element()
        }
        RunStatus::Cancelled => {
            app_icon("ban").xsmall().text_color(theme.muted_foreground).into_any_element()
        }
        RunStatus::Interrupted => {
            app_icon("circle-stop").xsmall().text_color(theme.warning).into_any_element()
        }
        RunStatus::Skipped => {
            app_icon("skip-forward").xsmall().text_color(theme.muted_foreground).into_any_element()
        }
    }
}

/// Local date and time, e.g. "Sep 23, 14:02".
fn when(at: chrono::DateTime<chrono::Utc>) -> String {
    at.with_timezone(&chrono::Local).format("%b %-d, %H:%M").to_string()
}

fn duration(run: &Run) -> Option<String> {
    let seconds = (run.finished_at? - run.started_at).num_seconds().max(0);
    Some(if seconds < 60 {
        format!("{seconds} s")
    } else if seconds < 3_600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3_600, seconds % 3_600 / 60)
    })
}

fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{} {}", format_number(count), if count == 1 { one } else { many })
}

/// What a run did, in a few words: "12,400 documents", "3 different · 1 left only", "Nothing to
/// write".
fn run_summary(kind: TaskKind, run: &Run) -> String {
    if let Some(error) = &run.error
        && run.collections.is_empty()
    {
        return error.lines().next().unwrap_or_default().to_string();
    }
    match run.trigger {
        RunTrigger::Preview => {
            if !run.stops.is_empty() {
                return "Preview: the safety limit would stop it".into();
            }
            return format!("Preview: would {}", planned_summary(run));
        }
        RunTrigger::Undo => {
            let restored = run.writes().written as u64;
            return format!("Undo: {} put back", plural(restored, "document", "documents"));
        }
        RunTrigger::Manual | RunTrigger::Schedule | RunTrigger::CatchUp => {}
    }
    match kind {
        TaskKind::Export | TaskKind::Import | TaskKind::Copy => {
            plural(run.documents(), "document", "documents")
        }
        TaskKind::Compare => {
            let counts = run.differences();
            let parts: Vec<String> = [
                (counts.different, "different"),
                (counts.only_left, "left only"),
                (counts.only_right, "right only"),
                (counts.minor, "minor"),
            ]
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, label)| format!("{} {label}", format_number(count)))
            .collect();
            if parts.is_empty() { "No differences".into() } else { parts.join(" · ") }
        }
        TaskKind::Sync => {
            let writes = run.writes();
            let parts: Vec<String> = [
                (writes.inserted, "inserted"),
                (writes.replaced, "replaced"),
                (writes.deleted, "deleted"),
            ]
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, label)| format!("{} {label}", format_number(count as u64)))
            .collect();
            if parts.is_empty() { "Nothing to write".into() } else { parts.join(" · ") }
        }
    }
}

/// "insert 20, replace 10 and delete 3", from what a run worked out before writing.
fn planned_summary(run: &Run) -> String {
    let [inserts, replaces, deletes] =
        run.collections.iter().filter_map(|c| c.planned).fold([0; 3], |total, planned| {
            [total[0] + planned[0], total[1] + planned[1], total[2] + planned[2]]
        });
    format!(
        "insert {}, replace {} and delete {}",
        format_number(inserts),
        format_number(replaces),
        format_number(deletes)
    )
}

/// What one collection's line in the run details says.
fn collection_summary(kind: TaskKind, run: &crate::tasks::model::CollectionRun) -> String {
    if let Some(error) = &run.error {
        return error.lines().next().unwrap_or_default().to_string();
    }
    if let Some(note) = &run.note {
        return note.clone();
    }
    if run.writes.is_none()
        && run.documents == 0
        && let Some([inserts, replaces, deletes]) = run.planned
    {
        return format!(
            "Would insert {}, replace {} and delete {}",
            format_number(inserts),
            format_number(replaces),
            format_number(deletes)
        );
    }
    let single = Run {
        collections: vec![run.clone()],
        ..Run::start(Uuid::nil(), crate::tasks::model::RunTrigger::Manual)
    };
    run_summary(kind, &single)
}

/// The schedule in a few words, as the list shows it: "Daily at 02:00", "Manual" or "Paused".
fn schedule_summary(task: &Task) -> String {
    if task.paused && !task.schedule.is_manual() { "Paused".into() } else { task.schedule.label() }
}

/// When the task's schedule runs it next.
fn next_run(task: &Task) -> Option<chrono::DateTime<chrono::Local>> {
    let now = chrono::Local::now();
    let from = task.schedule_from.map(|from| from.with_timezone(&chrono::Local));
    task.schedule.next_after(&from.filter(|from| *from > now).unwrap_or(now))
}

/// Records the connections as they are now for the task's scheduled runs, after asking when it
/// writes to a Production or protected connection.
fn approve_again(state: Entity<AppState>, task: &Task, window: &mut Window, cx: &mut App) {
    let id = task.id;
    let target = state.read(cx).task_protected_target(task);
    let approve = move |protected_writes: bool, cx: &mut App| {
        state.update(cx, |app, cx| {
            if let Err(error) = app.approve_task(id, protected_writes) {
                app.set_status_message(Some(crate::state::StatusMessage::error(error)));
            }
            cx.notify();
        })
    };
    match target {
        Some(target) => open_confirm_dialog(
            window,
            cx,
            format!("Allow scheduled writes to {target}?"),
            format!(
                "{target} is a Production or protected connection. Each scheduled run of “{}” \
                 writes to it without asking. The safety limit still stops a run that would delete \
                 or replace too much.",
                task.name
            ),
            "Allow scheduled writes",
            true,
            move |_, cx| approve(true, cx),
        ),
        None => approve(false, cx),
    }
}

fn last_result(app: &AppState, task: &Task) -> String {
    match app.task_runs(task.id).first() {
        None => "Not run yet".into(),
        Some(run) if run.status == RunStatus::Running => "Running…".into(),
        Some(run) => format!(
            "{} {}",
            run.status.label(),
            crate::bson::relative_age(
                run.started_at.timestamp_millis(),
                chrono::Utc::now().timestamp_millis()
            )
        ),
    }
}

/// A "New task" button whose menu opens each tool; Save as task there adds the task.
fn new_task_button(state: Entity<AppState>, primary: bool) -> impl IntoElement {
    let button = Button::new("new-task").icon(Icon::new(IconName::Plus).xsmall()).label("New task");
    let button = if primary { button.primary() } else { button.ghost() };
    button.small().dropdown_menu(move |menu, _, _| {
        let open_transfer = |mode: TransferMode| {
            let state = state.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                state.update(cx, |app, cx| app.open_transfer_tab_for_mode(mode, cx));
            }
        };
        let open_compare = |scope: CompareScope| {
            let state = state.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                state.update(cx, |app, cx| {
                    app.open_scoped_compare_tab(scope, None, cx);
                });
            }
        };
        menu.item(
            PopupMenuItem::new("Export…")
                .icon(app_icon("upload"))
                .on_click(open_transfer(TransferMode::Export)),
        )
        .item(
            PopupMenuItem::new("Import…")
                .icon(app_icon("download"))
                .on_click(open_transfer(TransferMode::Import)),
        )
        .item(
            PopupMenuItem::new("Copy…")
                .icon(Icon::new(IconName::Copy))
                .on_click(open_transfer(TransferMode::Copy)),
        )
        .item(
            PopupMenuItem::new("Compare collections…")
                .icon(app_icon("git-compare-arrows"))
                .on_click(open_compare(CompareScope::Collections)),
        )
        .item(
            PopupMenuItem::new("Compare or sync databases…")
                .icon(Icon::new(IconName::LayoutDashboard))
                .on_click(open_compare(CompareScope::Databases)),
        )
    })
}

/// Save controls for a Transfer or Compare tab. A tab opened from a task saves back to it;
/// any other tab saves a new task.
pub fn save_task_controls(state: Entity<AppState>, tab: TabKey, cx: &App) -> AnyElement {
    let app = state.read(cx);
    let linked = app.tab_task_id(&tab).filter(|id| app.task(*id).is_some());
    let Some(task_id) = linked else {
        return Button::new("save-as-task")
            .icon(app_icon("bookmark-plus").xsmall())
            .label("Save as task…")
            .small()
            .ghost()
            .tooltip("Save these settings as a task you can run again from Tasks")
            .on_click(move |_, window, cx| {
                crate::app::dialogs::open_save_task_dialog(state.clone(), tab.clone(), window, cx)
            })
            .into_any_element();
    };
    let name = app.task(task_id).map(|task| task.name.clone()).unwrap_or_default();
    div()
        .flex()
        .items_center()
        .child(
            Button::new("save-task")
                .icon(app_icon("save").xsmall())
                .label("Save task")
                .small()
                .ghost()
                .tooltip(format!("Save these settings into “{name}”"))
                .on_click({
                    let (state, tab) = (state.clone(), tab.clone());
                    move |_, _, cx| AppCommands::save_linked_task(&state, &tab, cx)
                }),
        )
        .child(
            Button::new("save-task-more")
                .icon(Icon::new(IconName::Ellipsis).xsmall())
                .small()
                .ghost()
                .tooltip("More save options")
                .dropdown_menu(move |menu, _, _| {
                    let (state, tab) = (state.clone(), tab.clone());
                    menu.item(
                        PopupMenuItem::new("Save as new task…")
                            .icon(app_icon("bookmark-plus"))
                            .on_click(move |_, window, cx| {
                                crate::app::dialogs::open_save_task_dialog(
                                    state.clone(),
                                    tab.clone(),
                                    window,
                                    cx,
                                )
                            }),
                    )
                }),
        )
        .into_any_element()
}

impl TasksView {
    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(spacing::sm())
            .p(spacing::lg())
            .text_center()
            .child(app_icon("list-checks").size(px(28.0)).text_color(cx.theme().muted_foreground))
            .child(div().text_base().font_weight(FontWeight::MEDIUM).child("No tasks yet"))
            .child(div().max_w(px(420.0)).text_sm().text_color(cx.theme().muted_foreground).child(
                "A task is a Transfer or Compare you can run again with one click. Set \
                         one up, then choose Save as task.",
            ))
            .child(div().pt(spacing::xs()).child(new_task_button(self.state.clone(), true)))
            .into_any_element()
    }

    fn render_list(
        &self,
        tasks: &[Task],
        current: Option<Uuid>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.state.read(cx);
        let muted = cx.theme().muted_foreground;
        let rows = tasks.iter().map(|task| {
            let id = task.id;
            let selected = current == Some(id);
            let running = app.task_is_running(id);
            let status = if running {
                Some(RunStatus::Running)
            } else {
                app.task_runs(id).first().map(|run| run.status)
            };
            let attention = !running && app.task_attention(task).is_some();
            let icon = if attention {
                Some(
                    Icon::new(IconName::TriangleAlert)
                        .xsmall()
                        .text_color(cx.theme().danger)
                        .into_any_element(),
                )
            } else {
                status.map(|status| status_icon(status, cx))
            };
            let result = last_result(app, task);
            div()
                .id(ElementId::Name(format!("task-row-{id}").into()))
                .debug_selector(move || format!("task-row-{}", id))
                .flex()
                .gap(spacing::sm())
                .px(spacing::sm())
                .py(spacing::xs())
                .rounded(crate::theme::borders::radius_md())
                .cursor_pointer()
                .when(selected, |row| row.bg(cx.theme().list_active))
                .when(!selected, |row| row.hover(|row| row.bg(cx.theme().list_hover)))
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.select(id, cx);
                    view.focus.focus(window, cx);
                }))
                .child(div().w(px(14.0)).pt(px(3.0)).flex_shrink_0().children(icon))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(task.name.clone()),
                        )
                        .child(div().text_xs().text_color(muted).truncate().child(format!(
                            "{} · {}",
                            task.spec.kind().label(),
                            schedule_summary(task)
                        )))
                        .child(div().text_xs().text_color(muted).truncate().child(result)),
                )
        });
        div()
            .id("task-list")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .p(spacing::sm())
            .overflow_y_scrollbar()
            .children(rows)
            .into_any_element()
    }

    fn render_detail(&self, task: &Task, cx: &mut Context<Self>) -> AnyElement {
        let app = self.state.read(cx);
        let id = task.id;
        let running = app.task_is_running(id);
        let muted = cx.theme().muted_foreground;
        let state = self.state.clone();

        let run_button = if running {
            Button::new("task-cancel")
                .icon(app_icon("circle-stop").xsmall())
                .label("Cancel")
                .small()
                .on_click({
                    let state = state.clone();
                    move |_, _, cx| AppCommands::cancel_task_run(&state, id, cx)
                })
        } else {
            Button::new("task-run")
                .primary()
                .icon(app_icon("play").xsmall())
                .label("Run now")
                .small()
                .on_click({
                    let state = state.clone();
                    move |_, window, cx| AppCommands::run_task(state.clone(), id, window, cx)
                })
        };
        let previewable = matches!(
            task.spec.kind(),
            TaskKind::Sync | TaskKind::Copy | TaskKind::Import | TaskKind::Export
        );
        let scheduled = !task.schedule.is_manual();
        let paused = task.paused;
        let actions = div()
            .flex()
            .flex_wrap()
            .gap(spacing::xs())
            .child(run_button)
            .when(previewable, |actions| {
                actions.child(
                    Button::new("task-preview")
                        .icon(Icon::new(IconName::Eye).xsmall())
                        .label("Preview")
                        .small()
                        .ghost()
                        .tooltip("Work out what a run would change, without writing")
                        .disabled(running)
                        .on_click({
                            let state = state.clone();
                            move |_, window, cx| {
                                AppCommands::preview_task(state.clone(), id, window, cx)
                            }
                        }),
                )
            })
            .child(
                Button::new("task-edit")
                    .icon(app_icon("pencil").xsmall())
                    .label("Edit")
                    .small()
                    .ghost()
                    .on_click({
                        let state = state.clone();
                        move |_, _, cx| state.update(cx, |app, cx| app.edit_task(id, cx))
                    }),
            )
            .child(
                Button::new("task-schedule")
                    .icon(app_icon("calendar-clock").xsmall())
                    .label("Schedule…")
                    .small()
                    .ghost()
                    .tooltip("When it runs by itself, and its safety limit")
                    .on_click({
                        let state = state.clone();
                        move |_, window, cx| {
                            open_schedule_dialog(state.clone(), id, window, cx);
                        }
                    }),
            )
            .when(scheduled, |actions| {
                let state = state.clone();
                actions.child(
                    Button::new("task-pause")
                        .icon(app_icon(if paused { "play" } else { "pause" }).xsmall())
                        .label(if paused { "Resume" } else { "Pause" })
                        .small()
                        .ghost()
                        .on_click(move |_, _, cx| {
                            state.update(cx, |app, cx| {
                                if let Err(error) = app.set_task_paused(id, !paused) {
                                    app.set_status_message(Some(
                                        crate::state::StatusMessage::error(error),
                                    ));
                                }
                                cx.notify();
                            })
                        }),
                )
            })
            .child(
                Button::new("task-delete")
                    .icon(app_icon("trash").xsmall())
                    .label("Delete")
                    .small()
                    .ghost()
                    .disabled(running)
                    .on_click({
                        let state = state.clone();
                        let name = task.name.clone();
                        move |_, window, cx| {
                            let state = state.clone();
                            open_confirm_dialog(
                                window,
                                cx,
                                format!("Delete “{name}”?"),
                                "Its run history is deleted too. The data it worked on isn't touched.",
                                "Delete task",
                                true,
                                move |_, cx| AppCommands::delete_task(&state, id, cx),
                            );
                        }
                    }),
            );

        let selected_run = self.selected_run.and_then(|run| app.task_run(id, run)).cloned();
        let body = match selected_run {
            Some(run) => self.render_run(task, &run, cx),
            None => self.render_history(task, cx),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .px(spacing::lg())
                    .pt(spacing::md())
                    .pb(spacing::sm())
                    .child(
                        div().text_lg().font_weight(FontWeight::SEMIBOLD).child(task.name.clone()),
                    )
                    .child(div().text_sm().text_color(muted).child(format!(
                        "{} · {}",
                        task.spec.kind().label(),
                        task.spec.subject()
                    )))
                    .child(div().pt(spacing::xs()).child(actions))
                    .child(self.render_schedule(task, cx)),
            )
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// The Schedule and Safety rows, and why scheduled runs can't write when they can't.
    fn render_schedule(&self, task: &Task, cx: &mut Context<Self>) -> AnyElement {
        let app = self.state.read(cx);
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let id = task.id;
        let scheduled = !task.schedule.is_manual();
        let lines = |first: AnyElement, second: String| {
            div()
                .flex()
                .flex_col()
                .text_sm()
                .child(first)
                .child(div().text_color(muted).font_features(tabular()).child(second))
                .into_any_element()
        };

        let queued = app.tasks.queue.iter().any(|(queued, ..)| *queued == id);
        let value = if !scheduled {
            lines("Manual".into_any_element(), "Runs when you choose Run now.".into())
        } else if task.paused {
            let first = div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .child(app_icon("pause").xsmall().text_color(muted))
                .child("Paused")
                .child(div().text_color(muted).child(format!("· {}", task.schedule.label())));
            lines(first.into_any_element(), "Resume to run it on its schedule again.".into())
        } else {
            let next = if queued {
                "Due now, waiting for another task to finish.".to_string()
            } else {
                next_run(task)
                    .map_or("No run coming up.".into(), |at| format!("Next: {}", run_time(at)))
            };
            let closed =
                if task.run_when_closed { " · also while OpenMango is closed" } else { "" };
            let first = format!("{}, local time{closed}", task.schedule.label());
            lines(first.into_any_element(), next)
        };
        // The same label and value list the reference peek uses.
        let mut facts = DescriptionList::new()
            .columns(1)
            .label_width(px(110.0))
            .child(DescriptionItem::new("Schedule").value(value));

        if task.spec.write_connection().is_some() {
            let limit = &task.safety;
            let first = format!(
                "More than {}% of a collection, or {} times the usual, deleted or replaced",
                limit.percent, limit.jump
            );
            let mut second = format!(
                "Run now asks first; a scheduled run stops before writing. Changes under {} \
                 documents never count.",
                format_number(limit.floor)
            );
            if let Some(target) = app.task_protected_target(task)
                && task.approval.as_ref().is_some_and(|approval| approval.protected_writes)
            {
                second.push_str(&format!(" Scheduled runs may write to {target}."));
            }
            facts = facts.child(
                DescriptionItem::new("Safety").value(lines(first.into_any_element(), second)),
            );
        }
        let mut section = div().flex().flex_col().gap(spacing::sm()).pt(spacing::md()).child(facts);

        let running = app.task_is_running(id);
        let attention = app.task_attention(task).filter(|attention| {
            !running || !matches!(attention.fix, Fix::ShowRun(_) | Fix::RunAnyway)
        });
        if let Some(attention) = attention {
            let state = self.state.clone();
            let task = task.clone();
            let fix = match attention.fix {
                Fix::Approve => Button::new("task-fix")
                    .label("Approve again")
                    .on_click(move |_, window, cx| approve_again(state.clone(), &task, window, cx)),
                Fix::Edit => Button::new("task-fix")
                    .label("Edit")
                    .on_click(move |_, _, cx| state.update(cx, |app, cx| app.edit_task(id, cx))),
                Fix::Resume => Button::new("task-fix").label("Resume").on_click(move |_, _, cx| {
                    state.update(cx, |app, cx| {
                        if let Err(error) = app.set_task_paused(id, false) {
                            app.set_status_message(Some(crate::state::StatusMessage::error(error)));
                        }
                        cx.notify();
                    })
                }),
                Fix::RunAnyway => {
                    Button::new("task-fix").label("Run anyway…").on_click(move |_, window, cx| {
                        AppCommands::run_task(state.clone(), id, window, cx)
                    })
                }
                Fix::ShowRun(run_id) => Button::new("task-fix").label("Show run").on_click(
                    cx.listener(move |view, _, _, cx| {
                        view.selected_run = Some(run_id);
                        cx.notify();
                    }),
                ),
                Fix::RunnerSettings => Button::new("task-fix")
                    .label(crate::helpers::background_runner::OPEN_SETTINGS)
                    .on_click(move |_, _, cx| {
                        crate::helpers::background_runner::open_settings();
                        state.update(cx, |state, cx| state.refresh_background_runner(cx));
                    }),
            };
            section = section.child(
                div()
                    .id("task-attention")
                    .debug_selector(|| "task-attention".into())
                    .flex()
                    .items_start()
                    .gap(spacing::sm())
                    .p(spacing::sm())
                    .rounded(crate::theme::borders::radius_md())
                    .bg(theme.warning.opacity(0.1))
                    .child(div().pt(px(3.0)).child(
                        Icon::new(IconName::TriangleAlert).xsmall().text_color(theme.warning),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .text_sm()
                            .child(div().font_weight(FontWeight::MEDIUM).child(attention.reason))
                            .when(!attention.detail.is_empty(), |text| {
                                text.child(div().text_color(muted).child(attention.detail))
                            }),
                    )
                    .child(fix.small()),
            );
        }
        section.into_any_element()
    }

    fn render_history(&self, task: &Task, cx: &mut Context<Self>) -> AnyElement {
        let app = self.state.read(cx);
        let runs = app.task_runs(task.id);
        let muted = cx.theme().muted_foreground;
        let kind = task.spec.kind();
        let mut list = div()
            .id("task-history")
            .size_full()
            .flex()
            .flex_col()
            .px(spacing::lg())
            .pb(spacing::md())
            .overflow_y_scrollbar()
            .child(
                div()
                    .pt(spacing::sm())
                    .pb(spacing::xs())
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(muted)
                    .child("History"),
            );
        if let Some(note) = &app.tasks.store_note {
            list =
                list.child(div().pb(spacing::xs()).text_xs().text_color(muted).child(note.clone()));
        }
        if runs.is_empty() {
            return list
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("Not run yet. Run now runs it with the settings above."),
                )
                .into_any_element();
        }
        list.children(runs.iter().map(|run| {
            let run_id = run.id;
            div()
                .id(ElementId::Name(format!("task-run-{run_id}").into()))
                .debug_selector(move || format!("task-run-{run_id}"))
                .flex()
                .items_center()
                .gap(spacing::sm())
                .px(spacing::sm())
                .py(spacing::xs())
                .rounded(crate::theme::borders::radius_md())
                .cursor_pointer()
                .hover(|row| row.bg(cx.theme().list_hover))
                .font_features(tabular())
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.selected_run = Some(run_id);
                    cx.notify();
                }))
                .child(div().w(px(14.0)).flex_shrink_0().child(status_icon(run.status, cx)))
                .child(div().w(px(110.0)).flex_shrink_0().text_sm().child(when(run.started_at)))
                .child(div().w(px(90.0)).flex_shrink_0().text_sm().child(run.status.label()))
                .child(
                    div()
                        .w(px(64.0))
                        .flex_shrink_0()
                        .text_sm()
                        .text_color(muted)
                        .children(duration(run)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(muted)
                        .truncate()
                        .child(run_summary(kind, run)),
                )
        }))
        .into_any_element()
    }

    fn render_run(&self, task: &Task, run: &Run, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let kind = task.spec.kind();
        let mut facts =
            vec![format!("Started {}", when(run.started_at)), run.trigger.label().to_string()];
        facts.extend(duration(run));
        let mut body = div()
            .id("task-run-detail")
            .debug_selector(|| "task-run-detail".into())
            .size_full()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .px(spacing::lg())
            .pb(spacing::md())
            .overflow_y_scrollbar()
            .child(
                div().pt(spacing::sm()).child(
                    Button::new("task-run-back")
                        .icon(Icon::new(IconName::ChevronLeft).xsmall())
                        .label("History")
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.selected_run = None;
                            cx.notify();
                        })),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(status_icon(run.status, cx))
                    .child(
                        div().text_base().font_weight(FontWeight::MEDIUM).child(run.status.label()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(format!("· {}", run_summary(kind, run))),
                    ),
            )
            .child(div().text_xs().text_color(muted).child(facts.join(" · ")));

        if !run.stops.is_empty() {
            let heading = if run.trigger == RunTrigger::Preview {
                "The safety limit would stop this run"
            } else {
                "The safety limit stopped this run"
            };
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .p(spacing::sm())
                    .rounded(crate::theme::borders::radius_md())
                    .bg(cx.theme().warning.opacity(0.1))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(
                                Icon::new(IconName::TriangleAlert)
                                    .xsmall()
                                    .text_color(cx.theme().warning),
                            )
                            .child(heading),
                    )
                    .children(run.stops.iter().map(|stop| div().text_sm().child(stop.clone())))
                    .when(run.trigger == RunTrigger::Preview, |block| {
                        block.child(div().text_xs().text_color(muted).child(
                            "Run now asks first, and its answer is Run anyway. A scheduled run \
                             stops here instead.",
                        ))
                    })
                    .when(
                        matches!(run.trigger, RunTrigger::Schedule | RunTrigger::CatchUp)
                            && !self.state.read(cx).task_is_running(task.id),
                        |block| {
                            let (state, task_id) = (self.state.clone(), task.id);
                            block.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(spacing::sm())
                                    .child(
                                        Button::new("task-run-anyway")
                                            .label("Run anyway…")
                                            .small()
                                            .on_click(move |_, window, cx| {
                                                AppCommands::run_task(
                                                    state.clone(),
                                                    task_id,
                                                    window,
                                                    cx,
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted)
                                            .child("Runs it once now, after asking."),
                                    ),
                            )
                        },
                    ),
            );
        }

        let undoable = {
            let app = self.state.read(cx);
            !app.task_is_running(task.id)
                && app.tasks.undo.get(&task.id).is_some_and(|undo| undo.run_id == run.id)
        };
        if undoable {
            let state = self.state.clone();
            let task_id = task.id;
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Button::new("task-undo-run")
                            .icon(app_icon("rotate-ccw").xsmall())
                            .label("Undo this run")
                            .small()
                            .on_click(move |_, window, cx| {
                                AppCommands::undo_task_run(state.clone(), task_id, window, cx)
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("Available until OpenMango closes or this task runs again."),
                    ),
            );
        }

        if let Some(error) = run.error.clone() {
            body = body.child(
                div()
                    .flex()
                    .items_start()
                    .gap(spacing::sm())
                    .p(spacing::sm())
                    .rounded(crate::theme::borders::radius_md())
                    .bg(cx.theme().danger.opacity(0.08))
                    .child(div().flex_1().min_w_0().text_sm().child(error.clone()))
                    .child(
                        Button::new("task-run-copy-error")
                            .icon(Icon::new(IconName::Copy).xsmall())
                            .label("Copy error")
                            .xsmall()
                            .ghost()
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(error.clone()))
                            }),
                    ),
            );
        }

        if !run.collections.is_empty() {
            body = body
                .child(
                    div()
                        .pt(spacing::xs())
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(muted)
                        .child("Collections"),
                )
                .children(run.collections.iter().map(|collection| {
                    let failed = collection.error.is_some();
                    div()
                        .flex()
                        .gap(spacing::sm())
                        .text_sm()
                        .child(div().w(px(14.0)).flex_shrink_0().pt(px(3.0)).when(failed, |cell| {
                            cell.child(
                                Icon::new(IconName::TriangleAlert)
                                    .xsmall()
                                    .text_color(cx.theme().danger),
                            )
                        }))
                        .child(
                            div()
                                .w(px(180.0))
                                .flex_shrink_0()
                                .truncate()
                                .child(collection.name.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .when(!failed, |cell| cell.text_color(muted))
                                .child(collection_summary(kind, collection)),
                        )
                }));
        }

        body = body
            .child(
                div()
                    .pt(spacing::xs())
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(muted)
                    .child("Log"),
            )
            .children(run.log.iter().map(|line| {
                let level = match line.level {
                    LogLevel::Info => None,
                    LogLevel::Warning => Some(("Warning", cx.theme().warning)),
                    LogLevel::Error => Some(("Error", cx.theme().danger)),
                };
                div()
                    .flex()
                    .gap(spacing::sm())
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(div().flex_shrink_0().text_color(muted).child(
                        line.at.with_timezone(&chrono::Local).format("%H:%M:%S").to_string(),
                    ))
                    .children(
                        level.map(|(label, color)| {
                            div().flex_shrink_0().text_color(color).child(label)
                        }),
                    )
                    .child(div().flex_1().min_w_0().child(line.message.clone()))
            }))
            .when(run.log_dropped > 0, |body| {
                body.child(
                    div().text_xs().text_color(muted).child(format!(
                        "{} more lines weren't kept.",
                        format_number(run.log_dropped)
                    )),
                )
            });
        body.into_any_element()
    }
}

impl Render for TasksView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (tasks, none, attention, appearance, load_error) = {
            let app = self.state.read(cx);
            (
                self.shown(app),
                app.tasks.tasks.is_empty(),
                app.tasks_needing_attention(),
                app.settings.appearance.clone(),
                app.tasks.load_error.clone(),
            )
        };
        if attention == 0 {
            // Nothing needs attention: the filter goes away, and doesn't come back on by itself.
            self.attention_only = false;
        }
        let current = self.current(&tasks);
        let header = div()
            .flex()
            .items_center()
            .gap(spacing::md())
            .px(spacing::lg())
            .py(spacing::md())
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Tasks"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Saved Transfer and Compare setups, run again with one click."),
                    ),
            )
            .when(!none, |header| header.child(new_task_button(self.state.clone(), false)));

        let body = if none {
            self.render_empty(cx)
        } else {
            let task = tasks.iter().find(|task| Some(task.id) == current).cloned();
            let only = self.attention_only && attention > 0;
            let filter = (attention > 0).then(|| {
                let choice = |id: &'static str, label: String, selected: bool| {
                    Button::new(id).label(label).selected(selected).small()
                };
                div().px(spacing::sm()).pt(spacing::sm()).child(
                    ButtonGroup::new("task-filter")
                        .compact()
                        .child(choice("task-filter-all", "All".into(), !only))
                        .child(choice(
                            "task-filter-attention",
                            format!("Needs attention ({attention})"),
                            only,
                        ))
                        .on_click(cx.listener(|view, selection: &Vec<usize>, _, cx| {
                            view.attention_only = selection.first() == Some(&1);
                            cx.notify();
                        })),
                )
            });
            let list = div()
                .size_full()
                .flex()
                .flex_col()
                .children(filter)
                .child(div().flex_1().min_h_0().child(self.render_list(&tasks, current, cx)));
            let detail = task
                .map(|task| self.render_detail(&task, cx))
                .unwrap_or_else(|| div().into_any_element());
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(
                    h_resizable("tasks-split")
                        .child(
                            resizable_panel()
                                .size(px(300.0))
                                .size_range(px(200.0)..px(520.0))
                                .child(list),
                        )
                        .child(resizable_panel().size_range(px(320.0)..Pixels::MAX).child(detail)),
                )
                .into_any_element()
        };

        div()
            .id("tasks-view")
            .debug_selector(|| "tasks-view".into())
            .key_context("Tasks")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .bg(islands::content_bg(&appearance, cx))
            .on_action(cx.listener(|view, _: &TaskNext, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &TaskPrevious, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &EditSelectedTask, _, cx| {
                let tasks = view.shown(view.state.read(cx));
                if let Some(id) = view.current(&tasks) {
                    view.state.update(cx, |app, cx| app.edit_task(id, cx));
                }
            }))
            .child(header)
            .children(load_error.map(|error| {
                div()
                    .px(spacing::lg())
                    .py(spacing::sm())
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error)
            }))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::component::Root;
    use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};
    use uuid::Uuid;

    use crate::components::ContentArea;
    use crate::connection::ops::compare::Side;
    use crate::connection::ops::compare_database::SyncMode;
    use crate::state::compare::{CompareConfig, CompareScope};
    use crate::state::{AppState, ConfigManager, TabKey};
    use crate::tasks::model::{CollectionRun, LogLevel, Run, RunTrigger, Task, TaskSpec};

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }

    fn setup(cx: &mut TestAppContext) -> (tempfile::TempDir, gpui_kit::Entity<AppState>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply_design_tokens(cx);
            crate::keyboard::bind_keymap(cx, &Default::default());
        });
        let directory = tempfile::tempdir().unwrap();
        let state = cx.new(|_| {
            AppState::with_config(
                Arc::new(crate::connection::ConnectionManager::new()),
                ConfigManager::with_config_dir(directory.path().into()),
            )
        });
        (directory, state)
    }

    /// `debug_bounds` wants a `'static` selector.
    fn selector(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    fn compare_task(name: &str) -> Task {
        Task::new(name.into(), TaskSpec::Compare { config: CompareConfig::default() })
    }

    #[gpui_kit::test]
    fn tasks_tab_lists_tasks_and_opens_a_run(cx: &mut TestAppContext) {
        let (_directory, state) = setup(cx);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| ContentArea::new(state.clone(), cx));
            Root::new(view, window, cx).bordered(false)
        });
        cx.simulate_resize(size(px(1200.0), px(800.0)));
        state.update(cx, |app, cx| app.open_tasks_tab(cx));
        draw(cx);
        draw(cx);
        assert!(cx.debug_bounds("tasks-view").is_some(), "the Tasks tab shows");

        let first = compare_task("Alpha");
        let second = compare_task("Beta");
        let mut run = Run::start(second.id, RunTrigger::Manual);
        run.collection_mut("orders").error = Some("Connection refused".into());
        run.collection_mut("items");
        run.log(LogLevel::Error, "Connection refused");
        run.finish(false);
        let run_id = run.id;
        state.update(cx, |app, _| {
            app.upsert_task(second.clone()).unwrap();
            app.upsert_task(first.clone()).unwrap();
            app.record_task_run(run, false);
        });
        draw(cx);
        assert!(cx.debug_bounds(selector(format!("task-row-{}", first.id))).is_some());
        assert!(
            cx.debug_bounds(selector(format!("task-run-{run_id}"))).is_none(),
            "the first task by name is selected, and it has no runs"
        );

        let row = cx.debug_bounds(selector(format!("task-row-{}", second.id))).unwrap();
        cx.simulate_click(row.center(), Default::default());
        draw(cx);
        let history =
            cx.debug_bounds(selector(format!("task-run-{run_id}"))).expect("the run is listed");
        cx.simulate_click(history.center(), Default::default());
        draw(cx);
        assert!(cx.debug_bounds("task-run-detail").is_some(), "the run's details open");

        // Up moves the selection back to the first task, whose history has no runs.
        cx.simulate_keystrokes("up");
        draw(cx);
        assert!(cx.debug_bounds("task-run-detail").is_none());
        assert!(cx.debug_bounds(selector(format!("task-run-{run_id}"))).is_none());
    }

    /// In a narrow window the actions wrap instead of running out of view.
    #[gpui_kit::test]
    fn the_actions_wrap_in_a_narrow_window(cx: &mut TestAppContext) {
        let (_directory, state) = setup(cx);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| ContentArea::new(state.clone(), cx));
            Root::new(view, window, cx).bordered(false)
        });
        cx.simulate_resize(size(px(700.0), px(700.0)));
        let mut config = CompareConfig { scope: CompareScope::Databases, ..Default::default() };
        config.sides[0].database = "openmango_dbcompare_right_with_a_long_name".into();
        config.sides[1].database = "openmango_dbcompare_left_with_a_long_name".into();
        let mut task = Task::new(
            "Sync".into(),
            TaskSpec::Sync {
                config,
                target: Side::Right,
                mode: SyncMode::AddMissing,
                excluded: vec![],
            },
        );
        task.schedule = crate::tasks::schedule::Schedule::Every { minutes: 20 };
        state.update(cx, |app, cx| {
            app.upsert_task(task).unwrap();
            app.open_tasks_tab(cx);
        });
        draw(cx);
        draw(cx);
        let nodes = cx.update(|window, _| gpui_kit::base::test_support::snapshots(window));
        let window = cx.update(|window, _| window.bounds().size.width);
        for label in ["Run now", "Preview", "Edit", "Schedule…", "Pause", "Delete"] {
            let button = nodes
                .iter()
                .find(|node| node.label() == Some(label))
                .unwrap_or_else(|| panic!("{label} is on screen"));
            assert!(button.bounds().right() <= window, "{label}: {:?}", button.bounds());
        }
    }

    /// A scheduled task whose last run failed shows why, with its fix, and the list can show
    /// only such tasks. Open, from a notification, selects the task it names.
    #[gpui_kit::test]
    fn a_task_that_needs_attention_shows_why_and_can_be_filtered(cx: &mut TestAppContext) {
        let (_directory, state) = setup(cx);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| ContentArea::new(state.clone(), cx));
            Root::new(view, window, cx).bordered(false)
        });
        cx.simulate_resize(size(px(1200.0), px(800.0)));
        let daily = |name: &str| {
            let mut task = compare_task(name);
            task.schedule = crate::tasks::schedule::Schedule::Every { minutes: 60 };
            task
        };
        let (healthy, broken) = (daily("Alpha"), daily("Beta"));
        let mut run = Run::start(broken.id, RunTrigger::Schedule);
        run.error = Some("The filter isn't valid JSON".into());
        run.finish(false);
        let run_id = run.id;
        state.update(cx, |app, cx| {
            app.upsert_task(healthy.clone()).unwrap();
            app.upsert_task(broken.clone()).unwrap();
            app.record_task_run(run, true);
            app.open_tasks_tab(cx);
        });
        draw(cx);
        draw(cx);
        let click = |label: &str, cx: &mut VisualTestContext| {
            let nodes = cx.update(|window, _| gpui_kit::base::test_support::snapshots(window));
            let node = nodes
                .iter()
                .find(|node| node.label() == Some(label))
                .unwrap_or_else(|| panic!("{label} is on screen"));
            cx.simulate_click(node.bounds().center(), Default::default());
            draw(cx);
        };
        let row = |task: &Task, cx: &mut VisualTestContext| {
            cx.debug_bounds(selector(format!("task-row-{}", task.id))).is_some()
        };
        assert!(cx.debug_bounds("task-attention").is_none(), "Alpha, selected first, is fine");

        click("Needs attention (1)", cx);
        assert!(!row(&healthy, cx) && row(&broken, cx), "only Beta is listed");
        assert!(cx.debug_bounds("task-attention").is_some(), "Beta's details say why");
        click("Show run", cx);
        assert!(cx.debug_bounds("task-run-detail").is_some(), "the failed run opens");

        state.update(cx, |app, cx| app.open_task(healthy.id, cx));
        draw(cx);
        assert!(row(&healthy, cx), "Open shows every task again");
        assert!(cx.debug_bounds("task-attention").is_none(), "and selects Alpha");
        assert!(cx.debug_bounds(selector(format!("task-run-{run_id}"))).is_none());
    }

    #[gpui_kit::test]
    fn a_compare_tab_saves_a_sync_task_and_edit_returns_to_its_tab(cx: &mut TestAppContext) {
        use crate::connection::ops::compare_database::{
            CollectionKind, CollectionPair, SideCollection,
        };
        let (_directory, state) = setup(cx);
        let side = || {
            Some(SideCollection {
                kind: CollectionKind::Collection,
                estimated: None,
                bytes: None,
                indexes: None,
            })
        };
        let (compare_id, task_id) = state.update(cx, |app, cx| {
            let config = CompareConfig { scope: CompareScope::Databases, ..Default::default() };
            let id = app.open_compare_tab_with(config, cx);
            let tab = app.compare_tab_mut(id).unwrap();
            tab.pairs = vec![
                CollectionPair { name: "audit".into(), sides: [side(), side()] },
                CollectionPair { name: "orders".into(), sides: [side(), side()] },
            ];
            tab.sync.set_target(Side::Right);
            tab.sync.set_mode(SyncMode::Mirror);
            tab.sync.toggle_pair(0);

            let spec = app.compare_task_spec(id).unwrap();
            let TaskSpec::Sync { target, mode, excluded, .. } = &spec else {
                panic!("a database tab in sync mode saves a Sync task: {spec:?}");
            };
            assert_eq!(
                (*target, *mode, excluded.clone()),
                (Side::Right, SyncMode::Mirror, vec!["audit".to_string()])
            );
            let task = Task::new(spec.default_name(), spec);
            let task_id = task.id;
            app.upsert_task(task).unwrap();
            let key =
                TabKey::Compare(crate::state::compare::CompareTabKey { id, connection_id: None });
            app.link_tab_to_task(&key, task_id);
            (id, task_id)
        });

        // Edit shows the tab already open for the task instead of opening another.
        state.update(cx, |app, cx| {
            let tabs = app.open_tabs().len();
            app.select_tab(0, cx);
            app.edit_task(task_id, cx);
            assert_eq!(app.open_tabs().len(), tabs);
            assert_eq!(app.active_compare_tab_id(), Some(compare_id));
        });

        // The saved settings survive a restart.
        let saved = state.read_with(cx, |app, _| app.config.load_tasks().unwrap());
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, task_id);

        // A tab opened to edit the task puts its sync choices back once the listing arrives.
        state.update(cx, |app, cx| {
            while !app.open_tabs().is_empty() {
                app.close_tab(0, cx);
            }
            app.edit_task(task_id, cx);
            let id = app.active_compare_tab_id().unwrap();
            let tab = app.compare_tab_mut(id).unwrap();
            tab.begin();
            tab.receive_pairs(Ok(vec![
                CollectionPair { name: "orders".into(), sides: [side(), side()] },
                CollectionPair { name: "audit".into(), sides: [side(), side()] },
            ]));
            assert_eq!(tab.sync.target, Some(Side::Right));
            assert_eq!(tab.sync.mode, SyncMode::Mirror);
            assert_eq!(tab.sync_excluded_names(), vec!["audit".to_string()]);
        });
    }

    #[gpui_kit::test]
    fn an_unreadable_task_file_is_never_overwritten(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("tasks.json"), "{ not json").unwrap();
        let state = cx.new(|_| {
            AppState::with_config(
                Arc::new(crate::connection::ConnectionManager::new()),
                ConfigManager::with_config_dir(directory.path().into()),
            )
        });
        state.update(cx, |app, _| {
            assert!(app.tasks.load_error.is_some());
            assert!(app.upsert_task(compare_task("New")).is_err());
            assert!(app.remove_task(Uuid::new_v4()).is_err());
        });
        assert_eq!(
            std::fs::read_to_string(directory.path().join("tasks.json")).unwrap(),
            "{ not json"
        );
        let _ = CollectionRun::default();
    }
}
