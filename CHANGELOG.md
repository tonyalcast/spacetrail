# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Cargo workspace scaffold (core, macos, store, cleaners, proto, daemon, cli, xtask).
- Safety rail: clippy `disallowed-methods` for deletion/process APIs + `cargo xtask check-safety`.
- CI: fmt, clippy, tests, x86_64 cross-check, MSRV, cargo-deny.
- Scanner: parallel lstat-only walker (hard links once, no symlink following, one filesystem, cloud-only files never read, unreadable folders reported as unknown with privacy vs. ownership distinction).
- macOS layer: capacity (statfs + Finder's "available"), APFS volume breakdown, firmlinks, Full Disk Access probe, dataless guard.
- CLI: `status`, `scan`, `top`, `doctor`, with `--json` and `--binary`.
- `spacetrail suggest`: read-only recommendations grouped by risk (safe / review / manual) with sizes, how-to commands, what regenerates, and apps to close first.
- Cleaner catalog: 60 built-in TOML rules (editors, JS/Python/Go/Rust/.NET/JVM caches, Xcode, Android, AI models, browsers, chat apps, logs, installers) embedded at build time; user rules from `~/.config/spacetrail/cleaners`.
- Detectors: git worktrees, stale project artifacts (node_modules, target, venvs, bin/obj, Gradle, Pods, …), orphaned editor workspaces, old Cursor agent versions, Docker reclaimable space (`docker system df`), large app data, swap/Trash/log-hog advice.
- Scanner tracks the newest modification time per folder (staleness) and can keep the least recently modified large files.
- Desktop app: draggable custom title bar; Largest / Oldest files tabs with a "Hide system files" filter; Settings with API keys for Anthropic, OpenAI and OpenAI-compatible servers (stored in the Keychain) and a model picker loaded from the provider; "✨ Explain" on every file and folder asks the chosen model where it comes from, whether it is safe to delete and what deleting it would change (metadata only, never contents; shows what was sent).
- Desktop app (`apps/desktop`, Tauri): free-space gauge with status, APFS volume bar, drill-down treemap with striped "could be freed" areas, suggestion cards by risk with copy-command and Show-in-Finder, a what-if simulator, and a Full Disk Access onboarding that opens System Settings and detects the grant. Signed with a stable identity so macOS keeps its permissions across rebuilds; the permission belongs to the app, not the terminal.
