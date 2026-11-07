# CLAUDE.md — spacetrail

macOS-native disk space monitor, explainer and cleaner in Rust (CLI + TUI + daemon). Source of truth: `docs/BRIEF.md` (vision/requirements), `docs/PLAN.md` (milestones), `docs/ARCHITECTURE.md`, `docs/CLEANER_SPEC.md`, `docs/NOTES.md` (spike findings). Everything in the project is in **English**.

## Workflow
- Work milestone by milestone (see PLAN.md). Ending a milestone means: tests green, `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo fmt --all`, a conventional commit (`feat(scanner): …`, `fix(store): …`, `docs: …`), and a short summary of what changed and what's next.
- If macOS behavior is uncertain (APFS, TCC, FSEvents, dataless), write a tiny spike, run it, and record the result in `docs/NOTES.md`. Don't guess.
- Read-only commands on the real machine (`scan`, `status`, `top`, `diff`, `timeline`, `doctor`) are allowed. **Any real `clean --apply` requires showing the dry-run plan and getting explicit approval first.**

## Commands
```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo deny check
cargo run -p spacetrail-cli -- <args>        # the `spacetrail` binary
cargo bench -p spacetrail-core               # scanner benchmarks
cargo xtask gen-tree <dir> --files N         # synthetic fixture trees
cargo insta review                           # TUI / output snapshots
```

## Crate boundaries
- `spacetrail-core`: platform-agnostic logic only (model, walker, diff, forecast, formatting). No `unsafe`, no macOS APIs. OS access goes through the `Platform` trait.
- `spacetrail-macos`: **all** FFI/`unsafe` and macOS integration.
- `spacetrail-store`: the only crate that talks SQL.
- `spacetrail-cleaners`: schema, planner, and the **single executor**, which is the only code that may delete files or spawn processes.
- `spacetrail-proto`: serde wire types only. `spacetrail-daemon`: daemon library. `spacetrail-cli`: the binary (CLI, TUI, HTML report, `daemon run`).
- Dependency direction: cli → daemon → cleaners → store → core; macos → core; proto is a leaf. No cycles.

## Non-negotiable safety rules
1. Tests never touch the real home directory. Always use `SPACETRAIL_HOME` + `tempfile` (and point `HOME` at the temp dir in integration tests).
2. Every deletion goes through the single executor in `spacetrail-cleaners` (deny-list, canonicalization, containment, no symlink following, TOCTOU re-validation). `remove_dir_all`/`remove_file`/`remove_dir`/`unlink`/`Command::new` are banned by `clippy.toml` `disallowed-methods`. The only allowed `#[expect(clippy::disallowed_methods)]` is inside the executor, and CI greps for it.
3. Dry-run is the default; Trash is the default; `manual` risk is never auto-deleted; autoclean only runs `safe` cleaners the user enabled, with caps.
4. Community/user cleaners can't execute commands unless the user opts in per cleaner (`cleaners.allow_commands`). Commands are argv arrays, never shell strings.
5. Never read or open dataless (cloud placeholder) files: `lstat` only. Never require root.
6. No telemetry, no network calls. The tool works fully offline.

## Conventions
- Edition 2024, MSRV 1.85. `thiserror` in libraries, `anyhow` only in the binary. `tracing` for logs, never `println!` in libraries.
- Sizes: store bytes as `u64`. Default display is allocated bytes in decimal units (Finder-compatible); `--binary` switches to IEC.
- Every CLI command supports `--json` (stable, documented shape) and honors `NO_COLOR`. Never use color as the only signal: risk is always icon + label.
- Unreadable means "unknown size", never 0. Cloud-only files are reported separately.
- Tests: unit + `proptest` for size math and path safety; `insta` snapshots for CLI output and TUI (`TestBackend`); an integration test with fixtures for every built-in cleaner; `criterion` for the scanner.
