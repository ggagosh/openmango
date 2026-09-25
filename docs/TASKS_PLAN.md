# Tasks and scheduling — plan

Status: decisions confirmed 2026-09-23. Everything planned is built, PRs 1 through 7; see Implementation status. Still to try on real machines: the background runner's system entries and the system notifications on each system.

A task is a saved Transfer or Compare setup. People run it with one click, give it a schedule,
and see the result of every run, including runs that happen while OpenMango is closed.

This replaces two lines in [features.md](features.md): "Task presets for transfer operations" and
"Scheduler for recurring import/export/copy". It also covers "Per-operation timeline/log for
long-running jobs" for task runs.

## Decisions

All twelve were confirmed on 2026-09-23. The alternative column records what was considered.

| # | Decision | Chosen | Alternative |
|---|---|---|---|
| 1 | Build order | Tasks you run yourself first, then the in-app scheduler, then the background runner that works while OpenMango is closed, one platform per PR | Background runner in the first release |
| 2 | Task kinds | Export, Import, Copy, Compare (report only) and Sync | Start with Export, Copy and Sync |
| 3 | How schedules are entered | Presets: every N minutes or hours, daily, weekdays, weekly on chosen days, monthly on a day, all at a local time | Also accept cron expressions |
| 4 | Scheduled writes to Production or protected connections | Allowed only when turned on for that task, after a confirmation, and always with the safety limit | Never allowed |
| 5 | Safety limit | A scheduled run stops before writing when it would delete or replace more than 10% of the target, or more than 3 times the most the task changed in its last 10 successful runs. Changes under 100 documents never stop a run; inserts never count. **Run anyway** runs it once. Editable per task, applies to every scheduled write run (section 6) | 10% only |
| 6 | Missed runs | Run once at the next chance, labelled "Catch-up" | Skip and mark as Missed |
| 7 | Scheduled export file names | The run's date and time are added automatically, in an order that sorts by time and without colons, which Windows doesn't allow in file names (`orders-2026-09-24T0200.jsonl`). "Keep the last 30 files" is optional and off by default; schedules that run more often than daily suggest turning it on | Overwrite the same file each time |
| 8 | Notifications | Failures and problems only, once per outage, plus one when the task works again. Success notifications can be turned on per task | Every failed run |
| 9 | History kept | The last 100 runs per task, and nothing older than 90 days | — |
| 10 | Where Tasks lives | Its own tab, opened from a sidebar button next to Agent Activity, with a badge for problems | Inside Agent Activity |
| 11 | Retries | Temporary failures, such as a dropped connection, retry up to 5 times per step, with random waits that grow from at most 4 seconds to at most 1 minute. The unfinished step resumes where it stopped. A run stops retrying after 15 minutes or when its next scheduled run is due, whichever comes first | Fixed waits, or no automatic retries |
| 12 | Preview | A **Preview** button runs a task without writing and shows what it would insert, replace and delete, and whether the safety limit would stop it. In PR 2 | Later |

Technical choices this plan makes without needing a decision:

- Runs happen one at a time, in a queue.
- Run history is stored encrypted, with its key in the system keychain, like History and AI memory.
- The background runner is the same OpenMango program started without a window, not a separate
  helper.
- The system scheduler holds one entry that wakes OpenMango about every 15 minutes. The schedules
  themselves live in OpenMango.

## Implementation status

**PR 1, tasks you run yourself — built.** Save as task in Transfer and Compare, the Tasks tab with
Run now, Edit, Delete and Cancel, and run history with details per collection and a log.

Where PR 1 differs from the plan, and why:

- **Runs use the sidebar's connections.** A run opens the connections it needs if they are closed,
  as Compare's pickers do, rather than a private connection per run. Transfers run through a
  Transfer tab state that no tab shows, so every export, import and copy path, and its production
  confirmation, is the Transfer tab's own. A private connection per run comes with the retries in
  PR 2, which need a fresh connection anyway.
- **Every task that writes asks before it runs**, in the dialog that also confirms Production
  writes. A Sync task lists both databases first, so the question names how many collections it
  writes.
- **A Sync task stores the collections it leaves out**, not the ones it writes. The Compare tab only
  offers collections that differed at the time, so a saved list would miss collections that were
  identical that day and ones added later.
- **The Tasks tab isn't restored with the workspace**, like Agent Activity. It is one click away in
  the sidebar.
- **Not yet:** Undo from a sync run's details, and the Transfer speed measurements. Both move to
  PR 2.

**PR 2a, safety — built.** PR 2 was split: 2a is safety, 2b is recovery (retries, resuming, a
private connection per run, the stalled-run check, the run time limit, Transfer measurements).

- **Run now previews first** for a Sync, and for an Import or Copy that clears or drops its target:
  the run is recorded at once, works out what it would insert, replace and delete, applies the
  safety limit, then asks. The question gives the counts; when the limit would stop the run it
  says why and its answer is Run anyway. Closing the question ends the run as not confirmed.
  Scheduled runs (PR 3) will stop instead of asking.
- **Preview** does the same and stops there, recorded as a Preview run.
- **The safety limit** is `src/tasks/safety.rs`, as decided. For an Import or Copy that clears or
  drops its target, replacing the whole target is the point, so only the jump and empty-source
  rules apply to it.
- **An empty source** is one more stop reason, so Run anyway can let it through.
- **Mirror deletes last:** inserts and replacements first, then a pass that only deletes, skipped
  when anything before it failed. This applies to every Mirror a task runs.
- **Undo this run** in a sync run's details, for the task's last sync, until the app closes or the
  task runs again. It is recorded as an Undo run.
- **Not yet:** editing the three numbers per task comes with the schedule editor in PR 3; until
  then every task uses 10%, 3 times and 100.

**PR 2b, recovery for Compare and Sync — built.** PR 2's recovery half was split again: 2b covers
the engines tasks call directly, 2c covers transfers.

- **Own connections.** Compare, Sync and Undo runs open connections of their own, each SSH tunnel
  keyed by a fresh id, and close them when the run ends. They never open or disturb the sidebar's.
  Transfers got theirs in 2c.
- **Which failures are retried** follows section 7.1, decided where the error happens
  (`Error::is_transient`) and carried in the engines' messages as `Failure`. A DNS failure always
  counts as one that can pass; telling a typo from an outage by the task's history is left out.
- **Retries** follow section 7.2: per collection, with a fresh connection, full-jitter waits,
  5 attempts, a 15-minute budget, logged in the run. Sync is safe to repeat because it compares
  again and rereads each document before writing.
- **A stalled step** (no progress for 10 minutes) is stopped and retried; **a run past 24 hours**
  stops as failed. Both numbers are fixed until the schedule editor in PR 3.
