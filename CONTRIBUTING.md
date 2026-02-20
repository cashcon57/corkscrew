# Contributing to Corkscrew

Thanks for your interest in Corkscrew! This guide covers everything you need to get started.

## Table of Contents

- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Project Structure](#project-structure)
- [Making Changes](#making-changes)
- [Pull Request Process](#pull-request-process)
- [Coding Conventions](#coding-conventions)
- [NexusMods Compliance](#nexusmods-compliance)
- [Testing](#testing)
- [Issue Guidelines](#issue-guidelines)

## Getting Started

### Prerequisites

- **Node.js** 18+ — [nodejs.org](https://nodejs.org/)
- **Rust** (stable) — [rustup.rs](https://rustup.rs/)
- **Tauri CLI v2** — `cargo install tauri-cli`

**macOS additional:**
- Xcode Command Line Tools: `xcode-select --install`

**Linux additional:**
- System dependencies for Tauri: see [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#linux)

### Clone and Run

```bash
git clone https://github.com/cashcon57/corkscrew.git
cd corkscrew
npm install
cargo tauri dev
```

This starts the app in development mode with hot-reload for the Svelte frontend and recompilation for the Rust backend.

## Development Setup

### Key Commands

| Command | What it does |
|---------|-------------|
| `cargo tauri dev` | Start development server with hot-reload |
| `cargo tauri build` | Create production build (.app/.dmg on macOS) |
| `cargo test -p corkscrew` | Run Rust backend tests |
| `npx svelte-check --threshold error` | TypeScript/Svelte type checking |
| `npm run build` | Build the Svelte frontend only |

### Editor Setup

The project uses:
- **Rust** — Standard `rustfmt` formatting
- **Svelte/TypeScript** — Svelte 5 with runes (`$state`, `$derived`, `$effect`)
- **CSS** — Custom properties (CSS variables) defined in `src/app.css`

Recommended VS Code extensions:
- Svelte for VS Code
- rust-analyzer
- Tauri

## Project Structure

```
src/                    Svelte 5 frontend (SvelteKit, adapter-static)
├── lib/api.ts          Tauri IPC wrappers (typed invoke calls)
├── lib/types.ts        Shared TypeScript interfaces
├── lib/components/     Reusable Svelte components
├── routes/             SvelteKit pages
└── app.css             Design system tokens and themes

src-tauri/src/          Rust backend
├── lib.rs              Tauri command handlers (IPC entry points)
├── database.rs         SQLite with versioned migrations
├── plugins/            Game detection plugins (trait-based)
└── ...                 Feature modules (see README for full list)
```

**Frontend-backend communication** happens through Tauri IPC:
1. Rust functions annotated with `#[tauri::command]` in `lib.rs`
2. TypeScript wrappers in `src/lib/api.ts` call them via `invoke()`
3. Types are defined in both `src/lib/types.ts` (TS) and the respective Rust modules

## Making Changes

### Branch Naming

Use descriptive branch names:
- `feat/collection-install` — New features
- `fix/plugin-sort-crash` — Bug fixes
- `refactor/database-queries` — Code refactoring
- `docs/contributing-guide` — Documentation

### Commit Messages

Write clear commit messages that explain the **why**, not just the **what**:

```
Add progress events for mod installation

Replace generic spinner with step-by-step progress feedback during
mod installation using Tauri event system. Users now see which step
is active (extracting, deploying, syncing plugins).
```

- First line: imperative mood, under 72 characters
- Body: explain motivation and context when not obvious
- Reference issues when applicable: `Fixes #42`

## Pull Request Process

1. **Fork and branch** — Create a feature branch from `main`
2. **Make your changes** — Keep PRs focused on a single concern
3. **Test locally:**
   - `cargo test -p corkscrew` — All Rust tests must pass
   - `npx svelte-check --threshold error` — Zero type errors
   - `cargo tauri dev` — Manual smoke test if UI changes are involved
4. **Open a PR** against `main` with:
   - A clear title (under 72 characters)
   - Description of what changed and why
   - Screenshots for UI changes
   - Any testing notes
5. **CI must pass** — The PR checks run automatically (Rust tests, Svelte type check, formatting)
6. **Address review feedback** — Push additional commits to the same branch

### PR Size Guidelines

- **Small PRs are preferred** — Easier to review, faster to merge
- If a feature is large, consider splitting into multiple PRs (e.g., backend first, then frontend)
- If you're adding a new Rust module, include tests in the same PR

### What Makes a Good PR

- Focused scope — one feature or fix per PR
- Tests for new Rust functions
- Types updated in both Rust and TypeScript when adding/changing IPC commands
- No unrelated changes mixed in
- Works on both macOS and Linux (or clearly documented as platform-specific)

## Coding Conventions

### Rust

- Follow standard `rustfmt` formatting
- Use `thiserror` for error types in modules, `String` errors for Tauri commands
- Database access goes through `ModDatabase` methods (never raw SQL in `lib.rs`)
- New IPC commands: add the `#[tauri::command]` in `lib.rs`, register in the `generate_handler!` macro, add the TypeScript wrapper in `api.ts`, and add the type in `types.ts`
- Use `Arc<ModDatabase>` (not `Mutex`) — the database has internal locking

### Svelte / TypeScript

- Svelte 5 runes: use `$state()`, `$derived()`, `$effect()` — not Svelte 4 stores
- Types: define interfaces in `src/lib/types.ts`, not inline
- CSS: use the design tokens from `app.css` (e.g., `var(--bg-primary)`, `var(--text-primary)`)
- Keep components in `src/lib/components/`, pages in `src/routes/`

### Adding a New Game

Games are added as plugins implementing the `GamePlugin` trait:

1. Create `src-tauri/src/plugins/your_game.rs`
2. Implement the `GamePlugin` trait (see `skyrim_se.rs` for the pattern)
3. Register in `games.rs` plugin registry
4. Add game-specific plugin management if needed

## NexusMods Compliance

**This is critical.** Corkscrew must comply with NexusMods API terms:

- **NEVER** automate downloads for free NexusMods users
- Free users must click "Slow Download" on the NexusMods website for every download
- Only premium users may use API-initiated downloads
- The `is_premium` check in `nexus.rs` enforces this — do not bypass it
- Respect API rate limits; use graceful error handling (skip on failure, don't retry aggressively)
- When in doubt about a NexusMods integration, ask before implementing

Violating these terms risks getting the app's API access revoked for all users.

## Testing

### Rust Tests

```bash
cd src-tauri
cargo test
```

Tests are co-located with the code they test (in `#[cfg(test)]` modules at the bottom of each `.rs` file). When adding new functionality, include unit tests.

### Frontend Type Checking

```bash
npx svelte-check --threshold error
```

This catches TypeScript errors, missing types, and Svelte template issues.

### Manual Testing

For UI changes or new features, test with:
1. `cargo tauri dev` — Development mode
2. A real Wine bottle with Skyrim SE installed (if testing game-specific features)
3. Both light and dark themes

## Issue Guidelines

### Bug Reports

Include:
- OS and version (macOS 15.x, SteamOS 3.x, Ubuntu 24.04, etc.)
- Wine source (CrossOver, Whisky, Proton, etc.)
- Game and mod details if relevant
- Steps to reproduce
- Error messages or screenshots

### Feature Requests

- Check existing issues first
- Describe the use case, not just the solution
- If it involves NexusMods integration, note the compliance considerations

## Questions?

Open an issue or start a discussion. We're happy to help you get set up.
