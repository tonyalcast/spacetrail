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
- `apps/desktop` (`spacetrail-app`): Tauri desktop app; the only crate allowed to use the network (`reqwest`), see rule 6. Build with `cd apps/desktop && cargo tauri build` (signs with the `spacetrail-dev` identity).
- Dependency direction: cli → daemon → cleaners → store → core; macos → core; proto is a leaf. No cycles.

## Non-negotiable safety rules
1. Tests never touch the real home directory. Always use `SPACETRAIL_HOME` + `tempfile` (and point `HOME` at the temp dir in integration tests).
2. Every deletion goes through the single executor in `spacetrail-cleaners/src/executor` (deny-list, canonicalization, containment, no symlink following, TOCTOU re-validation, Trash + journal + undo). Only the executor may call `spacetrail_macos::trash::move_to_trash`; nothing may call `removeItemAtURL`. `remove_dir_all`/`remove_file`/`remove_dir`/`unlink`/`Command::new` are banned by `clippy.toml` `disallowed-methods`. The only allowed `#[expect(clippy::disallowed_methods)]` are inside the executor and `spacetrail-macos/src/probe.rs` (a fixed allow-list of read-only system probes such as `diskutil apfs list -plist`). `cargo xtask check-safety` enforces this in CI.
3. Dry-run is the default; Trash is the default; autoclean only runs `safe` cleaners the user enabled, with caps. `manual` ("your call") items are never removed in bulk or automatically: only items the user ticks one by one in the app's plan (author's request, 2026-11-05), and only through the tool's own uninstaller (`ollama rm`, `rustup toolchain uninstall`, `simctl runtime delete`, `hf cache rm`, `docker volume prune`…) or the Trash, never permanently. Protected items (default toolchain/Node version, non-deletable runtimes) are never removed.
4. Community/user cleaners can't execute commands unless the user opts in per cleaner (`cleaners.allow_commands`). Commands are argv arrays, never shell strings.
5. Never read or open dataless (cloud placeholder) files: `lstat` only. Never require root.
6. No telemetry. The CLI, the libraries and the daemon make no network calls and work fully offline. **Single exception, requested by the author (2026-11-05):** the desktop app (`apps/desktop`) may call the LLM provider the user configured, and only when the user presses "Explain" on an item. It sends metadata only (path with home shown as `~`, size, dates, download origin, names of up to 30 items inside a folder, which the user can turn off). It never sends file contents and shows the user exactly what was sent. API keys live in `~/.config/spacetrail/api-keys.json` (mode 600, folder 700; moved out of the Keychain on 2026-11-05 at the author's request because the Keychain prompted after every rebuild of the self-signed app). Never log or display them. Any other network use needs the author's explicit approval.

## Conventions
- Edition 2024, MSRV 1.88 (raised from 1.85 on 2026-11-05 for the plist/quick-xml fix RUSTSEC-2026-0194). `thiserror` in libraries, `anyhow` only in the binary. `tracing` for logs, never `println!` in libraries.
- Sizes: store bytes as `u64`. Default display is allocated bytes in decimal units (Finder-compatible); `--binary` switches to IEC.
- Every CLI command supports `--json` (stable, documented shape) and honors `NO_COLOR`. Never use color as the only signal: risk is always icon + label.
- Unreadable means "unknown size", never 0. Cloud-only files are reported separately.
- Tests: unit + `proptest` for size math and path safety; `insta` snapshots for CLI output and TUI (`TestBackend`); an integration test with fixtures for every built-in cleaner; `criterion` for the scanner.
