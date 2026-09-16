# Accessibility QA

OpenMango supports keyboard activation for app-owned buttons with Return, Enter, and Space. Disabled controls are removed from the tab order, focus is visibly indicated, closing search returns focus to its owning view, and confirmation dialogs restore the control that opened them.

## VoiceOver test matrix

Run this checklist on every release candidate with VoiceOver enabled (`Command-F5`). Record the macOS version and result in the release notes.

### Sidebar

1. Focus the sidebar with `Command-0`.
2. Traverse connections, databases, and collections with the arrow keys.
3. Open and close search; confirm focus returns to the tree.
4. Open context menus and invoke connection, database, and collection actions without a pointer.

### Transfer

1. Open import, export, and copy tabs from the keyboard.
2. Traverse source, destination, format, query, and write-mode controls.
3. Verify destructive confirmations default to **Cancel**.
4. Start and cancel a transfer; verify progress, failures, skipped work, and cancellation remain visible.

### Action bar and dialogs

1. Open the action bar with `Command-K`, filter commands, and execute one with Return.
2. Verify displayed shortcuts match the active registered key bindings.
3. Open a confirmation, traverse both actions, cancel with Escape, and verify focus returns to the opener.
4. Repeat for file pickers and unsaved-change prompts.

### Documents and JSON editor

1. Traverse document rows and properties in tree and table modes.
2. Start, commit, and cancel an inline edit.
3. Open the detached JSON editor, use search, save, and close it from the keyboard.
4. Verify query and save errors remain visible and focus does not disappear after closing overlays.

### Aggregation

1. Open the Aggregation tab with `Command-Option-4`; focus lands in the stage list and shows a focus ring.
2. Move between stages with the arrow keys, skip or include one with Space, and open its editor with Return. Escape returns to the stage list.
3. Add a stage with `Command-Shift-N`, pick an operator by typing its name, and confirm with Return.
4. Delete a stage with Backspace, then undo it from the notification or with `Command-Z` in the stage list.
5. Switch between Stages and Text; verify a text error stays visible and blocks switching back until fixed.
6. Verify each stage row announces its number, operator, and skipped or failed state.

## Framework limitation

GPUI exposes accessibility roles, names, and selected or toggled state on any element with an id (`.role()`, `.aria_label()`, `.aria_selected()`), and gpui-kit controls accept `accessibility_label` or `aria_label`. It does not yet offer live regions, so status changes such as "Stage deleted" are not announced; they stay visible on screen instead.
