# GPUI Kit 0.6 migration

Branch: `migrate/gpui-kit-0.6`.

Implementation complete. App launch and user acceptance are pending.

## Plan

1. Replace the patched GPUI Component 0.5 dependency with the published GPUI Kit
   0.6.0 facade, including its matching runtime, components, icons, and editor
   language support. Update application initialization and imports.
2. Review the local vendor changes and migrate their application callers to the
   supported upstream APIs, preserving editing, navigation, and dialog behavior.
3. Replace the custom button renderer with upstream buttons and remove `vendor/`
   and the Cargo patch. Refresh the dependency lockfile.
4. Review the resulting changes and format the source. Leave app launch and test
   execution to the user; do not add tests as part of this migration.

## Sources

- [Getting started](https://gpui-kit.com/docs/getting-started)
- [0.6.0 release notes](https://github.com/longbridge/gpui-kit/releases/tag/v0.6.0)
- [Icons and assets](https://gpui-kit.com/docs/assets)
- [Tabs: complete component documentation](https://gpui-kit.com/component/tabs/)

## Changes

- Replaced the GPUI 0.2 / patched Component 0.5 dependencies with the published
  `gpui-kit` 0.6.0 facade. The lockfile now resolves the matching GPUI 0.3.4 runtime
  family and Tree-sitter 0.26.
- Removed all 211 files under `vendor/`, the Cargo patch, the custom button
  renderer, and the custom indentation code superseded by the native editor.
- Migrated code editors and completion providers to `EditorState` / `Editor`;
  ordinary fields retain `InputState` / `Input`. Read-only Forge output uses
  `Textarea` instead of reverting user edits in a change subscription.
- Replaced AI mention highlight patches with native text decorations. Enter
  submits the prompt; Shift+Enter adds a newline.
- Switched to native buttons with explicit small toolbar sizing, neutral tabs,
  `DataTable`, declarative dialog footers, and current chart APIs.
- Workspace tabs now occupy the native `TitleBar` beside the platform window
  controls. GPUI Kit's base tab components provide full-width selected capsules,
  equal tab widths with horizontal overflow, system-font labels, and shortcut
  hints from the active keymap. The duplicate tab row and empty titlebar padding
  have been removed; dragging the window uses the surrounding titlebar gutters.
- Tab content is constrained to the titlebar's available viewport. Native scroll
  requests track both tab identity and position, including replacement preview
  tabs, and remain pending until layout is ready to consume them.
- Kept the application's extra SVGs in its asset bundle, exposed through the
  toolkit's `icon_named!` macro. Updated bundled theme schema links to 0.6.0.
- Adapted focus, timers, async updates, and table column context menus to the
  new runtime. Lazy document expansion continues to use application metadata;
  settings shortcut capture cancels when focus leaves the keybindings panel.

Workspace tab styling follows the supplied Ghostty reference; secondary panels
retain the configurable native tab variants. Visual behavior still needs user review.

## Verification

- `cargo check --lib --bin openmango` passed.
- `cargo clippy --lib --bin openmango -- -D warnings` passed.
- `cargo fmt` and whitespace review completed.
- Bundled theme color keys match the 0.6.0 theme schema.
- No tests were added or run, and the app was not opened. Tests specific to the
  deleted button renderer and indentation helper were removed with that code.