- **Tests:** the server's `failCommand` hook makes a network timeout three times in a row; the
  driver retries once, the run retries after that, and the run succeeds. A "not authorized" error
  fails at once.

**PR 2c, recovery for transfers — built.**

- **Own connections.** Export, Import and Copy runs, and the document counts their safety check
  reads, use connections the run opens, including the address the BSON tools reach through the
  run's own tunnel. The Transfer tab's code runs them unchanged, through a tab state no tab shows.
- **Retries** use 2b's numbers, but a retry starts the whole transfer over. That is only safe when
  starting over gives the same result: an export, or an import or copy with "Clear target first"
  or "Drop target first". Any other import or copy fails after the first failure, and the log says
  why. A retry on a Production target keeps the write the first try was confirmed for.
- **Not built: resuming from where a transfer stopped** (section 7.3, the last `_id` or line of a
  confirmed batch). Starting over covers the transfers that can repeat safely; resuming comes when
  scheduled imports and copies that append need it.
- **Tests:** `failCommand` makes an export's read time out three times; the run starts over and
  writes the whole file, and the sidebar's connection stays closed. A copy that appends, failing
  the same way, isn't run again.

**PR 3a, schedules while OpenMango is open — built.** PR 3 was split: 3a runs tasks on their
schedules safely, 3b adds needs attention, notifications and the outage rules (section 7.4).

- **The schedule editor** opens from Schedule… among the task's actions, not from the Save as
  task dialog. It holds the rule, a calendar that dims the days it doesn't run beside the next
  three run times, the safety limit's three numbers for a task that writes, the Production opt-in,
  and "Keep only the newest 30 files" for an export. Pause and Resume sit beside Schedule…; a
  resumed schedule doesn't catch up. The details list Schedule and Safety in the same label and
  value list as the reference peek.
- **Rules:** Every… counts from midnight, so every 2 hours runs at 00:00, 02:00 and so on, however
  the schedule was set. A monthly day a month doesn't have runs on its last day. Both clock changes
  follow section 5.1, with `chrono-tz` in the tests.
- **The scheduler** (`src/state/commands/task_schedule.rs`) waits for the earliest due run, but
  looks at the clock at least once a minute: timers don't count time the computer sleeps, so a run
  due during sleep starts within a minute of waking. A due time found more than 2 minutes late is
  a Catch-up; waiting in the queue behind another run doesn't make one.
- **The queue** starts a scheduled run only when no task is running. A task still running when it
  comes due is recorded as ⏭ Skipped, and so is a catch-up skipped because the next run is close.
- **Approval** records each connection's identity hash, the one Agent Activity uses. It includes
  the connection's name, so renaming a connection asks for approval again. A problem shows in the
  task's details with Approve again, which asks first when the target is Production or protected.
- **Scheduled runs never ask.** The safety limit stops them before writing, recorded as ⚠ Failed
  with its reasons and a Run anyway… button that goes through Run now. The approval stands in for
  the Production write confirmation: a run grants itself exactly the writes it makes. Retrying
  stops when the task's next run is due.
- **Scheduled exports** get their run's time in the file name unless the path already has a
  `${date}`, `${datetime}` or `${time}` placeholder. Keeping the newest 30 deletes only files named
  that way.
- **Tests:** next-run rules for every preset, month ends and both clock changes; the queue, the
  catch-up, skipping and pausing; approval and its revocation; the editor in a window, including
  the question before scheduled Production writes; and in Docker, a scheduled Mirror stopped by the
  limit, then writing into Production without a question, and scheduled exports keeping two files.

**PR 3b, problems and notifications — built.** Section 4.3's list, less the parts that belong to
the background runner (PR 4), and section 7.4.

- **Needs attention** (`AppState::task_attention`) applies to scheduled tasks only: whoever
  pressed Run now saw the result. A task needs attention when a connection it uses was deleted,
  signing in failed, its approval no longer matches, the safety limit stopped its last run, or its
  last run failed, was partly done or was cut short. A run records whether its failure can pass
  (`Run::failure`); such a failure needs attention only at the third in a row. Only a failure of
  the whole run is sorted that way: a collection that ran out of retries counts as lasting, so the
  shortcut can only add attention, never hide it.
- **The details** show the reason with its fix: Approve again, Edit, Resume, Run anyway… or Show
  run. A Needs attention filter sits above the list while any task needs it, the row shows ⚠, and
  the sidebar's Tasks button carries a count like Agent Activity's.
- **Notifications** use the app's error notification, with Open task, which selects the task. A
  scheduled run that fails notifies unless the run before it failed the same way; the first one
  that succeeds after a failure says "works again" in the status bar. Schedule… has "Also notify when
  a scheduled run succeeds". Runs someone started don't notify.
- **A failed sign-in** pauses a scheduled task, from any run. Saving the connection resumes it,
  since every edit gives the connection a new secret id; so does Resume.
- **Fix to PR 2:** connecting wrapped every error in plain text, so a run whose server couldn't be
  reached at the start failed at once instead of waiting and retrying. `Error::Connect` keeps the
  helpful text and the error it came from.

**PR 4, the background runner and macOS — built, not yet verified on a real login session.**

- **The lock** (`src/tasks/lock.rs`) is `tasks.lock` in the config folder, held with
  `File::try_lock`. The app takes it before its scheduler starts anything; an app that opens
  while the runner works waits for it, looking once a minute, then reads the tasks and runs again.
  Whoever takes it marks runs no process is finishing as interrupted, except its own. The run store
  has a 5-second busy timeout, since both processes can write it.
- **Both processes can write `tasks.json`** while the runner works and the app is open. The
  runner writes the list as it is on disk with only its own fields: when a task last ran, and the
  sign-in pause. The app, while it waits for the lock, keeps the later "last ran" from disk. So
  neither loses the other's changes, and no run happens twice.
- **`openmango --run-due-tasks`** (`src/app/background.rs`) takes the lock and reads `tasks.json`
  before starting gpui, and returns at once when the app is open or nothing is due. Otherwise it
  starts gpui without a window, reads the passwords of the connections the due tasks use, changing
  nothing in the keychain, opens the run history, and runs the due tasks that may run while
  OpenMango is closed through the same scheduler. It exits when nothing runs or waits. Its runs
  log "Started while OpenMango was closed." It sends no notifications yet.
- **macOS:** the launch agent is `Contents/Library/LaunchAgents/com.openmango.app.tasks.plist`
  (StartInterval 900, RunAtLoad), written by `scripts/release_macos.sh` and registered with
  `SMAppService` through `objc2` (`src/helpers/background_runner.rs`). It's registered when the
  first task turns on "Run even when OpenMango is closed" and removed when the last one turns it
  off or is deleted. `SMAppService` needs macOS 13 and an app bundle, so on macOS 11 and 12, and
  in `cargo run` builds, the option is shown switched off with why. A task set to run while closed
  needs attention when the agent is off in Login Items, with Open Login Items.
