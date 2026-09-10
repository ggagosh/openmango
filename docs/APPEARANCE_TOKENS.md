# Shared application corner radii

OpenMango uses one radius scale across built-in color themes, GPUI Base/Components, and custom UI. The values live in `src/theme.rs::borders`.

| Role | Token | Radius |
| --- | --- | --- |
| Small indicators, compact badges, chart marks | `borders::radius_xs()` | 3 px, derived from the control radius |
| Buttons, inputs, tabs, rows, menus, small controls | `borders::radius_sm()` | 6 px |
| Panels, dialogs, cards, larger surfaces | `borders::radius_md()` | 8 px |

`apply_design_tokens` sets the native toolkit's `Theme.radius` and `Theme.radius_lg` from these same values. It runs after loading the initial theme and after every color-theme change. GPUI's Base token projection receives the same scale. Color-theme JSON files do not define their own geometry.

The existing Islands helpers delegate to these tokens. Custom UI uses the role functions instead of numeric `.rounded(px(...))` calls or rem-based `.rounded_sm()`/`.rounded_md()` shortcuts. Workspace tabs use the control radius; their counters use the compact badge radius.

Circles such as connection/status dots and color swatches retain `rounded_full()`. Slim drag/scroll indicators can also remain fully rounded. These are geometric shapes rather than competing control-radius settings. Native window frames remain platform-owned.

When adding a control, prefer the themed GPUI component. When composing a custom surface, choose a role from the table. Avoid changing the radius simply to distinguish colors or selected states; use the existing theme's foreground, background, and focus treatments.

Verification covers every bundled color theme and confirms matching radius tokens in the Component and Base layers. The app-wide source audit also checks for remaining numeric/rem-based radius overrides outside this token module.
