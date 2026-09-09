# GPUI Kit 0.6 migration

Branch: `migrate/gpui-kit-0.6`.

The application uses the published toolkit. Final desktop acceptance and updated screenshots remain part of draft-PR review.

## Scope

1. Replace the patched GPUI Component 0.5 dependency with the published GPUI Kit
   0.6.0 facade, including its matching runtime, components, icons, and editor
   language support. Update application initialization and imports.
2. Review the local vendor changes and migrate their application callers to the
   supported upstream APIs, preserving editing, navigation, and dialog behavior.
3. Replace the custom button renderer with upstream buttons and remove `vendor/`
   and the Cargo patch. Refresh the dependency lockfile.
4. Validate the migrated application with formatting, Clippy, Rust tests, and
   headless native UI checks. Keep desktop acceptance as a separate manual check.

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
  controls. GPUI Kit's base tab components provide full-width selected surfaces,
  equal tab widths with horizontal overflow, app-font labels, and shortcut
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
- Native and custom UI now share the application radius scale, including all
  built-in color themes. See [appearance tokens](APPEARANCE_TOKENS.md).

Workspace tab styling follows the supplied Ghostty reference; secondary panels
retain the configurable native tab variants. Visual behavior still needs user review.

## Verification

- `just fmt-check` and `just lint` passed.
- Headless GPUI tests cover query-input focus/caret/undo, control alignment,
  group selection and collapse, and radius consistency across color themes.
- The complete Rust suite is run with `just test` before PR creation; local Docker
  integration requires `DOCKER_HOST` to point to the active Docker context.
- Bundled theme color keys match the 0.6.0 theme schema.
- Desktop screenshot capture is pending; automated UI checks run headlessly.
