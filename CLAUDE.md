# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

openmango is a TUI (Terminal User Interface) application for MongoDB management, built with OpenTUI and React.

## Tech Stack

- **Runtime**: Bun
- **UI Framework**: OpenTUI (`@opentui/core`, `@opentui/react`) - a React-based terminal UI framework
- **React**: v19
- **Language**: TypeScript (strict mode)

## Commands

```bash
# Install dependencies
bun install

# Run with hot reload (development)
bun dev
```

## Architecture

The app uses OpenTUI's React bindings to render terminal interfaces. Key concepts:

- `createCliRenderer()` - creates the terminal renderer from `@opentui/core`
- `createRoot(renderer).render()` - React-style mounting from `@opentui/react`
- JSX components use terminal-specific elements: `<box>`, `<text>`, `<ascii-font>`
- Layout uses flexbox properties (`alignItems`, `justifyContent`, `flexGrow`)
- Text styling via `TextAttributes` from `@opentui/core`

Entry point: `src/index.tsx`
