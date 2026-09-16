# Error handling plan

Scope: every place an error reaches the user, across all windows and views.

Inputs: an inventory of every error surface in `src/` (about 120 `StatusMessage::error` call sites,
15 event templates, and 30+ inline renderings), the `better-ui` and `emil-design-eng` skills, and a
survey of Compass, mongosh, DataGrip, pgAdmin, Sequel Ace, Postico, TablePlus, RedisInsight, VS Code,
Zed, Raycast, Vercel Geist, GitHub Desktop, Postman, Apple HIG, GNOME HIG, Fluent, Material,
Atlassian, Carbon, JetBrains and NN/g guidance.

## 1. What is wrong today

The screenshot (aggregation `$group` without `_id`) shows most of it in one frame:

```
Aggregation failed: MongoDB error: Kind: Command failed: Error code 15955 (Location15955): a group s…
Stage 1 failed. Fix it and run again.
MongoDB error: Kind: Command failed: Error code 15955 (Location15955): a group specification must
include an _id, labels: {}, source: None, server response: Some(RawDocumentBuf { data: "ea00…" })
```

| # | Sev | Problem | Evidence |
| --- | --- | --- | --- |
| 1 | HIGH | Error text is invisible. `danger_foreground` is the text color for a *filled* danger button (#211209 in Mango Dark, #ffffff in Mango Light), but 13 surfaces use it as text on normal backgrounds. | `views/documents/view.rs:970` (query error card), `header/stats_panel.rs:43`, `schema_view.rs:81`, `header/filter_bar.rs:306,326`, `forge/output/results.rs:75,84`, `databases.rs:232,326`, `query_library.rs:1477`, `settings/keybindings.rs:479`, `settings.rs:2376`, `connection_manager/tabs.rs:534` |
| 2 | HIGH | Raw driver output reaches users. `mongodb::Error`'s Display prints `Kind: …, labels: {}, source: None, server response: Some(RawDocumentBuf { data: "<hex>" })`; write errors print Rust `Debug` structs. Only the message the server wrote matters. | `error.rs:6` wraps with `MongoDB error: {0}`; `mongodb-3.5.1/src/error.rs:61-67, 706, 878-889` |
| 3 | HIGH | Prefix noise hides the cause. `Aggregation failed: MongoDB error: Kind: Command failed: Error code 15955 (Location15955): ` is 91 characters, and the banner shows 100. `Parse error:` is prepended to 168 non-parse messages (conflicts, cancels, SSH auth, URI). Connection errors read `Connection failed: Parse error: MongoDB error: Kind: Server selection timeout: …`. | `error.rs:24`, `connection/manager.rs:618-642`, `components/content/shell.rs:29-42` |
| 4 | HIGH | The same error shows in 3–4 places. Aggregation: banner + results + stage editor + row. Query: banner + card + filter bar. Connect: banner + manager footer + Details dialog. Indexes, stats, schema, create index, bulk update, JSON editor: inline + banner. | inventory §c |
| 5 | HIGH | Errors disappear before they're read. The single banner slot is cleared on any tab switch and overwritten by any later info message; agent activity overwrites its own error with "Loaded N databases" within milliseconds. Error toasts auto-hide after 5 s. | `tabs/model.rs:95-99`, `app_state/mod.rs:349-355`, `commands/actions.rs:141`, `gpui-component notification.rs:841` |
| 6 | HIGH | Some failures show nothing useful. Explain never renders `explain.error` (modal says "No explain summary yet."); "Connection not found" sets no message; create/rename collection dialogs close before the operation fails. | `explain/mod.rs:615`, `commands/connections.rs:31-37`, `app/dialogs.rs:82-84` |
| 7 | HIGH | Crash: model-list errors are cut with `&msg[..57]`, which panics on multi-byte UTF-8. | `views/ai.rs:1120`, `views/settings.rs:2266` |
| 8 | MED | Errors name no way forward. Save conflict says "reload before saving" without a Reload button; banner-only errors (save, delete, drop, transfer, sidebar actions) offer nothing; copy says "Review it again" with no button. | inventory §d |
| 9 | MED | Long red monospace body text. gpui `Alert` colors the whole body `danger`; stage editor alert grows until it covers the editor. | `aggregation/stage_editor.rs:142-170`, `gpui-component alert.rs` (fg = danger) |
| 10 | MED | Seven containers, three text colors, two sizes, four tint strengths for the same job; only two surfaces can copy details; connection Details uses the UI font and has no Copy. | inventory §d |
| 11 | MED | One-slot `StatusMessage` with only Info/Error: no id, no timestamp, no details, no history. Two startup failures show only the first (`.or()`). | `state/status.rs:3-23`, `app_state/mod.rs:236-239` |
| 12 | LOW | Stringly-typed logic: transfer "cancelled" = message contains "cancel"; connection hints match English text. | `transfer/progress_panel.rs:59-61`, `connection/manager.rs:644-662` |

## 2. What other apps do well

- **One home per error, chosen by scope** (Compass, NN/g, Fluent, JetBrains). Validation on the field, operation errors in the panel that ran them, dialog errors above the dialog buttons, connection loss as persistent state. Other places point to the home: Compass shows "An error occurred on Stage 3" on later stages instead of repeating it.
- **Human first line, raw message second, code as a chip** (mongosh `MongoServerError[Location15955]: …`, Geist two-sentence rule, Atlassian). Never a bare "Error" title.
- **Details behind a disclosure with Copy** (Compass "View error details" JSON modal capped at 60vh, Raycast "Copy Logs", RedisInsight drops bodies over 400 chars into a log download).
- **At most two verb-first actions** (Retry, Reconnect, Go to stage, Reload document, Increase time limit). JetBrains: error notifications always carry a fix, or explain one.
- **Never auto-dismiss errors that need action; keep a history** so transient toasts are safe (VS Code notification center, JetBrains Notifications, Postman Console).
- **Dedupe and morph** (VS Code same-id replace, Zed one slot per error type, Compass connection toast turns progress → failure in place).
- **Known server errors get a specific explanation** (Compass maxTimeMS hint, GitHub Desktop maps git errors to causes and fixes).
- **Tone:** red icon and title, neutral body; monospace only for payloads; no shake.

## 3. Target model

### 3.1 `ErrorReport`: one structure for every error

```rust
pub struct ErrorReport {
    pub title: String,            // "Couldn't run stage 1", "Couldn't connect to Local"
    pub message: String,          // "A $group stage needs an _id field."  (human, one or two sentences)
    pub server_message: Option<String>, // "a group specification must include an _id"
    pub code: Option<i32>,        // 15955
    pub code_name: Option<String>,// "Location15955"
    pub details: Option<String>,  // pretty JSON of errInfo / server reply, hints, connection trace
    pub kind: ErrorKind,          // Validation, Server, Conflict, Connection, Auth, Timeout, Io, Cancelled
    pub retryable: bool,
}
```

- `ErrorReport::from_mongo(&mongodb::error::Error)` matches `ErrorKind` and codes instead of strings:
  - `Command(CommandError)` → `server_message = message`, code and codeName kept.
  - `Write(WriteError)`, `InsertMany`, `BulkWrite` → per-document messages, `details` from `errInfo`.
  - `ServerSelection` / `Io` / `DnsResolve` / `ProxyConnect` → Connection kind, topology text only in `details`.
  - `Authentication` → Auth; `InvalidTlsConfig` → Connection with a TLS hint.
- A small code table turns common codes into a human `message`, keeping the server text:

| Code | Human message |
| --- | --- |
| 11000 DuplicateKey | "A document with this {field} already exists." (from `keyValue`) |
| 121 DocumentValidationFailure | "The document doesn't match the collection's validation rules." (details: errInfo) |
| 13 Unauthorized / 18 AuthenticationFailed | "This user can't run that command." / "Wrong username or password, or wrong auth database." |
| 50 MaxTimeMSExpired | "The operation hit its time limit." |
| 26 NamespaceNotFound / 48 NamespaceExists | "The collection doesn't exist." / "A collection with this name already exists." |
| 85, 86 Index option/key conflicts | "An index with these keys or this name already exists with different options." |
| 40324 / 40323 | "Unknown pipeline stage {name}." |
| 15955 | "A $group stage needs an _id field." |
| 2 BadValue, 9 FailedToParse, 14 TypeMismatch | server text as the message |

- `Display` for `crate::error::Error` becomes the human line (no `MongoDB error: Kind:` prefixes); `Error::Parse(String)` stops being the catch-all: add `Error::Message(String)` for plain messages and `Error::Cancelled`, and keep `Parse` for real parse failures.
- **Copy text** (one format everywhere):
  ```
  Couldn't run stage 1: A $group stage needs an _id field.
  MongoServerError[Location15955]: a group specification must include an _id
  {details JSON}
  ```

### 3.2 Where each error lives (one home)

| Error | Home | Also | Never |
| --- | --- | --- | --- |
| Invalid input (filter, stage JSON, field value, form field) | Inline under the input, warning tone | editor border | banner, toast |
| Operation in a visible panel (query, aggregation, indexes, stats, schema, explain, transfer) | That panel's error callout | a pointer where it helps ("Stage 1 failed" on the row) | banner |
| Dialog action (create index, bulk update, connection test, import) | Inline above the dialog buttons; dialog stays open | — | banner, closing the dialog |
| Action with no visible home (sidebar create/drop/rename, clipboard, file write, background agent op) | Error toast with Copy details (+ Retry when retryable) | history | banner |
| Connection lost / connect failed | Sidebar node error state + tab callout with Reconnect | toast when it happens | repeated banners |
| App-level (startup load failures, update failure) | Toast that stays until dismissed | history | — |

Every error also goes to an **error history** (status bar "2 errors" button → list with time, scope,
Copy), so toasts can auto-hide safely and nothing is lost on tab switch.

### 3.3 Components

- **`ErrorCallout`** (new, `src/components/error_callout.rs`), replacing the shell banner, gpui `Alert`
  for errors, and every hand-built tinted box:
  - Icon + title in `danger` (warning tone uses `warning` for validation); message in `foreground`,
    UI font, `text_sm`, wraps up to 3 lines then "Show more".
  - Muted chip `Location15955 · 15955` when there's a code.
  - "Details" disclosure: `details` in mono `text_xs` on a muted surface, max height 240 px, scrolls.
  - Actions row: up to two verb-first buttons + "Copy details". Optional close.
  - Sizes: `inline` (single line, for rows and editors), `panel` (default), `dialog`.
- **Error toasts** use gpui `Notification` with a dedupe id per (scope, code), `Copy details`
  action, and `autohide(false)` when there's a fix action; 8 s otherwise, paused on hover.
- **`StatusMessage`** keeps Info for the status bar; errors become `ErrorReport` pushed to history
  and, when they have no home, to a toast.

### 3.4 Visual and motion rules (better-ui, emil-design-eng)

| Before | After | Why |
| --- | --- | --- |
| Whole error body in `danger`, sometimes mono | Icon + title `danger`, body `foreground`, mono only in Details | Long red text is hard to read; color carries severity once |
| `danger_foreground` as text on normal surfaces | `danger` for text on normal surfaces; `danger_foreground` only on filled danger | The token is for filled buttons; on cards it's invisible |
| Same error in banner + panel + editor + row | One callout at the home; rows show "Failed" pointer | Duplicates compete and push content away |
| Stage-editor alert grows over the editor | Inline size, one line + "Details", editor keeps its height | The input you need to fix must stay visible |
| Error toast auto-hides after 5 s | Stays until dismissed when it has an action; otherwise 8 s, paused on hover; always in history | Errors need reading time and a record |
| Banner cleared on tab switch | Errors live at their home or in history | Navigation shouldn't erase evidence |
| Callout appears with no transition | Callout appears instantly (user-triggered, frequent) | Emil: no animation on frequent, keyboard-driven actions |
| Toast enter | gpui-kit default slide + fade, respects reduced motion | Occasional, benefits from spatial cue |
| Details disclosure | Instant, no height animation | Frequent, and height animation causes layout jank |
| — | No shake on failures | Motion must not be the only signal; oscillation is discouraged |

## 4. Work, by area

1. **Foundation** — `ErrorReport` + `from_mongo` + code table + copy format (unit tests per kind/code);
   `Error::Message`/`Cancelled`; human `Display`; `annotate_connection_error` moves hints/trace into
   `details`; fix the `&msg[..57]` panics.
2. **Components** — `ErrorCallout`; error toast helper; error history (state + status bar button + list).
3. **Aggregation** — one callout in results (panel size), stage editor inline one-liner that doesn't
   cover the editor, row "Failed" pointer only, no banner; stages after the failing one say
   "Blocked by stage N".
4. **Documents** — query card uses the callout (Retry, Copy); filter bar shows only the inline parse
   error; save conflict gets **Reload document**; insert/delete/update errors go to a toast with
   Copy (and Reload/Retry where they apply); inline value errors stay inline only.
5. **Indexes, stats, schema, explain** — callouts with Retry; explain renders its error in the modal;
   no banners.
6. **Dialogs** — create index, bulk/property update, create/rename database and collection (stay open
   until success), query library import, keybindings: error inline above the buttons.
7. **Connections** — manager shows one callout (title + human cause + Details with hint/trace +
   Copy); connect failure marks the sidebar node and shows a tab callout with Reconnect; one toast when
   it happens; "Connection not found" gets a message.
8. **Transfer, Forge, AI, settings, updater, JSON editor** — callouts instead of custom boxes;
   transfer detail in `foreground` not muted; cancelled via `Error::Cancelled`, not string matching;
   JSON editor errors stay in its window only.
9. **Remove the shell error banner** and the tab-switch clearing once every caller has a home;
   startup failures become persistent toasts.
10. **Guard** — a unit test that fails if `danger_foreground` is used as a text color outside filled
    danger surfaces (grep-based, like the existing theme contrast test).

## 5. Decisions

1. **No global banner.** Connection failures mark the connection in the sidebar (warning icon, reason
   on hover) and raise one notification with Reconnect. Operation errors stay in their panel.
2. **Ask AI** appears on error callouts only when the AI assistant is set up; it opens the AI panel
   with the error, its code, and the query or pipeline that caused it.
3. **Error history is session-only.** The status bar shows "N errors"; the list clears on restart.
