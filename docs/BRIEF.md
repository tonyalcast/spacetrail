# spacetrail — Project Brief & Kickoff Prompt for Claude Code

> Paste this whole file into Claude Code (or save it as `docs/BRIEF.md` in an empty repo and say: *"Read docs/BRIEF.md and follow it"*).

---

## 0. How I want you to work (read first)

1. **Start in plan mode. Do not write code yet.** Read this entire brief, then produce:
   - `docs/PLAN.md` — phased implementation plan with milestones, each with acceptance criteria.
   - `docs/ARCHITECTURE.md` — crate layout, data model, SQLite schema, daemon ↔ CLI/TUI protocol, macOS APIs used and why.
   - `docs/CLEANER_SPEC.md` — the TOML schema for community cleaners, with 3 full examples.
   - A short list of **open questions / risks** for me. Wait for my approval before starting Phase 1.
2. Create a `CLAUDE.md` at repo root summarizing conventions (build/test commands, safety rules from §6, crate boundaries) so future sessions stay consistent.
3. Work **milestone by milestone**. At the end of each: all tests green, `cargo clippy -- -D warnings` clean, `cargo fmt` applied, a conventional commit, and a short summary of what changed + what's next.
4. **Never run a destructive operation against my real home directory during development.** All cleaner/deletion tests run inside a sandbox (`SPACETRAIL_HOME` / temp dirs). If you want to run a real `clean` on my machine, ask first and show the dry-run plan.
5. Read-only commands on my real machine (`scan`, `status`, `top`, `diff`) are fine — in fact, once the MVP works, **run a real scan of my Mac and tell me what's eating my disk** (that's the problem that started this project: my disk is filling up uncontrollably).
6. When a macOS behavior is uncertain (APFS accounting, TCC, FSEvents edge cases), write a tiny spike/experiment, run it, and record the finding in `docs/NOTES.md` instead of guessing.

---

## 1. Vision

**spacetrail** is an open-source, macOS-native disk space **monitor, explainer, and cleaner** for power users and developers, written in Rust.

It answers three questions no existing tool answers together:

1. **"How has my disk usage evolved over time?"** → continuous, low-overhead history with timeline charts.
2. **"What grew, where, and who is writing right now?"** → growth attribution by directory over any time window, plus real-time detection of hot paths and the processes writing the most bytes.
3. **"What can I safely delete?"** → community-maintained, declarative cleaners classified by risk (`safe` / `review` / `manual`), with dry-run by default, Trash by default, a journal, and undo. Safe rules can be automated on a schedule.

### Why it's different

| Tool | Gap spacetrail fills |
|---|---|
| ncdu / dust / dua / gdu | One-shot snapshots; no history, no "what changed", no cleaning knowledge |
| DaisyDisk / GrandPerspective | GUI-only, no history, no automation, closed-source |
| CleanMyMac & co. | Closed-source, opaque about what they delete, subscription |
| kondo / npkill | Only project artifacts, no system view, no monitoring |

**Principles:** local-only (zero telemetry), transparent (every byte deleted is explained and logged), safe by default, fast, scriptable (`--json` everywhere), community-extensible.

---

## 2. Decisions already made

