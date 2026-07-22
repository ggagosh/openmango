# Product

## Register

product

## Users

MongoDB power users, application developers, and small engineering teams doing day-to-day database work on macOS. They need to inspect, query, edit, analyze, and move data quickly while retaining clear context about the active connection and the consequences of each operation.

## Product Purpose

OpenMango is a fast, native, keyboard-friendly MongoDB workbench. It brings document browsing, aggregation, schema analysis, explain plans, Forge, and guarded data transfer into one coherent desktop workflow without Electron or web views. Success means experienced users can work quickly while the product makes connection identity, persistence, privacy, and destructive effects explicit.

## Brand Personality

Fast, calm, trustworthy. Interaction patterns should feel familiar to users of DataGrip and VS Code: dense when useful, predictable under the keyboard, and restrained rather than decorative.

## Anti-references

Avoid decorative SaaS-dashboard styling, modal-heavy workflows, surprising custom controls, ornamental motion, excessive cards, and visual noise that competes with data. Do not trade safety or clarity for novelty, and do not hide important scope or destructive effects behind color alone.

## Design Principles

1. Prefer earned familiarity over invention: standard desktop-tool patterns should disappear into the task.
2. Put context at the point of action: connection, environment, namespace, and write impact must be visible where decisions are made.
3. Make safety explicit and recoverable: validate before writes, confirm destructive work, preserve user input, and report failures clearly.
4. Maintain keyboard and pointer parity: every core workflow must be discoverable, focusable, and operable without a mouse.
5. Keep private work local: never persist credentials or resolved secrets, and make portable data formats inspectable and versioned.

## Accessibility & Inclusion

Target WCAG AA contrast and complete keyboard operation. Provide visible labels, tooltips, focus indicators, predictable focus order, and focus restoration after overlays. Never encode meaning through color alone. Respect reduced-motion preferences and avoid decorative motion. Where GPUI cannot yet expose complete native accessibility semantics, preserve clear visible text and keyboard behavior and adopt semantic APIs as the framework makes them available.