- **At launch** the app asks the system about the entry only when a task uses it or one is known
  to be registered; on Windows and Linux asking starts a program. The schedule editor asks when it
  opens, to show whether the option is available.
- **Verified:** a `--run-due-tasks` run with no window, from a dev build signed with the
  development identity, read a Sync task's connection passwords from the keychain without a
  prompt, ran the task and recorded it (2026-09-24).
- **Found on a signed build (2026-09-24):** `SMAppService` reports "not found" for an agent that
  is in the bundle but has never been registered, so the status reads as not registered whenever
  `Contents/Library/LaunchAgents` has the file; only a bundle without it can't have one.
- **Not verified yet:** registering the launch agent, the Login Items flow and a launchd-started
  run, which need the app built as a bundle.

**PR 5, Windows — built, checked on the Windows CI machines, not yet on a signed-in PC.**

- **The entry** is the Task Scheduler task `OpenMango\Run due tasks`, created with the system's
  `schtasks.exe /Create /XML` from a definition in `src/helpers/background_runner.rs`, so no new
  crate. Its settings are section 5.3's: every 15 minutes, `StartWhenAvailable`, the battery
  settings off, `IgnoreNew` for a start while one still runs, and `ExecutionTimeLimit` of 25
  hours, one above a run's own limit. Its start boundary is one minute past a quarter hour, so a
  task due on the quarter hour runs a minute later instead of up to 15.
- **It runs as the signed-in user, only while they're signed in** (`InteractiveToken`). That is
  what lets it read the passwords Credential Manager keeps for them, like the macOS agent, which
  also runs only in the user's session. It needs no administrator rights and stores no password.
- **Status** comes from `schtasks /Query /XML`: missing is not registered, `<Enabled>false` is
  switched off, which needs attention with Open Task Scheduler. The XML is read for its markup,
  not the localized text `schtasks` prints otherwise.