| Topic | Decision |
|---|---|
| Platform | **macOS only** (Apple Silicon + Intel). Minimum: macOS 13 Ventura (confirm in plan). Architect so Linux could be added later behind a platform trait, but do not implement it. |
| Language | **Rust** (stable, edition 2021 or 2024 — your call, justify it). |
| Interfaces (v1) | **CLI + TUI** (ratatui). A GUI comes later → keep core logic in a library with a clean API; the daemon exposes a local API the future GUI will reuse. |
| Must-have visualization | **Timeline charts of storage growth** (total used/free over time, and per-directory history) in the TUI, plus `spacetrail report --html` to export a self-contained HTML report with interactive charts. |
| Differentiators (all in scope) | History & attribution · Community cleaners · Risk classification + automation · Real-time detection |
| Language of project | **English** — code, CLI messages, docs, README. |
| Distribution | Homebrew tap + `cargo install` + GitHub Releases (arm64 + x86_64; universal if practical), using `dist` (cargo-dist). Code signing + notarization planned (requires an Apple Developer ID — design for it, make it optional in CI until I add secrets). |
| License | `MIT OR Apache-2.0` (both license files). |
| Name | `spacetrail` (binary: `spacetrail`; consider a short alias like `st` only if it doesn't collide — check). Verify availability on crates.io and GitHub/Homebrew before publishing. |

---

## 3. Architecture (proposal — refine in ARCHITECTURE.md)

Cargo workspace:

```
spacetrail/
├── crates/
│   ├── spacetrail-core/      # domain model, scanner, size math, diffing, forecasting (platform-agnostic)
│   ├── spacetrail-macos/     # FFI & OS integration: statfs/volume capacity, FSEvents, proc_pid_rusage,
│   │                         # dataless detection, firmlinks, tmutil, Trash, launchd, notifications, TCC checks
│   ├── spacetrail-store/     # SQLite (rusqlite, bundled) — schema, migrations, rollups, queries
│   ├── spacetrail-cleaners/  # cleaner TOML schema, loader, validator, planner, executor, journal/undo
│   ├── spacetrail-daemon/    # background agent: scheduled snapshots, FSEvents incremental, live monitor,
│   │                         # alerts, autoclean; local API over a Unix domain socket (JSON-RPC or similar)
│   └── spacetrail-cli/       # the `spacetrail` binary: clap CLI + ratatui TUI + HTML report export
├── cleaners/                 # built-in cleaner rules (*.toml), embedded into the binary at build time
├── docs/
└── xtask/                    # (optional) dev tasks: fixtures, release helpers
```

Key libraries to evaluate (justify each choice, prefer few deps): `clap`, `ratatui` + `crossterm`, `rusqlite` (bundled), `serde` + `toml` + `serde_json`, `rayon` / `jwalk` (or a custom parallel walker), `trash`, `objc2` family / `core-foundation` for macOS APIs, `fsevent-sys` or thin custom FSEvents bindings, `tracing`, `anyhow`/`thiserror`, `insta` (snapshot tests), `tempfile`, `proptest`.

Paths:
- Config: `~/.config/spacetrail/config.toml` (respect `XDG_CONFIG_HOME`).
- Data/DB: `~/Library/Application Support/spacetrail/spacetrail.db`.
- User cleaners: `~/.config/spacetrail/cleaners/*.toml`.
- Everything overridable via `SPACETRAIL_HOME` (critical for tests).

---

## 4. Feature specification

### 4.1 Scanner (the foundation — get it right)

- Parallel directory walk; measure **allocated size** (`st_blocks * 512`), also keep apparent size. Show allocated by default.
- **Hard links**: count each `(dev, inode)` once.
- **Don't cross filesystems** by default. On APFS, `/` and `/System/Volumes/Data` are joined by **firmlinks** (`/usr/share/firmlinks`) — avoid double counting; define clearly what "scan the whole disk" means (likely scan `/System/Volumes/Data`, present paths in user-friendly form).
- **Never trigger cloud downloads**: detect dataless/placeholder files (iCloud Drive, `~/Library/CloudStorage` providers) via `st_flags & SF_DATALESS` and never open/read them. Report them separately ("cloud-only, 0 bytes local").
- **APFS clones** share blocks and cannot be cheaply deduplicated — document this limitation; don't pretend accuracy we don't have.
- **TCC / Full Disk Access**: without FDA, some dirs (`~/Library/Mail`, `Messages`, `Safari`, some `Containers`, other apps' data) are unreadable. Track those as **"unreadable (unknown size)"**, never as 0. `spacetrail doctor` detects whether FDA is granted and explains how to grant it (for the terminal app and for the daemon binary).
- **Unattributed space** = `used (from volume stats) − scanned`. Break down what we can: local Time Machine snapshots (`tmutil listlocalsnapshots /`), VM swap (`/System/Volumes/VM`), sleepimage, `/private/var/folders`, Docker/VM disk images, purgeable space. Show the rest honestly as "System / other / unattributed".
- Volume stats: total, used, free, **purgeable**, and "available for important usage" (`NSURLVolumeAvailableCapacityForImportantUsageKey`) — these differ on APFS and users are confused by it; explain in the UI.
- Performance target (verify with benchmarks): ≥1M files in < 60s on an M1 SSD warm cache; memory bounded (aggregate as you go, don't keep every file in memory — keep only directory aggregates + top-N largest files).
- Later optimization (spike it): `getattrlistbulk(2)` for faster enumeration.

### 4.2 History & attribution

- **Snapshots** stored in SQLite: volume stats every N minutes (cheap) + directory-size tree at a configurable interval (default: every 6h, configurable) and on demand.
- Store directory aggregates only above a threshold (e.g. ≥ 50 MB) up to a configurable depth, **plus** always-tracked roots (home top-level, `~/Library/*`, `/Applications`, all paths referenced by cleaners). Path table normalized (`paths(id, path)`), sizes in a fact table.
- **Incremental updates via FSEvents**: the daemon persists the last FSEvents event ID; on restart it replays changes since then (`sinceWhen`) and rescans only affected subtrees. Handle `MustScanSubDirs`, `HistoryDone`, dropped-events and "root changed" flags by falling back to a rescan of the affected root.
- **Retention & rollups**: raw for 7 days → hourly for 90 days → daily forever (configurable). Target DB growth < 50 MB/year for a typical user.
- **Queries / commands**:
  - `spacetrail diff --since 7d` (or `--from/--to`) → top growers & shrinkers by directory, drill-down tree, with absolute and % change.
  - `spacetrail timeline [PATH] --range 24h|7d|30d|90d|all` → time series (ASCII/braille chart in terminal, `--json` for raw data).
  - **Forecast**: linear (or robust) regression over the last 7/30 days → "at the current rate, disk full in ~N days". Show confidence/variance, don't overclaim.

### 4.3 Real-time detection ("who is filling my disk right now?")

- **Hot paths**: from the FSEvents stream, aggregate bytes changed per directory over sliding windows (1 min / 10 min / 1 h) by stat-ing changed files (rate-limited, coalesced).
- **Top writing processes**: poll `proc_listpids` + `proc_pid_rusage` (`ri_diskio_byteswritten`) to compute per-process write rates **without root** (own-user processes; document limits for root/system processes). Optionally document `fs_usage` (root) as an advanced mode — do not require root.
- Correlate both into a "Live" view: e.g. *"com.docker.backend wrote 3.2 GB in the last 10 min; hottest path: ~/Library/Containers/com.docker.docker/…"*.
- `spacetrail watch` (CLI streaming) + TUI Live screen.

### 4.4 Alerts

- Rules (configurable): free space below X GB or Y %; growth > N GB in a window; forecast "full in < D days"; a single path growing > N GB/h.
- Delivered by the daemon as macOS notifications (start with `osascript display notification`; plan a proper notifier for the future GUI app). Debounced, with quiet hours. `spacetrail alerts` lists history.

### 4.5 Cleaners (community-extensible)

**Risk classes (core concept — show everywhere with icon + label, never color alone):**

- `safe` — fully regenerable automatically, no user data, negligible impact (e.g. Xcode DerivedData, log files > 7 days, package-manager caches that rebuild transparently). **Eligible for autoclean.**
- `review` — regenerable but costly (large re-downloads/rebuilds) or may contain something the user wants (e.g. Docker images, Gradle caches, stale `node_modules`, old Downloads). Interactive only, explicit confirmation.
- `manual` — user data or expensive assets (AI models, iOS backups, Xcode Archives, simulators, VMs). spacetrail **only measures and explains**, shows the exact command/steps; never deletes automatically (allow interactive per-item Trash at most, behind a strong confirmation — decide in plan).

**Declarative TOML schema (draft — finalize in CLEANER_SPEC.md):**

```toml
id = "xcode-derived-data"
name = "Xcode DerivedData"
description = "Build intermediates and indexes. Xcode regenerates them on next build."
risk = "safe"
category = "developer/xcode"
regenerates = "Automatically on next Xcode build (first build will be slower)."
docs = "https://developer.apple.com/documentation/xcode"

[detect]
any_path_exists = ["~/Library/Developer/Xcode/DerivedData"]
# also: command_exists = "docker", app_bundle_id = "com.apple.dt.Xcode"

[measure]
paths = ["~/Library/Developer/Xcode/DerivedData/*"]

[preconditions]
process_not_running = ["Xcode"]

[[action]]
type = "delete_contents"          # delete_contents | trash | command | manual
paths = ["~/Library/Developer/Xcode/DerivedData/*"]
older_than = "0d"                 # optional age filter
exclude = []
```

- `command` actions use **argv arrays, never shell strings** (no injection). Only built-in (embedded, reviewed) rules may run commands by default; user/community rules with `command` actions require an explicit opt-in flag/config.
- `manual` actions carry `instructions` (markdown) and optional suggested commands.
- **Project-artifact cleaners** (kondo/npkill-style): find projects by marker files under configured code roots, detect staleness by last git commit date or newest source mtime, offer to remove regenerable artifacts: `node_modules` (package.json), `target` (Cargo.toml), `.venv`/`venv` (pyvenv.cfg), `build`/`.gradle` (Gradle), `Pods` (Podfile), `.next`/`.nuxt`/`.turbo`/`.parcel-cache`, `.dart_tool`, `.terraform`, `__pycache__`, project `DerivedData`. Default `risk = review`, default staleness 60 days.
- `spacetrail cleaners list|show <id>|test <file.toml>` — `test` validates schema, resolves paths in a sandbox, and prints the plan (this is what community contributors run before a PR).

**Initial built-in catalog** (macOS + developer heavy — the author's machine has most of these: Docker, Xcode, Android/Gradle, Node via nvm/npm/yarn/bun, Python via conda, Rust, Go, .NET/NuGet, PlatformIO, Ollama, LM Studio, VS Code, Cursor, Windsurf, Antigravity, JetBrains). Verify each path/command, mark risk, and write tests for each:

| Area | Examples (risk) |
|---|---|
| System/user | `~/Library/Caches/*` age-filtered (safe), `~/Library/Logs` > 7d (safe), DiagnosticReports (safe), Trash items > 30d (safe; "empty all" = review), old `.dmg/.pkg/.zip` installers in Downloads (review → Trash), Downloads > 90d (review → Trash), large files report (manual), local TM snapshots (review, `tmutil thinlocalsnapshots`), iOS/iPadOS backups (manual), Mail downloads (safe), Messages attachments (manual) |
| Xcode / Apple dev | DerivedData (safe), CoreSimulator caches (safe), unavailable simulators `xcrun simctl delete unavailable` (safe), iOS DeviceSupport (review), simulator runtimes (manual), Archives (manual), CocoaPods cache (safe) |
| Containers / VMs | Docker: build cache `docker builder prune` (review), `docker system prune` (review), dangling volumes (manual), report `Docker.raw` real allocated size; OrbStack, Colima (review/manual) |
| JS | npm cache `npm cache clean --force` (safe), yarn (safe), pnpm `pnpm store prune` (safe), bun cache (safe), old nvm Node versions (manual), stale `node_modules` (review), Playwright/Puppeteer browsers (review) |
| Python | pip cache (safe), uv cache (safe), conda `conda clean --all` (safe), stale venvs (review), Hugging Face hub cache (manual) |
| JVM / Android | Gradle caches & wrapper dists (review), Maven `~/.m2` (review), Android AVDs & system images (manual) |
| Rust / Go / .NET | cargo registry/git caches (review), rustup toolchains (manual), stale `target/` (review), `go clean -cache` (safe), `go clean -modcache` (review), NuGet `dotnet nuget locals all --clear` (review) |
| AI / ML | Ollama models (manual, list with `ollama list`), LM Studio models (manual), HF cache (manual) |
| Editors / IDEs | VS Code/Cursor/Windsurf/Antigravity `Cache`, `CachedData`, `CachedExtensionVSIXs`, `GPUCache`, `logs` (safe, age-filtered; precondition app not running for full wipe), old extension versions (review), JetBrains caches/logs (safe) |
| Homebrew | `brew cleanup --prune=all` (safe) |
| Browsers / Electron apps | Chrome/Brave/Edge/Arc caches, Slack/Discord/Teams caches (review, app must be closed) |
| Embedded | PlatformIO packages (manual) |

### 4.6 Cleaning engine & safety

- `spacetrail clean` → builds a **plan** (dry-run is the default output): per cleaner, items, bytes, risk, method. Execute requires `--apply` (interactive confirm) or `--apply --yes` (non-interactive; refuses `review`/`manual` unless explicitly listed).
- **Trash by default** (via NSFileManager `trashItem`, recording the resulting Trash URL); `--permanent` is opt-in. Commands (docker, brew…) are logged as non-undoable.
- **Journal** in SQLite: action id, timestamp, cleaner, every path, bytes, method, Trash location, outcome. `spacetrail undo [action-id]` restores Trash-based actions. `spacetrail history` lists actions and total bytes freed.
- **Hard safety rails** (unit + property tests required):
  - Deny-list: `/`, `~`, `~/Library` root, `~/Documents|Desktop|Pictures|Movies|Music` roots, `/System`, `/Applications` root, `/usr`, `/Library` root, cloud storage roots, anything outside the cleaner's declared paths after canonicalization.
  - Never follow symlinks when deleting; re-validate paths right before deletion (TOCTOU).
  - Never touch dataless (cloud) files.
  - Respect `preconditions` (process not running, command exists).
  - Cap per run (`max_bytes`, `max_items`) for autoclean.
- **Autoclean**: launchd schedule (e.g. weekly), only `safe` cleaners the user enabled in config, writes journal, sends a summary notification ("Freed 8.4 GB: DerivedData 5.1 GB, npm cache 2.0 GB, …").

### 4.7 Daemon

- LaunchAgent (`~/Library/LaunchAgents/dev.spacetrail.agent.plist` or similar), installed via `spacetrail daemon install|uninstall|status|logs`; `launchctl bootstrap gui/$UID`. Low priority (`ProcessType=Background`, `LowPriorityIO`).
- Responsibilities: volume-stat sampling, scheduled tree snapshots, FSEvents incremental tracking, live monitor ring buffers, alerts, autoclean.
- Local API over a Unix domain socket (versioned JSON-RPC or similar) — used by the TUI for live data now, and by the future GUI/MCP server later. The CLI must still work without the daemon (direct DB reads + on-demand scan).
- Budgets: idle CPU < 1 %, RSS < 100 MB, no noticeable battery impact; pause heavy scans on battery/low-power mode (configurable).

### 4.8 CLI surface (all commands support `--json`)

```
spacetrail                      # opens the TUI
spacetrail status               # volume stats, growth 24h/7d/30d, forecast, top growers, daemon health
spacetrail scan [PATH]          # on-demand scan, stores a snapshot
spacetrail top [PATH] [--depth N] [--files N]
spacetrail diff --since 7d [PATH]
spacetrail timeline [PATH] --range 30d
spacetrail watch                # live hot paths + top writers
spacetrail clean [--risk safe|review|manual] [--cleaner ID]... [--apply] [--yes] [--permanent]
spacetrail cleaners list|show|test
spacetrail undo [ACTION_ID]
spacetrail history
spacetrail alerts
spacetrail report --html [--range 90d] [-o FILE]
spacetrail daemon install|uninstall|status|logs|run
spacetrail doctor               # FDA, daemon, DB integrity, FSEvents state, versions
spacetrail config get|set|edit
```

Human output: aligned tables, human-readable sizes (decimal GB to match Finder, with a flag for binary), colors that degrade gracefully (`NO_COLOR`), never color-only meaning.

### 4.9 TUI (ratatui)

Screens (tab bar, vim + arrow keys, `?` help overlay, mouse support optional):

1. **Overview** — KPI row (total / used / free / purgeable, Δ 24h / 7d / 30d, forecast days-to-full), **timeline chart of used space** with range switch (24h/7d/30d/90d/all), top 10 growers in the selected range, reclaimable space by risk class.
2. **Explorer** — ncdu-style tree: size, Δ over selected range, % bar, item count; enter to drill down; `t` opens that directory's timeline; `c` shows applicable cleaners; open in Finder / reveal.
3. **Timeline** — per-path history chart, compare up to 4 paths (fixed color order + legend + direct labels), crosshair cursor with values.
4. **Live** — hot paths and top writing processes with sparklines (needs daemon; explains how to start it otherwise).
5. **Cleanup** — cleaners grouped by risk, checkboxes, plan preview with bytes, confirm modal, progress, result summary.
6. **History** — cleaning journal with undo.

Charts: braille-marker line charts, one y-axis only, readable axis labels with dates, consistent palette, colorblind-safe, works in light and dark terminals. Snapshot-test the TUI with `insta` using a fake backend.

### 4.10 HTML report

`spacetrail report --html` → single self-contained file (inline JS/CSS, no CDN, works offline): KPIs, used/free timeline, top growers table, per-directory timelines, reclaimable by risk, cleaning history. Interactive hover tooltips. Light/dark. This is the bridge until the GUI exists.

---

## 5. Future (design for it, don't build it now)

- GUI: menu-bar app + window (Tauri or SwiftUI shell) consuming the daemon API; proper notifications; `SMAppService` login item.
- MCP server so AI agents can query disk usage and *propose* cleanups that the user approves.
- Linux support behind the platform trait.
- Duplicate-file finder, APFS clone-aware accounting, `getattrlistbulk` fast path.

---

## 6. Non-negotiable safety rules (put these in CLAUDE.md)

1. Tests never touch the real home directory — always `SPACETRAIL_HOME` + `tempfile`.
2. Every deletion path goes through the single executor with deny-list, canonicalization, symlink and TOCTOU checks. No `remove_dir_all` anywhere else in the codebase (enforce with a clippy `disallowed_methods` config).
3. Dry-run is the default; Trash is the default; `manual` risk is never auto-deleted.
4. Community/user cleaners cannot execute commands unless the user opts in.
5. Never read dataless (cloud) files; never require root.
6. No telemetry and no network calls — the tool works fully offline.

---

## 7. Quality, CI & community

- Tests: unit, property tests (size math, path safety), integration tests for **every** built-in cleaner against fixture trees, TUI snapshot tests, scanner benchmarks (`criterion`) on generated trees.
- CI (GitHub Actions, macOS arm64 runner; add Intel if available): fmt, clippy `-D warnings`, tests, `cargo deny` (licenses/advisories), MSRV check, build release artifacts on tags via `dist`.
- Release: `dist` → GitHub Releases + Homebrew tap (`<org>/homebrew-tap`); crates.io publish; signing/notarization step gated on secrets.
- Repo hygiene: README (pitch, GIF recorded with `vhs`, install, quickstart, FAQ incl. "why does Finder show a different number?" and FDA setup), `CONTRIBUTING.md`, **`docs/CONTRIBUTING_CLEANERS.md`** (step-by-step: copy template → `spacetrail cleaners test` → PR checklist with risk justification), `CODE_OF_CONDUCT.md`, `SECURITY.md`, issue templates (bug, new cleaner request, cleaner false-positive), PR template, `CHANGELOG.md` (Keep a Changelog), semantic versioning, `good first issue` labels (new cleaners are perfect first issues).

---

## 8. Suggested phases (refine in PLAN.md)

| Phase | Scope | Done when |
|---|---|---|
| 0 | Workspace scaffold, CI, licenses, CLAUDE.md, docs from §0 | CI green on an empty skeleton |
| 1 — MVP | Scanner (hard links, firmlinks, dataless, TCC-unreadable), volume stats, SQLite store, `status`, `scan`, `top`, `diff`, `timeline`, `doctor`, TUI Overview + Explorer with timeline chart | **Real read-only scan of my Mac with a clear "what's eating my disk" report** |
| 2 | Cleaner schema + loader + validator, executor with safety rails, Trash + journal + undo, first ~25 cleaners with tests, TUI Cleanup + History, `report --html` | Dry-run + apply + undo proven in sandbox tests |
| 3 | Daemon + launchd, scheduled snapshots, FSEvents incremental with persisted event IDs, rollups/retention, alerts, forecast, autoclean | Runs a week on my Mac within resource budgets |
| 4 | Live detection: FSEvents hot paths + per-process write rates, `watch`, TUI Live | Correctly flags a synthetic process writing GBs |
| 5 | Packaging & launch: `dist`, Homebrew tap, signing, README/GIF, contributor docs, v0.1.0 | `brew install <tap>/spacetrail` works on a clean Mac |

Start now with step §0.1: plan mode, produce the docs, list open questions, and wait for my go-ahead.