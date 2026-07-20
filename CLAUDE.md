# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project scope

**odx** is a Rust CLI (binary name `odx`, crate name `odoo-cli`) that scaffolds and operates Odoo development projects: creating projects, managing the Python venv, running Odoo, managing the Postgres dev database via Docker Compose, running filtered test suites, exporting translations, etc.

odx CE targets **vanilla** upstream Odoo (`git clone --branch <version> --depth 1 https://github.com/odoo/odoo.git`) with no patches to Odoo core. Contributions should not reintroduce core-patching flows unless the project explicitly changes that policy.

## Common commands

```bash
cargo build                                   # debug build -> target/debug/odx
cargo run -- --help                           # run the CLI
cargo run -- new my_project -v 18.0           # exercise a subcommand locally

cargo fmt --all -- --check                    # formatting (CI-enforced)
cargo clippy --all-targets -- -D warnings     # lint (CI-enforced, warnings fail)
cargo deny check                              # license/advisory/ban policy (deny.toml)
cargo audit                                   # RustSec advisory scan (needs `cargo install cargo-audit --locked`)

cargo test --lib                              # fast unit tests (what CI runs on every push)
cargo test --lib <test_name>                  # run a single unit test, e.g. cargo test --lib project_addon_modules_finds_nested_custom_addons
cargo test                                    # full suite including tests/integration_tests.rs — clones/downloads real Odoo sources, slow, needs network
```

Unit tests live inline per-module (`#[cfg(test)] mod tests` in `src/utils.rs`, `src/commands/test.rs`, `src/commands/clean.rs`, `src/tui.rs`, ...) and are the only tests CI runs (`cargo test --lib`). `tests/integration_tests.rs` builds real projects with `odx new`/`odx install` against actual Odoo 17.0/18.0/19.0 sources (downloaded as zips, or cloned as a fallback) — treat it as a slow, network-dependent suite you run deliberately, not as part of a normal edit loop.

There is no separate lint/build script — CI (`.github/workflows/ci.yml`) runs exactly the `lint`, `deny`, and `build` jobs shown above as three parallel jobs.

## Architecture

`src/main.rs` just parses `Cli` (clap, from `src/cli.rs`) and calls `cli.run()`. Everything else lives in the `odoo_cli` lib crate (`src/lib.rs`):

- **`cli.rs`** — clap `Cli`/`Commands` definitions and the single `match` that dispatches each subcommand to its `commands::<name>::execute(...)` function. This is the map of what the CLI does; start here when adding a subcommand or flag.
- **`commands/`** — one module per subcommand (`run`, `update`, `update_module`, `shell`, `db`, `i18n`, `test`, `install`, `sync`, `clean`, `new`, `doctor`). Each exposes an `execute(ui: &Ui, ...)` function returning `Result<(), String>`. `db.rs` itself has a `#[derive(Subcommand)]` (`DbCommands`: start/stop/logs/ls/psql/drop) dispatched from its own `execute`.
- **`utils.rs`** — shared, stateless helpers used across commands: locating the project root (walks up looking for `compose.yml`/`compose.yaml`), resolving the venv Python / a specific Python version (via pyenv, `python<version>`, or `python3`/`python` + version check), running child processes (`execute_command*`, including a line-streaming variant with optional log-file mirroring and heartbeat used by `odx test`), addon discovery (`custom_addons`/`external_addons` scanning, building `addons_path`, syncing it into `odoo.conf.local`), Odoo version detection, and zip extraction for `odx new`.
- **`ui.rs`** — the `Ui` abstraction all commands take instead of printing directly. Wraps `--json`/`--quiet`/`--color`/`--no-progress` global flags into `info`/`warn`/`error`/`success`/`heading`/`check`/`spinner`/`progress_bar`/`prompt_confirm`. `--json` mode suppresses colors, progress, and interactive prompts (prompts error out). Commands should go through `Ui`, not `println!`/`eprintln!`, to stay consistent under `--json`/`--quiet`.
- **`os_context.rs`** / **`install_guide.rs`** — OS/distro detection and per-OS install instructions, used by `doctor`/`install` to give actionable remediation steps for missing system dependencies (see `src/dependencies/system-deps.toml`).
- **`tui.rs`** — ratatui/crossterm live log dashboard for `odx run` (level-colored, scrollable, filterable by log level, `/`-search). Also exposes `OdooLogLine`/`colorize()`, reused by `run.rs`'s non-TTY fallback so plain output is level-colored too. Owns graceful child shutdown (SIGINT then kill, Unix; hard kill on Windows) and terminal restore on quit/crash/panic.
- **`project_template/`** — `include_str!`-embedded templates (`compose.yml`, `odoo.conf`, `README.md`, `AGENTS.md`) rendered via simple `{{var}}` substitution (`generate_from_template`) when `odx new` scaffolds a project. `AGENTS.md.template` is written into every *generated* Odoo project (not this repo) to brief agents working inside it — update it if odx's generated-project conventions change.

### Generated-project layout (what odx operates on)

A project created by `odx new` looks like: `compose.yml` (Postgres via Docker Compose), `odoo.conf` / `odoo.conf.local` (the local copy is git-ignored and has `addons_path` kept in sync by odx), `src/odoo` (vanilla Odoo checkout), `custom_addons/`, `external_addons/`, `.venv`. Most commands (`run`, `update`, `test`, `i18n`, ...) require being invoked from inside such a project tree; they call `find_project_root()` to locate it and `ensure_venv()`/`require_odoo_bin()` to validate prerequisites before doing anything.

### `odx test` specifics

`commands/test.rs` is the largest/most involved command: it discovers modules under `custom_addons`, creates a timestamped temp database, runs `odoo-bin` once with `--test-tags`, streams/mirrors output to `.testing/sessions/<run_id>/combined.log`, and guarantees the temp database is dropped afterward (via a `Drop` guard and a Ctrl+C handler), even on failure or interruption.

### `odx run` specifics

Defaults to a live TUI dashboard (`src/tui.rs`) when stdout is a real TTY and none of `--plain`/`--json`/`--no-progress` are set; otherwise falls back to plain, level-colored streaming. Both paths always mirror the complete, unfiltered log to `.testing/sessions/run-<timestamp>/run.log`, mirroring the session-directory convention `odx test` established.

## Policy files that affect CI

- `deny.toml` — cargo-deny license allowlist, advisory policy, and source restrictions (only crates.io allowed). If you add a dependency with a license/advisory exception, document why in a comment near the exception, per `CONTRIBUTING.md`.
- `.github/workflows/security-audit.yml` — runs `cargo audit` on `Cargo.toml`/`Cargo.lock` changes and daily via cron.
- Releases (`.github/workflows/release.yml`) are tag-triggered (`v*`) and require the tag to match the `version` in `Cargo.toml`; they build Linux/Debian/Arch/Windows artifacts via `packaging/` and `scripts/release/`.

## Conventions

- Commit subjects optionally use a bracketed prefix matching this repo's history: `[FEAT]`, `[FIX]`, `[BUG]`, `[ADD]`, `[MIG]`. First line in English; body can be in any language.
- Errors are plain `Result<(), String>` / `Result<T, String>` throughout (no `anyhow`/`thiserror`) — match that style in new code.
- Prefer focused PRs; avoid mixing large refactors with functional changes (see `CONTRIBUTING.md`).
