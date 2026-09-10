# Native connection workflow research

Reviewed 2026-09-10. Research and implementation notes for `redesign/connection-workflow`; no dependencies added.
Sources: current primary Apple, MongoDB Compass, and JetBrains documentation; installed GPUI Kit/Base/Component 0.6.0 source.
Reference clone: [`refs/gpui-kit` at 36b51819deb52c947a79f8de29e0e9175eda7464](https://github.com/longbridge/gpui-kit/tree/36b51819deb52c947a79f8de29e0e9175eda7464).
Recommendations below are OpenMango design decisions inferred from those sources, not claims that another product implements every detail.

## Implemented workflow

- The sidebar shows every saved connection. **Connections** opens the manager, **New** starts a draft, and each connection has its own Connect and actions menu. The global globe dropdown and settings gear are removed.
- The manager uses GPUI Kit's searchable List, native form fields, tabs, switches, buttons, and button groups. URI and optional name come first; Auth, TLS, Network, Advanced, and Access hold the remaining settings.
- **Test** validates the current draft and is optional. Results belong to the full draft snapshot, including credentials and transport settings, and cannot overwrite a newer test or another connection's status.
- **Save** persists settings without connecting. Existing sessions retain their original connection settings until **Reconnect**; the editor indicates when saved changes require it. **Save & Connect / Reconnect** waits for persistence and rechecks unsaved database work before reconnecting.
- Failed persistence preserves both the editor draft and any existing session. URI editing preserves unknown options, encoded credentials, TLS aliases, and in-progress query delimiters. Credentials are hidden even when a pasted URI is incomplete.
- Opening the manager no longer reads its entity while it is under construction or update, addressing the reported New Connection crash.

Headless checks cover opening/reusing the manager, unsaved-change cancellation, native list confirmation, URI typing/pasting, persistence success/failure, active-session preservation, stale test results, and rendering each tab at normal and narrow sizes. Desktop visual review remains manual: open New, paste a URI, Save, connect from its sidebar row, edit and Save while connected, then Reconnect.

## Recommended direction

Use one clearly labeled **Connections…** entry to a saved-connection list and detail editor.
Put **New connection** inside that surface; keep direct Connect/Disconnect commands with the connection they affect.
Remove the competing global globe dropdown and gear route; menu/keyboard equivalents can invoke the same entry point.
The application sidebar remains the place to navigate active databases, while the connection surface owns configuration.
This simplifies ownership without making double-click the only way to connect or edit.

Apple describes toolbars as convenient access to frequently used commands; the inference here is to expose distinct tasks, not several ambiguous routes to the same task.
DataGrip provides a concrete list/detail precedent: selecting a saved data source displays its settings in the adjacent pane.
[Apple: Toolbars](https://developer.apple.com/design/human-interface-guidelines/toolbars), [DataGrip: Data Sources and Drivers](https://www.jetbrains.com/help/datagrip/data-sources-and-drivers-dialog.html)

## Saved configuration and active session are different things

| Concept | Meaning in OpenMango | Visible treatment |
| --- | --- | --- |
| Saved connection | Persisted name, endpoint, credentials reference, and options | Searchable row; editable without connecting |
| Draft | Unsaved edits to a new or saved connection | Unsaved marker; preserved while Test runs |
| Test result | Result for one exact configuration snapshot | Inline success/error; invalidated when connection-affecting fields change |
| Active session | A live connection established from a configuration snapshot | Connected/Connecting/Disconnected state beside its name |

DataGrip explicitly separates a data source configuration from a session that wraps a live connection.
Compass separately documents saved/favorite connections, currently active connections, connection actions, and connected-row metadata editing.
Those distinctions support clear labels; they do not require exposing technical session machinery to the user.
[DataGrip: Connection to a database](https://www.jetbrains.com/help/datagrip/connecting-to-a-database.html), [Compass: Connections Sidebar](https://www.mongodb.com/docs/compass/connect/connections/)

## One predictable editing flow

1. Open Connections: show saved rows and the selected connection's details; opening or selecting a row performs no network operation.
2. New connection: create a local draft, focus the connection-string field, and retain it until saved or explicitly discarded.
3. Edit: select an existing row, then modify the adjacent form; leave the saved record unchanged until Save.
4. Test: test the current draft as entered, without saving it or replacing the active session.
5. Save: persist validated settings and retain the editor/selection; show a quiet saved state.
6. Connect: use the selected saved configuration; for a new or dirty draft, label the combined action **Save & Connect**.
7. A successful connection reveals/focuses the corresponding database navigation; failure leaves the form and its draft available for correction.

Compass documents Save, Connect, and Save & Connect as separate outcomes, including an unsaved one-off Connect path.
For OpenMango, the primary recommendation is saved connections plus an explicitly named combined action; one-off connections need not become a new feature in this redesign.
DataGrip documents Test Connection as a distinct check of connection settings and communication.
[Compass: Connect](https://www.mongodb.com/docs/compass/connect/), [DataGrip: connection settings](https://www.jetbrains.com/help/datagrip/data-sources-and-drivers-dialog.html)

## Form composition and action hierarchy

Keep name and connection string near the top, with an endpoint summary that excludes credentials.
Offer paste-first URI entry; expose Authentication as a clear nearby section rather than requiring users to edit a password inside a URI.
Keep TLS, Network/SSH, and other advanced settings in progressive sections or a short native tab strip.
The initial section should answer what to connect to and how to authenticate; optional configuration should not dominate first use.
Keep fields left-aligned at a readable width beside the connection list, with one scrollable form region and a stable action footer.

Compass supports pasting a connection string and exposes Authentication, TLS/SSL, and SSH through advanced options.
Its documentation warns that editing a connection string can expose credentials and points to Authentication fields for password editing.
Apple's data-entry guidance emphasizes reducing the amount people must supply and preventing mistakes.
[Compass: Connect](https://www.mongodb.com/docs/compass/connect/), [Apple: Entering data](https://developer.apple.com/design/human-interface-guidelines/entering-data)

| State | Secondary actions | Primary action |
| --- | --- | --- |
| New draft | Test Connection; Save | Save & Connect |
| Saved, unchanged, disconnected | Test Connection | Connect |
| Saved, dirty, disconnected | Test Connection; Save | Save & Connect |
| Saved, unchanged, connected | Test Connection; Disconnect | Open databases |
| Connected configuration with edits | Test Connection | Save; explicit reconnect only if requested |

Avoid two equally prominent Connect buttons on the same surface. A list-row quick action and the editor footer can share one command implementation.
Do not advertise test success as a prerequisite for Save or Connect: it is useful feedback, not a mandatory extra click.

## Dirty navigation and editing connected configurations

Switching rows, starting another New draft, or closing the editor with unsaved changes uses one **Save / Discard / Cancel** decision.
Cancel keeps the current draft and focus; Save resumes the original navigation only after persistence succeeds.
Keep routine validation and successful tests inline. Reserve alerts for a pending decision that would otherwise lose work.
Apple recommends using alerts sparingly and avoiding them for information alone; the three-way dirty decision is this design's application of that guidance.
[Apple: Alerts](https://developer.apple.com/design/human-interface-guidelines/alerts)

Editing and saving an endpoint, authentication, TLS, or network option must not silently mutate or replace a live session.
Show **Saved changes apply on the next connection** while the active session continues with its original configuration.
Metadata-only changes such as name/color may update presentation immediately; distinguish them from connection-affecting edits.
An explicit Reconnect operation can apply the new settings through the application's existing session/dirty-work guards.
Compass provides a useful conservative precedent: its documented connected edits are name, color, and favorite status.
[Compass: edit connected connections](https://www.mongodb.com/docs/compass/connect/connections/#edit-connections)

## Testing, errors, and asynchronous work

Test Connection shows **Testing…** on the initiating button and a local pending indicator, preserving all entered values.
On success, show a brief inline result; on failure, show a readable category, concise message, and expandable technical detail.
Connection failures should remain reviewable next to the relevant configuration rather than existing only in a transient toast.
Compass exposes a row error indicator and additional error review; OpenMango can retain the detail directly in its editor.
[Compass: connection errors](https://www.mongodb.com/docs/compass/connect/connections/#connect-to-mongodb)

Associate every request with the draft identity and revision that started it; late results must not attach to another connection or overwrite a newer test.
Changing connection-affecting fields invalidates the displayed test result; changing only the display name need not.
During Save & Connect, persist first. If connecting fails afterward, report **Saved, but could not connect** rather than implying the save failed.
Keep retry explicit. Show Cancel only when the operation actually supports cancellation, and keep request completion separate from view lifetime.

## Native GPUI 0.6.0 implementation map

| Need | Verified shipped API | Important boundary |
| --- | --- | --- |
| Saved connection list/search | `ListState::new(delegate, window, cx).searchable(true)`; `List::new(&state)` | `ListDelegate` owns search, rows, selection, and confirm behavior |
| Preserve selected row | `set_selected_index`; `scroll_to_item(ix, ScrollStrategy::Nearest, window, cx)` | Setting selection does not automatically scroll; use stable connection IDs |
| Name/URI/password fields | Retained `InputState`; `Input::new(&state)`; `.masked(true)` on password state | Labels and masking do not implement URI parsing or credential storage |
| Authentication/options | `SelectState::new(delegate, selected_index, window, cx)` | Store typed values; display labels are not configuration identity |
| Sections | `TabBar::new(id).selected_index(index).on_click(...)`; `Tab::new().label(...)` | TabBar callback receives `&usize`; it replaces child click callbacks |
| Progressive options | `Collapsible::new().open(open).content(...)` | Application owns the open bool and an accessible toggle button/header |
| Field labels/help | `v_form()` / `h_form()` plus `field().label(...).description(...)` | Form is layout, not validation or automatic submission |
| Pending/error state | `Button::loading(bool)`; `Alert::error(id, message)` / `Alert::success(...)` | Guard duplicate operations in state; spinner styling is not an async controller |

[Published List](https://docs.rs/crate/gpui-component/0.6.0/source/src/list/list.rs), [ListDelegate](https://docs.rs/crate/gpui-component/0.6.0/source/src/list/delegate.rs), [Input state](https://docs.rs/crate/gpui-base/0.6.0/source/src/input/base/state.rs), [Select](https://docs.rs/crate/gpui-component/0.6.0/source/src/select.rs)
[Published TabBar](https://docs.rs/crate/gpui-component/0.6.0/source/src/tab/tab_bar.rs), [Collapsible](https://docs.rs/crate/gpui-component/0.6.0/source/src/collapsible.rs), [Form](https://docs.rs/crate/gpui-component/0.6.0/source/src/form/form.rs), [Alert](https://docs.rs/crate/gpui-component/0.6.0/source/src/alert.rs), [Button](https://docs.rs/crate/gpui-component/0.6.0/source/src/button/button.rs)

## API and integration traps

- Current clone/docs expose `Form::new()`, `.label_layout(...)`, and `.footer(...)`; published 0.6.0 does not. Use `Form::vertical()` / `horizontal()` or helper constructors, with an ordinary native footer sibling.
- Retain field entities across tab/disclosure changes. `InputState::set_value` resets selection, scroll, and undo, so do not refill fields every render.
- A generic `InputState::validate` boolean predicate is not a connection-validation workflow. Preserve incomplete URI/password drafts and present actionable errors at the appropriate boundary.
- Make the editor's focus scope explicit. Enter in a multiline field, select popup, or dirty dialog must not accidentally trigger Connect.
- Snapshot status needed by row/render helpers before rendering child controls. Do not synchronously re-read a manager entity while it is already being updated.
- Keep existing native sizing, typography, buttons, and input appearance; Base is useful for interaction ownership, not a reason to introduce a second visual language.

[Current Form source](https://github.com/longbridge/gpui-kit/blob/36b51819deb52c947a79f8de29e0e9175eda7464/crates/component/src/form/form.rs#L29), [published input mutation behavior](https://docs.rs/crate/gpui-base/0.6.0/source/src/input/base/state.rs)

This report verifies source/API availability and documents design recommendations; it does not claim rendered UX validation or implementation completeness.
Apple's JavaScript-rendered pages were read through their official documentation JSON endpoints; linked HIG pages remain the human-readable references.
