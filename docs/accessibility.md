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

## Framework limitation

The current GPUI release does not expose macOS accessibility-role/name/live-region APIs for custom elements. OpenMango therefore cannot yet publish full native VoiceOver semantics for every custom control. Keyboard behavior, visible labels, tooltips, focus order, and focus restoration are covered in-app; native semantic announcements must be completed when GPUI exposes that API.