- **The installed program is a Windows GUI program**, so a start shows no window. Development
  builds are console programs and would flash one every 15 minutes, and a running runner would
  stop cargo from replacing the program, so they don't add the task; `cargo run --
  --run-due-tasks` tries the runner, as on macOS.
- **Uninstalling** deletes the task (`[UninstallRun]` in `resources/windows/openmango.iss`).
- **Checked:** the module compiles and passes clippy for Windows. On the Windows CI machines, a
  unit test adds a throwaway task with the real `schtasks`, reads it back as on, disables it,
  reads it as switched off, and deletes it; the package check adds the task and makes sure the
  uninstaller removes it.
- **Not verified yet:** Task Scheduler actually starting the installed program, and that run
  reading the passwords, which need a Windows PC with OpenMango installed and a signed-in user.

**PR 6, Linux — built, checked on the Linux CI machines, not yet in a desktop session.**

- **The entry** is a systemd user timer and service in `~/.config/systemd/user`:
  `openmango-tasks.timer` with `OnCalendar=*:1/15` and `Persistent=true`, and
  `openmango-tasks.service`, a oneshot that starts the AppImage with `--run-due-tasks`
  (`TimeoutStartSec=25h`, and `APPIMAGE_EXTRACT_AND_RUN=1` when OpenMango runs that way). A
  oneshot still running isn't started again. Written by `src/helpers/background_runner.rs`, then
  `systemctl --user daemon-reload` and `enable --now`; removing disables it and deletes both.
- **It runs while the user is signed in**, since the user's systemd manager stops at sign-out
  unless lingering is on. `Persistent` makes up a start missed while it didn't run, once.
- **Only the AppImage** can have it, through `helpers::linux::appimage_path`, the file the updater
  also uses. When the AppImage has moved, or the service differs from what this version writes,
  the status reads as not registered and the timer is written again.
- **Status** from `systemctl --user is-enabled`; `disabled` or `masked` needs attention. Without
  a systemd user session (`systemctl --user show-environment` fails), the option is shown off
  with why. There's no settings window for timers, so the fix button is Turn on, which writes and
  enables the timer again.
- **A locked keyring** (section 5.3): gpui unlocks the Secret Service collection before reading,
  which shows a dialog. With no window, the run would wait for an answer while holding the task
  lock, and an OpenMango opened meanwhile would wait too. So the runner first asks the default
  collection's `Locked` property with `busctl`, and when it's locked, logs it and exits. Changed
  from the plan: the due runs aren't recorded as waiting; they stay due, for the next start or
  for OpenMango when it opens, which asks to unlock the keyring as usual.
- **Every system:** the runner's status is read again, off the main thread, whenever OpenMango's
  window becomes active, so switching it back on in Login Items, Task Scheduler or systemd clears
  the attention without a restart.
- **Checked:** the module compiles and passes clippy for Linux. On the Linux CI machines,
  `systemd-analyze verify` accepts both units, and a unit test covers quoting the AppImage path.
- **Not verified yet:** the timer starting the AppImage in a desktop session, reading the
  passwords from an unlocked keyring, and skipping while it's locked.

**PR 7, system notifications — built.** Section 4.3's notifications, from the system itself.

- **What's sent:** the news a scheduled run already gives in OpenMango (PR 3b): its first failure
  in an outage, a paused schedule after a failed sign-in, working again, and a success when the
  task asks for it. Each is collected as the run ends (`TaskNotice`) and posted once, one per
  task: a newer one replaces the older.
- **In the app**, only when its window isn't active; otherwise its own notification or the
  status bar already says it. Changed from the plan, which kept an open app to in-app messages.
  These have an Open task button.
- **From the background runner**, always, after its runs, then a two-second wait before it
  exits, since macOS takes the notification after the call returns. Without a button: the runner
  has exited by the time someone clicks, and on Windows and Linux a click only reaches the
  process that posted. On macOS clicking one starts OpenMango, which registers its handler while
  it launches, so the click opens the task.
- **Clicks** anywhere on a notification, or Open task, select the task in the Tasks tab and bring
  OpenMango forward.
- **Permission (macOS):** gpui asks the first time a notification is posted, and has no way to ask
  earlier. Changed from the plan, which asked when a task is first set to run while closed.
- **Checked:** a unit test with gpui's test platform: nothing posted while someone's looking,
  Open task only from the app, a newer notification replacing the older, and a click selecting
  the task.
- **Not verified yet:** the notifications on a real desktop on each system, and a click on a
  macOS notification starting OpenMango.

**After trying it: saving, notifications and approval.** Changes from using the built tasks.

- **Save as task says what it saves.** It used to save a sync while the sync list was open and a
  comparison otherwise, asking only for a name. For two databases it now offers "Compares only"
  or "Compares, then syncs", with the side and mode, starting from the sync list or the task the
  tab belongs to; a sync can be saved with nothing to sync yet. A sentence says what each run
  does, and the task's details show it too. Save task on a tab linked to a task saves at once
  unless it would change whether or how the task syncs; then the dialog shows it first.
- **Saving a task to write another way withdraws its approval:** a different kind, write
  connection, or sync direction or mode. Before, a scheduled Add missing saved as a Mirror kept
  running on its old approval.
- **Every run, if asked:** Schedule…'s notification option now covers each scheduled run's start
  and end, replacing "notify on success". The end replaces the start's notification. Saved tasks
  keep their choice (`notify_success` still reads). The background runner posts as runs start
  and end, not only after all of them.
- **Approval lost on every connect:** each connect saved the connection through the full save, which
  gives it a new keychain id, part of the approval fingerprint. Connecting now saves only the
  time, so approvals and agent grants hold.
- **Run while closed on Windows and Linux:** the option was still shown only on macOS.
- **The Dock (macOS):** the background runner no longer bounces OpenMango's Dock icon.

## 1. What the evidence says

**DBeaver**, the closest comparable desktop database tool:

- Hands schedules to the operating system: Windows Task Scheduler on Windows, cron on macOS and
  Linux. Each run goes through DBeaver's own command line.
- Keeps a run log per task. Double-clicking a run shows the full log with output, errors and
  warnings. Logs are stored in the workspace.
- Its main pitfall is credentials. Scheduled tasks fail when passwords can only be unlocked by a
  signed-in user, so DBeaver added an "Automation (console)" mode that it describes as less
  secure.

**Studio 3T:**

- **New Task** offers a list of task types. Choosing one opens that tool's tab, where the task is
  configured and saved. This plan's "New task" works the same way.
- The scheduler has preset recurrences, including Monthly (chosen days of the month at a time) and
  Custom (days of the week or month, run once or repeated every N hours or minutes within the day).
  Schedules can have a start date and an optional end date.
- Running a Data Compare & Sync task opens a Comparison Results tab.
- Its documentation doesn't say whether schedules run while Studio 3T is closed.

**How each system handles missed runs**, from the primary documentation:

- **launchd (macOS):** "Unlike cron which skips job invocations when the computer is asleep,
  launchd will start the job the next time the computer wakes up." Several missed times become
  one run.
- **systemd timers (Linux):** a calendar timer missed during sleep runs once after resume.
  `Persistent=true` also covers runs missed while the computer was off.
- **Windows Task Scheduler:** `StartWhenAvailable` lets a missed task start as soon as possible.
  Two defaults work against laptops: `DisallowStartIfOnBatteries` and `StopIfGoingOnBatteries` are
  both true, so a task doesn't start on battery and stops when the laptop is unplugged.
- **cron** skips runs while the computer is asleep, which is why this plan doesn't use it.

**macOS 13 and later:** Apple recommends `SMAppService` for launch agents. Every background item,
however it was added, is listed under System Settings > General > Login Items & Extensions, where
the user can switch it off.

**The UI framework** (gpui) already has what the background runner needs:

- A mode with no window (`gpui_platform::headless()`) that uses the same platform code as the
  app, including its keychain calls.
- System notifications on macOS, Windows and Linux (`cx.show_system_notification`), with action
  buttons.

**Retries**, from MongoDB's documentation and AWS's retry guidance:

- The driver retries a supported read or write **once**, which covers a brief network drop or a
  replica set election, "but not persistent network errors".
- `getMore`, the call that fetches the next batch of a long read, is **not** retried. A connection
  that drops in the middle of scanning a collection fails that scan.
- Retrying is only safe when repeating a step has the same effect as doing it once.
- Waits should grow exponentially up to a cap, with random jitter so many clients don't retry in
  step. AWS's "full jitter" waits a random time between zero and the capped value.
- Errors that won't go away, such as a wrong password, should fail at once instead of retrying.

**This codebase** already has:

- `TransferConfig` and `TransferOptions`, and `CompareConfig`, all serializable. Tabs already save
  and restore them, so a task can store them as they are.
- The Agent Activity tab (`src/views/agent_activity.rs`), which lists stored operations with
  statuses such as Running, Failed and Interrupted, and marks runs cut short by a crash as
  interrupted at startup (`ActionStore::reconcile_interrupted`).
- A rule that revokes agent write access when a connection's address changes or it becomes
  Production or protected (`apply_agent_sharing_safety`). Scheduled write approval follows the
  same rule.
- Error kinds (`src/error/report.rs`): Connection, Timeout, Auth, Server, Validation, Conflict and
  Io. Driver errors are already sorted into them, which is where retry decisions start.
- The driver's own list of retryable server codes (`mongodb-3.5.1/src/error.rs`), such as "not
  primary", "shutting down" and "host unreachable". It isn't public, so the runner keeps its own
  copy, with a comment naming the driver version it came from, to recheck when the driver is
  upgraded.

### 1.1 Decisions checked against general practice

| # | Practice elsewhere | Result |
|---|---|---|
| 1 | Safety checks before automation; rclone recommends a dry run or confirmations while setting up a sync | Kept. Safety and Preview land in PR 2, before schedules |
| 2 | Studio 3T offers Import, Export and Data Compare & Sync tasks, among others | Kept |
| 3 | DBeaver and Studio 3T use presets; cron syntax is for developers | Kept. Cron in Later |
| 4 | Least privilege: unattended writes only where explicitly allowed | Kept |
| 5 | rclone stops a sync with a fatal error past `--max-delete` documents. A percentage alone lets huge collections lose millions of documents and trips on tiny ones; a fixed count trips every night on big collections that really do change a lot | Changed: 10% or an unusual jump against the task's own history, with a floor of 100 documents (section 6) |
| 6 | launchd and systemd run one catch-up; Kubernetes CronJobs can skip a start that is too late | Changed: the catch-up is skipped when the next regular run is close (section 5.2) |
| 7 | Sortable timestamps; Windows forbids `:` in file names | Changed: `orders-2026-09-24T0200.jsonl` |
| 8 | Google SRE: alert on real problems, avoid alert spam, keep the rest on a dashboard | Kept: one notification per outage, the rest in Needs attention |
| 9 | Kubernetes keeps only a few finished Jobs by default | Kept. 100 runs is small for a desktop app |
| 10 | — | Kept |
| 11 | AWS full jitter; Azure: finite retries, a retry budget, no stacked retry layers, fail fast on permanent errors | Kept, plus a stalled-run check and a run time limit (section 7.2) |

Also adopted from this check:

- **Mirror deletes last, and not at all after an error**, as rclone's default `sync` does
  (section 6).
- **An empty source never empties the target** (section 6).

## 2. What a task is

A task stores:

- **Name**, chosen by the user, and **kind**: Export, Import, Copy, Compare or Sync.
- **Settings**: the Transfer or Compare settings exactly as the tab holds them. A Sync task also
  stores the direction, the mode (Add missing, Add and update, Mirror) and which collections are
  included.
- **Schedule**: manual only, or one of the presets. Paused or active.
- **Run even when OpenMango is closed**: off by default.
- **Safety**: the safety limit, and whether scheduled writes to Production or protected
  connections are allowed.
- **Notifications**: failures only, or every run.

A task refers to saved connections by id. It never copies addresses or passwords. Credentials are
read from the keychain at run time, the same way connecting works today.

**Approval of scheduled writes.** When a task that writes gets a schedule, OpenMango records each
connection's identity, the same fingerprint the sync code and Agent Activity already use. Scheduled
runs stop with "Connection settings changed since this task was approved" when:

- the connection's address, SSH or proxy settings change,
- it becomes Production or protected,
- it is deleted.

One click on **Approve again** records the new identity. "Run now" is unaffected: it shows the
usual review and confirmation.

A read-only connection can't be the target of a writing task, as today.

## 3. The user flow

1. **Save a task from the tool.** Transfer and Compare get a **Save as task** button next to Run or
   Compare. It opens a small dialog: Name, pre-filled (for example "Export orders"), Schedule
   (Manual by default), and **Save**. For a writing task with a schedule, the dialog also shows the
   safety settings.
2. **Or start from Tasks.** **New task** offers Export, Import, Copy, Compare and Sync. Each one
   opens the matching tool, with **Save task** in place of Save as task.
3. **Run it.** **Run now** in the Tasks tab. Writing tasks show the same review and confirmation as
   in their tool. **Preview** (decision 12) runs the task without writing and shows what it would
   insert, replace and delete, and whether the safety limit would stop it. It's the way to check a
   writing task before giving it a schedule.
4. **Schedule it.** Choose a preset and a time. The next three run times are shown as you edit, for
   example "Next: Wed 24 Sep, 02:00".
5. **See what happened.** The task list shows each task's last result and next run. Selecting a
   task shows its history; selecting a run shows its details.
6. **Fix a problem.** A task that needs attention shows why, with the fix beside it: Run again,
   Approve again, Open System Settings, or Edit.
7. **Edit** opens the task in its tool. **Save task** updates it; **Save as new task** copies it.

## 4. Screen design

Rules applied from `/ui-skills`, `/better-ui` and `/emil-design-eng`:

| Rule | Where it applies |
|---|---|
| Use the project's existing components first | The list, split view, buttons, dialogs and confirmation are the ones Compare and Agent Activity use |
| Empty states have one clear next action | "No tasks yet" with one primary **New task** button, and a line saying tasks can also be saved from Transfer and Compare |
| Errors appear where the action happened | A failed run shows its error in its history row and run details, not only in a notification |
| Destructive or irreversible actions use a confirmation dialog | Deleting a task, turning on a schedule for Mirror, and allowing scheduled writes to Production |
| One accent color per view | Only the primary button uses the accent. Results use muted text and icons |
| A state change is never shown by color alone | Every result has an icon and a word: ✓ Succeeded, ⚠ Failed, ◐ Partly done, ⏭ Skipped, ⏸ Paused |
| Tabular numbers for data | Counts, durations and times |
| No animation unless it has a purpose | Nothing new animates. Selecting, running and finishing change icons and text. Rows don't reorder while a task runs |
| Concentric radii and existing tokens | The islands theme's radii, spacing and shadows, as in Compare |

### 4.1 Tasks tab

```
Tasks                                                            [+ New task ▾]
┌───────────────────────────────┬──────────────────────────────────────────────┐
│ ⚠ Nightly mirror              │ Nightly mirror                               │
│   Sync · Daily 02:00          │ Sync · Mirror · prod / shop → local / shop   │
│   Failed today 02:00          │ [Run now]  [Preview]  [Edit]  [Pause]  ···   │
│ ✓ Orders export               │                                              │
│   Export · Weekdays 07:30     │ ⚠ Connection settings changed since this     │
│   Succeeded 07:30             │   task was approved.     [Approve again]     │
│ ○ Staging check               │                                              │
│   Compare · Manual            │ Schedule  Daily at 02:00, local time         │
│                               │           Next: Thu 25 Sep, 02:00            │
│                               │           ☐ Run even when OpenMango is closed│
│                               │ Safety    Stop if more than 10% of the       │
│                               │           target would be deleted or replaced│
│                               │ History                                      │
│                               │  Today 02:00      ⚠ Failed      0:04         │
│                               │  Yesterday 02:00  ✓ Succeeded   3:12  +120 ~40 −3 │
└───────────────────────────────┴──────────────────────────────────────────────┘
```

- **List (left):** name, kind and schedule on the second line, last result on the third. Sorted
  by name. A **Needs attention** filter appears above the list when any task has a problem.
- **Detail (right):** the summary line, actions, the problem banner if any, then Schedule, Safety
  and History. Selecting a history row opens that run's details in place of the history.
- **Run details:** start time, what started it (you, the schedule, the background runner, or a
  catch-up), duration, result, counts, then a log per collection with warnings and errors. Buttons:
  Copy error, Run again, and Undo for a sync run whose undo is still available.
- **Running:** the row shows the progress line Transfer and Compare already use, and the detail
  has Cancel.

### 4.2 Schedule editor

- **Repeat**: Manual, Every…, Daily, Weekly, Monthly.
  - **Every…** takes a number and minutes or hours, from 15 minutes.
  - **Daily** takes a time and an optional "Weekdays only".
  - **Weekly** takes day toggles and a time. **Monthly** takes a day of the month and a time.
- The time zone is the computer's, shown as a label ("local time, Asia/Tbilisi").
- The next three run times update as you edit. That is the check that the schedule means what you
  intended.
- **Run even when OpenMango is closed** appears once the background runner exists for the platform.
  Turning it on the first time explains that OpenMango will appear in the system's login items.

### 4.3 Problems and notifications

A task **needs attention** when:

- its last run failed,
- its last three runs failed, even for temporary reasons such as a server outage,
- runs were missed and the catch-up also failed,
- the safety limit stopped a run,
- a run stopped as Partly done because it couldn't resume safely (section 7.3),
- a sign-in failure paused its schedule,
- its connection settings changed since approval, or a connection was deleted,
- the background runner was switched off in System Settings (macOS),
- the keyring was locked, so passwords couldn't be read (Linux, before login).

The sidebar button shows a badge with the number of tasks that need attention, like the Agent
Activity badge. A background run that fails also sends a system notification with an **Open**
button. While the app is open, the usual in-app message is used instead.

macOS asks permission the first time an app posts a notification. OpenMango asks at the moment a
task is first set to run while OpenMango is closed, with a line saying why, rather than surprising
the user during a run in the night.

### 4.4 States

| State | What shows |
|---|---|
| No tasks | Empty state with **New task** |
| Loading | The list's existing loading rows |
| Never run | "Not run yet" in place of the last result |
| Running | Progress line, Cancel |
| Paused | ⏸ Paused, with the schedule shown dimmed |
| Needs attention | ⚠ and the reason, with its fix |

### 4.5 Keyboard and accessibility

- Arrow keys move through the list, `enter` opens the selected task in its tool, and `cmd-F` finds a
  task by name. Delete asks for confirmation. Run now gets a binding in the keymap.
- Each row's accessible label reads as one sentence: "Nightly mirror, Sync, daily at 02:00, last
  run failed".
- Rows wrap at 200% text size instead of cutting off the result.

## 5. Scheduling

### 5.1 Working out the next run

A schedule is stored as a rule, such as "daily at 02:00", not as a list of times. The next run is
worked out with `chrono`, which is already a dependency, in the computer's local time zone.

- **Clocks going forward:** a time that doesn't exist that day, such as 02:30, runs at the first
  valid minute after it.
- **Clocks going back:** a time that happens twice runs once, at the first occurrence.
- Changing the computer's time zone moves the local times with it.

### 5.2 In-app scheduler

- One timer for the earliest next run. Nothing runs between runs.
- When the timer fires, OpenMango compares due times with the wall clock, which also catches runs
  missed during sleep. The same check runs at startup.
- A missed run runs once, labelled Catch-up (decision 6). The catch-up is skipped when the next
  regular run is less than half an interval away. A daily 02:00 task whose computer wakes at 23:00
  waits for 02:00 instead of running twice in three hours. This is the idea behind the "starting
  deadline" of Kubernetes CronJobs.
- Runs go through one queue, one at a time, like the `Forbid` policy of Kubernetes CronJobs. A task
  that is still running when it comes due again is recorded as Skipped with the reason "still
  running".
- The runner opens its own connection for the run and closes it afterwards, so a task doesn't need
  the connection to be open in the sidebar.

### 5.3 Background runner

The system scheduler starts `openmango --run-due-tasks` about every 15 minutes. That run starts
without a window, runs whatever is due, writes the history, sends notifications for failures and
exits.

- **When OpenMango is open,** it holds a lock file and runs tasks itself. The background run sees the
  lock and exits at once. Locks use `File::try_lock` from the standard library.
- **One system entry in total,** added when the first task turns on "Run even when OpenMango is
  closed" and removed when the last one turns it off. Editing a schedule never touches the system.
- **Runs can start up to 15 minutes late.** That is acceptable for database tasks.

Per platform:

| Platform | Entry | Settings that matter |
|---|---|---|
| macOS | A launch agent bundled in the app and registered with `SMAppService` | Starts every 900 seconds. The app reads its status to warn when it's switched off in Login Items |
| Windows | A Task Scheduler task, through `schtasks` | Repeats every 15 minutes. `StartWhenAvailable` on. `DisallowStartIfOnBatteries` and `StopIfGoingOnBatteries` off. `ExecutionTimeLimit`, which stops a task after 72 hours by default, set just above OpenMango's own run time limit (section 7.2) |
| Linux | A systemd user timer and service | `OnCalendar=*:1/15`, `Persistent=true`. Without a systemd user session, the option is shown as unavailable with the reason |

**Passwords without a window.** The run reads them through the same keychain calls as the app:

- On macOS it is the same signed app, so its keychain entries shouldn't prompt. PR 4 verifies this
  first.
- On Linux, the keyring stays locked until the user logs in after a restart. Those runs wait and
  are recorded as "waiting for the keyring", not failed.

## 6. Safety for unattended runs

"Run now" always shows the review and confirmation its tool shows today. Scheduled runs can't ask,
so writing tasks get these rules instead:

- **Safety limit (decision 5).** It catches mistakes nobody is there to see, such as a source that
  was emptied or half-restored, or a task pointing at the wrong database. Before writing, the run
  counts the documents it would delete or replace on the target. Inserts never count, since they
  destroy nothing.
  - Sync knows the count from its comparison, per collection.
  - Import and Copy with "Clear target first" or "Drop target first" use the target's document
    count.

  The run stops before any write when, for any collection:
  1. the count is more than **10%** of the target collection's documents, or
  2. the count is more than **3 times** the largest count of the task's last 10 successful runs for
     that collection. This check starts once the task has 3 successful runs, and
  3. in either case, the count is at least **100** documents, so small collections aren't stopped
     by small edits.

  Why both: 10% alone would let a 50-million-document collection lose 5 million documents, and a
  fixed count would stop every night a large collection that really does change 300,000
  documents a night. Comparing with the task's own history allows its usual volume and stops an
  unusual jump: on that collection, an accident deleting 5 million documents is about 16 times the
  usual and stops.

  A stopped run needs attention and shows the numbers: "Would delete 5,012,344 documents from
  orders; this task usually changes at most 310,000." **Run anyway** runs it once, with the
  confirmation a Run now has. The three numbers can be changed per task, and Preview shows the
  counts before a schedule is set. Runs stopped by the limit don't count as successful, so they
  don't raise the task's usual volume.
- **An empty source never empties the target unasked.** A Mirror, Import or Copy whose source has
  no documents while the target has some stops, whatever the limit, even below the 100-document
  floor. An empty source is more often the wrong database or a failed restore than an intended
  wipe. Run anyway lets it through when the wipe is intended.
- **Mirror deletes last.** Today database sync writes inserts, replacements and deletes in the order
  it finds them. A scheduled Mirror instead makes two passes per collection: first inserts and
  replacements, then a second pass that only deletes, using the comparison's existing row-kind
  filter. The delete pass is skipped when anything in the first pass failed. This follows rclone's
  default for `sync`, which "will only delete files if there have been no errors". The cost is one
  more read of each collection that has documents to delete.
- **Production and protected targets (decision 4)** are refused unless the task allows them.
  Allowing them uses a confirmation dialog that names the connection.
- **Approval** is revoked when a connection changes, as described in section 2.
- **Undo** stays available for sync runs as it is today. Run details link to it.

## 7. Failures, retries and resuming

A dropped connection, a laptop going to sleep or a replica set election in the middle of a run
should still end in a correct target: no half-copied collection left as if it were done, and no
duplicate documents. A failure that won't fix itself should stop at once and say why.

### 7.1 What is retried

| Retried | Not retried: fails at once |
|---|---|
| Connection dropped, reset or refused, or no server available | Wrong password or another sign-in failure |
| Timeouts | Not authorized for the operation |
| Server errors the driver itself treats as retryable: not primary, node recovering, shutting down, host unreachable, network timeout and the rest of its list | A document rejected by the collection's validator, or an invalid filter |
| The SSH tunnel or proxy dropped | A duplicate key in Insert mode, except while resuming (7.3) |
| A DNS lookup failed, for a task that has succeeded before | A DNS lookup failed for a task that has never succeeded, which is more likely a typo |
| | Disk full, or a file missing or unreadable |
| | The safety limit, or Cancel |

Two situations wait instead of failing:

- The keyring is locked (Linux, before login). The run waits for it.
- Another program changed a target document between the comparison and the write. Sync already
  rereads each document before writing and skips it; it counts as skipped, not failed.

### 7.2 How retries work

- **Per step, not per task.** A step is one collection, or one batch of an import. A failed step
  never restarts the whole task.
- **Fresh connection first.** Before each retry the runner opens a new connection, including a new
  SSH tunnel if there is one, and checks it with `ping` before continuing.
- **Waits** use full jitter: a random time between zero and the smaller of 60 seconds and
  2 seconds × 2 to the power of the attempt number. The caps are about 4, 8, 16, 32 and 60 seconds.
- **Limits (decision 11):** 5 attempts per step, 15 minutes of retrying per run, and never past the
  next scheduled run of the same task.
- **Sleep doesn't count.** Time the computer spends asleep isn't counted toward the 15 minutes.
  After waking, the step retries at once.
- **Cancel** works during a wait.
- **A stalled run counts as a timeout.** A step that makes no progress for 10 minutes, no documents
  read or written, is stopped and retried like a timeout. A driver call can wait on a half-open
  connection longer than that.
- **Run time limit.** A run that is still going after 24 hours stops as ⚠ Failed. The limit can be
  changed per task. Kubernetes Jobs (`activeDeadlineSeconds`) and Windows Task Scheduler (72 hours)
  have the same kind of limit.
- **Retries don't multiply unchecked.** The driver already retries a single call once. Azure's
  retry guidance warns that stacked retry layers multiply attempts. Here the worst case is the
  driver's one retry inside each of the runner's 5 attempts, all within the 15-minute budget.
- **Everything is logged:** "Connection dropped while copying orders (attempt 2 of 5). Waited 6 s."
  A run that needed retries but finished shows ✓ Succeeded, with "after 2 retries" in its details.

### 7.3 Resuming without repeating work

Each kind saves its progress in the run's record after every confirmed step. A retry, or a run cut
short by a crash or by quitting the app, continues from there instead of starting over.

| Kind | Saved progress | How it resumes |
|---|---|---|
| Compare, Sync | Finished collections | Only the unfinished collection runs again: it is compared again, then whatever still differs is written. Documents written before the failure now match, so they aren't written again. |
| Copy | Finished collections, and within a collection the last `_id` of a confirmed batch | The source is read in `_id` order from after that `_id`. The batch that was in flight is re-sent as replace by `_id`, so documents it had already written aren't duplicated. "Clear target first" and "Drop target first" aren't repeated. |
| Export | None within a file | Exports write to a temporary file and rename it only when complete, so a failed export never leaves a partial file under the real name. A retry starts that file again. |
| Import, JSON Lines or CSV | The line number of the last confirmed batch | Documents that have `_id`: the import continues after that line, re-sending the in-flight batch as replace by `_id`. Documents without `_id` can't be resumed safely, because the server gives every insert a new `_id` and a repeated batch would duplicate documents. The run stops as ◐ Partly done with the count imported, and needs attention. |
| Import or export with the BSON tools | None | Export: the dump is written again. Import: resumes only when "Drop target first" is on, since the drop makes the restart clean. Otherwise the run stops as ◐ Partly done. |

A run cut short by a crash or by quitting is resumed at the next chance, like a missed run, when its
kind can resume. Otherwise it is marked Interrupted and needs attention.

Reading Copy's source in `_id` order goes through the `_id` index, which can be slower on large
collections than reading in stored order. PR 2 measures the difference.

### 7.4 Long outages

- A run that uses up its retries ends as ⚠ Failed, for example "Server unreachable for 15 minutes".
  The next scheduled run tries again as usual.
- **One notification per outage (decision 8).** The first failed run notifies. Later failures for
  the same reason don't. The first run that succeeds afterwards sends "Nightly mirror works again".
- **Three failed runs in a row** make the task need attention even when each failure was temporary,
  so a long outage isn't missed.
- **A sign-in failure pauses the schedule** until the connection is fixed or the user resumes it.
  Repeating a wrong password every 15 minutes can lock the account on servers that lock accounts
  after failed sign-ins.

## 8. Run history

Each run records:

- when it started and how long it took,
- what started it: you, the schedule, the background runner, or a catch-up,
- its result: Succeeded, Failed, Partly done, Skipped or Cancelled,
- counts: inserted, replaced, deleted, exported, imported, differences found,
- a log per collection, capped at 1,000 lines per run, with warnings and errors.

**Storage.** An encrypted SQLite database, `tasks.db`, with the same SQLCipher setup as History and
its key in the system keychain. Logs never contain document contents. Server error messages can
include values (a duplicate-key error names the key), which is why the store is encrypted.

**Retention (decision 9).** The last 100 runs per task and nothing older than 90 days, trimmed
after each run.

**Crashes.** A run still marked Running at startup is marked Interrupted, the way
`reconcile_interrupted` handles agent operations. It then resumes from its saved progress at the
next chance, when its kind can resume (section 7.3).

## 9. Performance

**While idle.**

- In the app: one timer, no polling.
- With the background runner on: one short process about every 15 minutes. Target: it exits in
  under a second when nothing is due. Measure in PR 4.

**During a run.** Runs use the existing engines, which already stream:

- Comparison read about 600,000 documents per second locally, with about 132 MiB peak memory, in
  the million-document benchmark
  ([COMPARE_BENCHMARKS.md](COMPARE_BENCHMARKS.md)).
- Sync wrote about 14,000 replacements per second locally, including encrypted undo records.
- Database sync writes as it compares, so memory stays flat regardless of database size.

For example, a nightly Mirror of a 1,000,000-document database where 1% changed would take a few
seconds to compare and under a second to write, locally. Remote servers will be slower; nobody has
measured that yet. Transfer's export, import and copy speeds haven't been measured either. PR 1
adds a measurement for each.

**Effect on the open app.** Runs happen off the UI thread, as Transfer and Compare do today. The
queue keeps two heavy runs from competing for the same network and servers.

**History.** One small row per run, plus capped log lines, trimmed after each run.

## 10. Fitting into the app

- `TabKey::Tasks`, a single tab like Agent Activity, restored with the workspace.
- A sidebar button next to Agent Activity, with the needs-attention badge.
- Command palette: "Tasks", "New task…", and "Run task…", which picks a task by name.
- **Save as task** in the Transfer and Compare tabs. **Save task** when a tab was opened from a task.
- Agent Activity stays separate. Merging agent operations and task runs into one activity view is
  in Later.

## 11. Build order

Stacked PRs, each usable on its own:

1. **Tasks you run yourself.** The task model and store, Save as task in Transfer and Compare, the
   Tasks tab with Run now, Edit and Delete, and run history with details. No schedules yet.
2. **Safety and recovery.** The safety limit, the Production and protected opt-in, approval and its
   revocation. Mirror deleting last, and the empty-source check. Preview. Retries with a fresh
   connection, saved progress and resuming for each kind (section 7). This lands before any
   schedule can write, and Run now benefits from it too.
3. **In-app scheduler.** The schedule editor with next-run preview, the timer, catch-up, pause, the
   needs-attention badge and notifications.
4. **Background runner and macOS.** The mode without a window, the lock, `--run-due-tasks`, and the
   `SMAppService` agent. Starts by checking that passwords can be read without a window.
5. **Windows.** The Task Scheduler entry with the battery settings off.
6. **Linux.** The systemd user timer.

## 12. Tests

- **Next-run rules:** every preset, month ends, and both daylight-saving changes. Needs a time-zone
  database in tests; add `chrono-tz` as a dev-dependency.
- **Catch-up:** a timer that fires after a long sleep runs a missed task once.
- **Queue:** a task due while it is still running is recorded as Skipped.
- **Safety:** the limit stops a Mirror run and an Import with "Clear target first" before any write,
  in Docker integration tests. Unit tests for the limit's rule: the 10% check, the jump against
  history (inactive before 3 successful runs), the 100-document floor, stopped runs not counting as
  history, and Run anyway. An empty source never empties the target. Mirror skips its delete pass
  after an error. A changed connection revokes approval. A Production target is refused without the
  opt-in.
- **Dropped connections**, in Docker integration tests: restart the MongoDB container in the middle
  of a Copy, a Sync and an Import, and pause it long enough to cause timeouts. Each run must resume
  and finish with exactly the expected target contents, with no duplicates and no missing
  documents, and its log must show the retries.
- **Retry rules:** which errors are retried, built from real driver errors; wait times stay within
  their caps, with a fixed random seed; a run stops retrying at 15 minutes and at its next
  scheduled run; sleep time isn't counted.
- **Outages:** one notification for several failed runs, one when the task works again, attention
  after three failures in a row, and a paused schedule after a sign-in failure.
- **History:** retention trimming, and a crash in the middle of a run resuming from its saved
  progress.
- **UI:** save a task from Transfer and from Compare, run it, and open its run details, with the
  gpui test harness.
- **Background runner:** exits at once when the app holds the lock. The platform entries get a
  manual check on each platform, since CI can't register them.

## 13. Risks and things not verified

- **Reading passwords without a window** is unverified on all three platforms. PR 4 starts with it.
- **macOS approval:** how the Login Items prompt behaves for an agent registered by an unsigned
  development build. Release builds are signed.
- **Remote speeds** for every kind of run, and Transfer speeds even locally.
- **Linux without systemd** (some distributions and containers) gets no background runner.
- **Sleeping or turned-off computers** can't run anything. A missed run happens at the next chance.
- **Copy in `_id` order** may be slower than today's copy on large collections. Not measured yet.
- **Imports of documents without `_id`** can't resume after a dropped connection. They stop as
  Partly done instead.
- **The driver's retryable error list** is copied, not shared. A driver upgrade could change it
  without the runner noticing.

## 14. Later

- Cron expressions for schedules that the presets can't express.
- Start and end dates for a schedule, as Studio 3T has.
- Tasks that run a Forge script or an aggregation and export its result.
- Chains: run one task after another succeeds.
- Running tasks through MCP, as proposals approved in the app like other agent writes.
- Exporting and importing tasks as files.
- Email or webhook notifications.
- One activity view for task runs and agent operations.

## Sources

- [DBeaver: Task scheduler](https://dbeaver.com/docs/dbeaver/Task-Scheduler/)
- [DBeaver: Troubleshooting task scheduler issues](https://dbeaver.com/docs/dbeaver/Troubleshooting-task-scheduler-issues/)
- [Studio 3T: Tasks for MongoDB](https://studio3t.com/knowledge-base/articles/automate-schedule-mongodb-tasks/)
- [launchd.plist(5)](https://keith.github.io/xcode-man-pages/launchd.plist.5.html), also `man launchd.plist`
- [systemd.timer](https://www.freedesktop.org/software/systemd/man/latest/systemd.timer.html)
- [TaskSettings.StartWhenAvailable](https://learn.microsoft.com/en-us/windows/win32/taskschd/tasksettings-startwhenavailable)
- [TaskSettings.DisallowStartIfOnBatteries](https://learn.microsoft.com/en-us/windows/win32/taskschd/tasksettings-disallowstartifonbatteries)
- [TaskSettings.StopIfGoingOnBatteries](https://learn.microsoft.com/en-us/windows/win32/taskschd/tasksettings-stopifgoingonbatteries)
- [Apple: Manage login items and background tasks](https://support.apple.com/guide/deployment/manage-login-items-background-tasks-mac-depdca572563/web)
- [SMAppService notes](https://theevilbit.github.io/posts/smappservice/)
- [planif](https://docs.rs/planif/latest/planif/)
- [MongoDB: Retryable Reads](https://www.mongodb.com/docs/manual/core/retryable-reads/)
- [MongoDB: Retryable Writes](https://www.mongodb.com/docs/manual/core/retryable-writes/)
- [AWS: Exponential Backoff And Jitter](https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/)
- [AWS: Retry with backoff pattern](https://docs.aws.amazon.com/prescriptive-guidance/latest/cloud-design-patterns/retry-backoff.html)
- [Azure: Transient fault handling](https://learn.microsoft.com/en-us/azure/architecture/best-practices/transient-faults)
- [Kubernetes: CronJob](https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/)
- [rclone: Usage and options](https://rclone.org/docs/) (`--max-delete`, `--delete-after`, `--dry-run`)
- [TaskSettings.ExecutionTimeLimit](https://learn.microsoft.com/en-us/windows/win32/taskschd/tasksettings-executiontimelimit)
- [Google SRE: Monitoring Distributed Systems](https://sre.google/sre-book/monitoring-distributed-systems/)
