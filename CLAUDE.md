# OpenMango

GPU-accelerated MongoDB GUI client built with Rust and GPUI (like Studio 3T).

## Quick Start

**IMPORTANT: Always use `just` commands, never use `cargo` directly.**

```bash
just dev      # Run in development mode
just debug    # Run with debug logging
just check    # Type check without building
just lint     # Run clippy (with -D warnings)
just fmt      # Format code
just test     # Run tests
just release  # Build release with mimalloc
just precommit # Run all checks before commit
```

## Tech Stack

- **UI Framework**: [GPUI](https://www.gpui.rs/) - GPU-accelerated UI from Zed
- **Components**: [gpui-component](https://github.com/longbridge/gpui-component) v0.5 - 60+ UI components
- **Routing**: [gpui-router](https://github.com/justjavac/gpui-router) v0.3 - React Router-like navigation
- **Database**: mongodb driver v2.8
- **Async**: tokio

## Project Structure

```
src/
├── main.rs           # Entry point, window setup
├── theme.rs          # Custom colors, spacing, sizing, typography, fonts, borders
├── components/       # Reusable UI components
├── views/            # Screen/page components
│   └── documents/    # Document tree + editor view (view-model driven)
├── models/           # Data structures
├── connection/       # MongoDB connection manager
├── bson/             # BSON helpers and path utilities
└── state/            # App state + commands + events
    ├── app_state/    # Core state modules
    ├── commands.rs   # Async operations + event emission
    ├── events.rs     # AppEvent payloads
    └── status.rs     # Status messages
```

## GPUI Patterns

### Basic Component
```rust
use gpui::*;

struct MyComponent;

impl Render for MyComponent {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .child("Hello")
    }
}
```

### Using gpui-component
```rust
use gpui_component::button::Button;

Button::new("submit")
    .primary()
    .label("Submit")
    .on_click(|_, _, _| println!("clicked"))
```

### Routing with gpui-router
```rust
use gpui_router::{Router, Route};

Router::new()
    .route("/", home_view)
    .route("/settings", settings_view)
```

## Resources

### Local References (refs/)
The `refs/` directory contains local documentation and examples:

```
refs/
├── gpui/
│   ├── docs/           # GPUI documentation
│   └── examples/       # GPUI examples (animation, data_table, drag_drop, input, etc.)
└── gpui-component/
    ├── docs/           # Component documentation
    ├── examples/       # Component examples (dialog_overlay, input, hello_world, etc.)
    └── themes/         # Theme configurations
```

**Use these refs first** when looking for patterns or examples.

### Online Documentation
- GPUI Official: https://www.gpui.rs/
- GPUI Book: https://github.com/MatinAniss/gpui-book
- Component Docs: https://longbridge.github.io/gpui-component/

### Libraries
- gpui-component: https://github.com/longbridge/gpui-component
- gpui-router: https://github.com/justjavac/gpui-router
- gpui-form (future): https://github.com/stayhydated/gpui-form

## Theme System

Colors use gpui-component's theme system. Themes are defined in `themes/*.json` (Vercel Dark, Darcula Dark) and embedded at compile time. Runtime switching via `theme::apply_theme()` in Settings.

### Accessing theme colors
```rust
use gpui_component::ActiveTheme as _;

// In any render method or function with cx: &App
cx.theme().background      // Main background
cx.theme().foreground      // Primary text
cx.theme().primary         // Accent color (desaturated green)
cx.theme().sidebar         // Sidebar/panel background
cx.theme().border          // Standard borders
cx.theme().muted_foreground // Muted text
// See gpui-component docs for all available tokens
```

### Custom colors (theme-aware)
These read from the active theme's base colors (take `cx: &App`, return `Hsla`):
- `colors::syntax_*(cx)` — BSON syntax highlighting (key, string, number, boolean, null, object_id, date, comment)
- `colors::bg_dirty(cx)` — Dirty document highlight (warning with alpha)
- `colors::bg_error(cx)` — Error background (danger with alpha)

### Theme switching
- `theme::load_theme_config(theme_id)` — load a `ThemeConfig` by ID
- `theme::apply_theme(app_theme, window, cx)` — switch theme at runtime
- Theme JSONs are embedded via `include_str!` in `THEME_SOURCES`

### Rules
- **Never** use hardcoded hex colors in render methods — use `cx.theme()` tokens
- For BSON syntax highlighting, use `crate::theme::colors::syntax_*(cx)` (returns `Hsla`)
- Non-color design tokens (spacing, sizing, typography, fonts, borders) remain in `theme.rs`

## Development Notes

- Components go in `components/`, full-page views go in `views/`
- Async MongoDB ops must go through `state/commands.rs` (AppCommands)
- State management lives in `state/app_state/` with a session-per-tab model

## Architecture (Must-Follow)

### Session-per-tab
- Each open tab has a `SessionState` containing:
  - `SessionData` (documents, pagination, loading, request_id)
  - `SessionViewState` (selection, expanded nodes, drafts, dirty)
- Sessions are stored in `SessionStore` (AppState owns it).
- Mutate session state through AppState helpers (e.g. select node, expand, drafts, paging).

### Tabs
- Active tab is a single enum: `ActiveTab::{None, Index, Preview}`.
- Preview vs permanent tabs are handled in `state/app_state/tabs.rs`.

### Commands + Events
- All async operations emit `AppEvent` with payloads (errors, totals, etc.).
- If you add a new AppEvent, update `state/app_state/status.rs` to surface status messages.
- AppRoot reads `state.status_message` for status bar messaging.

### Documents View
- Tree/inline editing logic is centralized in `views/documents/view_model.rs`.
- UI layers should not manually mutate session internals.

## Feature Roadmap

See `docs/features.md` for the complete feature specification with priority levels (P0-P3).
