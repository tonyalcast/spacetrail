# spacetrail — Implementation plan

Status: **draft for approval**. Each milestone ends with: tests green, `cargo clippy --all-targets -- -D warnings` clean, `cargo fmt`, one conventional commit (or a few), and a short summary (what changed / what's next). Spikes record their results in `docs/NOTES.md`.

Legend: **AC** = acceptance criteria.

---

## Phase 0 — Scaffold (≈ 1 session)

### M0.1 Workspace & tooling
- Cargo workspace with the 7 crates from ARCHITECTURE §2 (empty `lib.rs` + one smoke test each), `xtask`, `rust-toolchain.toml` (stable + clippy + rustfmt), workspace lints (`unsafe_op_in_unsafe_fn`, `missing_debug_implementations` for libs, clippy pedantic subset).
- `clippy.toml` with `disallowed-methods` for deletion and process APIs, and `deny.toml`.
- `LICENSE-MIT`, `LICENSE-APACHE`, `README.md` stub, `CHANGELOG.md`, `.gitignore`, `.editorconfig`.
- **AC:** `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo deny check` all pass locally. Calling `std::fs::remove_dir_all` in any crate fails clippy (verified with a throwaway test).

### M0.2 CI
- GitHub Actions: `macos-latest` (arm64) job with fmt, clippy, test, deny, plus an MSRV job (1.85) and a grep guard on `expect(clippy::disallowed_methods)`. Add an Intel job if a GitHub Intel macOS runner label is still available (`macos-13`/`macos-15-intel`); otherwise run `cargo check --target x86_64-apple-darwin` on arm64.
- **AC:** CI green on the empty skeleton (needs a remote, see Q1).

---

## Phase 1 — MVP: read-only insight

The author's disk is at 93 %, so the "what is eating my disk" answer is moved **as early as possible** (M1.3), before the DB and TUI are built.

### M1.1 Core model & walker
- `spacetrail-core`: sizes, duration parsing, human formatting (SI/IEC), `DirTree` arena, parallel walker (lstat-only, no symlink follow, device boundary, hard-link dedupe, top-N files heap, unreadable tracking, dataless counting through the `Platform` trait).
- `xtask gen-tree` generates synthetic trees (N files, depth, hard links, symlink loops, unreadable dirs).
- **AC:** unit + proptest: totals equal the sum of unique `(dev,ino)` allocations; hard links counted once; symlink loops terminate; an unreadable dir is reported and not counted as 0; walking a tree with a dataless-flagged stub (faked through the Platform trait) never calls `open`. Criterion bench on 1M generated files exists and runs in < 60 s warm (report the number).

### M1.2 macOS platform layer + spikes S1–S4
- `spacetrail-macos`: `statfs`, NSURL capacity keys, APFS volume breakdown, firmlink table + display mapping, `SF_DATALESS`, FDA probe, unattributed hints (VM swap volume, sleepimage, `tmutil listlocalsnapshots`, `/private/var/folders` size, Docker.raw allocated size).
- Spikes S1 (dataless), S2 (TCC/FDA), S3 (capacity numbers), S4 (`getattrlistbulk`), all documented in NOTES.md.
- **AC:** on the author's Mac, `cargo run -- doctor` (temporary minimal CLI) prints FDA status and a container breakdown matching `diskutil apfs list` within 1 %. Temp-dir tests cover firmlink mapping. The dataless flag can't be set without a File Provider, so it's covered by spike S1 on real iCloud files plus the Platform-trait fake.

### M1.3 ★ First real scan of the author's Mac (read-only, no DB)
- Minimal `spacetrail scan` / `spacetrail top` (in-memory) and `status` (volume stats + attribution).
- Run it on the real machine (read-only, explicitly allowed by the brief) and write **`docs/reports/2026-xx-xx-first-scan.md`**: container breakdown, top 30 directories, top 30 files, unreadable list (FDA hint), cloud-only totals, unattributed explanation, and a list of "likely reclaimable" items by risk with the manual commands, but **nothing executed**.
- **AC:** report delivered; the numbers sum up (scanned + unreadable-known + other volumes + unattributed = container used).

### M1.4 Store & history
- `spacetrail-store`: migrations, schema from ARCHITECTURE §5, change-only `dir_sizes`, path normalization, tracked-path set, volume samples, queries: `latest_scan`, `size_at`, `diff(from,to,path,depth)`, `series(path, range)`.
- `scan` persists; `status` shows Δ 24h/7d/30d once ≥ 2 scans exist.
- **AC:** proptest: for random sequences of trees, `size_at(path, scan)` from the DB equals the tree value at that scan. A DB for 1 year of synthetic 6-hourly scans of a 5k-dir tree stays < 50 MB.

### M1.5 CLI surface (Phase 1 commands)
- `status`, `scan [PATH]`, `top [PATH] [--depth] [--files]`, `diff --since|--from/--to [PATH]`, `timeline [PATH] --range` (braille chart in terminal), `doctor`, `config get|set|edit`. `--json` everywhere, `NO_COLOR`, `--binary`.
- **AC:** `insta` snapshot tests for human and JSON output of each command against a fixture DB; `SPACETRAIL_HOME` respected (a test fails if any path under the real `$HOME` is touched: tests run with `HOME` pointed at a temp dir too).

### M1.6 TUI: Overview + Explorer
- App shell (tabs, vim/arrow keys, `?` help), Overview (KPIs, used-space timeline with range switch, top growers, reclaimable placeholder), Explorer (tree, Δ, % bar, drill-down, `t` → timeline, reveal in Finder via `open -R` through the executor's command runner).
- **AC:** snapshot tests at 80×24 and 120×40 on a `FakeSource`; manual run on the real DB.

**Phase 1 done when:** a real read-only scan of the author's Mac with a clear "what's eating my disk" report (M1.3, refreshed after M1.6 with history and the TUI).

---

## Phase 2 — Cleaners & safe execution

> **Reordered 2026-11-05:** with the author's disk at 99 %, the read-only half of Phase 2 (schema, loader, validator, planner, catalog and `spacetrail suggest`) was delivered right after M1.3, before M1.4. Still pending in Phase 2: the executor (`clean --apply`), journal/undo, per-cleaner fixtures in CI, TUI Cleanup/History and `report --html`.

### M2.1 Schema, loader, validator
- `CLEANER_SPEC.md` v1 implemented with serde `deny_unknown_fields`, path pattern engine, deny-list, risk/action matrix. `cleaners list|show|test`.
- **AC:** golden tests for every validation error; proptest for deny-list/containment (`..`, symlinks, case, NFD/NFC).

### M2.2 Planner
- Detect → measure (reuses the walker, dataless-aware) → age filter → preconditions → plan with items, bytes, risk, method, undoability, blocked reasons.
- **AC:** plan snapshot tests on fixture trees; the plan is identical between two runs (deterministic ordering).

### M2.3 Executor, journal, undo
- Trash via NSFileManager (spike S8), permanent via `unlinkat` over dir fds, command runner (argv, timeout, env allow-list), TOCTOU re-validation, caps, journal, `undo`, `history`.
- **AC (sandbox only):** apply → files in the sandbox Trash, journal rows correct → undo restores them byte-identical. A symlink-swap race test (replace a dir with a symlink to an outside dir between plan and execute) deletes nothing outside. `--yes` refuses `review` unless `--cleaner` is named; `manual` never executes.
- *Trash in tests:* the real `~/.Trash` is outside the sandbox. Tests use a `TrashBackend` trait with a sandbox implementation. The NSFileManager backend is tested in one dedicated, opt-in test (`#[ignore]`, run manually) on a temp volume/dir that the author approves.

### M2.4 Built-in catalog (~25 cleaners, each with fixtures + integration test)
Priority order, matching the author's machine: Docker (build cache, system prune, Docker.raw report) · Xcode (DerivedData, CoreSimulator caches, unavailable sims, DeviceSupport, Archives, runtimes) · Gradle/Android (caches, wrapper dists, AVDs, system images) · Node (npm, yarn, pnpm, bun, nvm versions, project `node_modules`) · Python (pip, uv, conda, HF cache, venvs) · Rust (cargo registry/git, project `target`, rustup toolchains) · Go caches · NuGet · Ollama / LM Studio · VS Code/Cursor/Windsurf/Antigravity caches & logs · JetBrains · Homebrew · `~/Library/Caches` age-filtered · Logs · DiagnosticReports · Trash > 30d · Downloads installers > 30d · PlatformIO.
- **AC:** every cleaner passes `cleaners test` in CI; paths verified (read-only) against the author's machine and the result noted in the cleaner's `docs` or NOTES.

### M2.5 TUI Cleanup + History, `report --html`
- **AC:** snapshot tests; `report --html` produces one file < 1 MB that works offline (CI checks it has no external `src=`/`href=`).

**Phase 2 done when:** dry-run → apply → undo proven in sandbox tests; then (with explicit approval) one dry-run plan shown for the real machine.

---

## Phase 3 — Daemon

- M3.1 `daemon run` (threads: sampler, scheduler, socket server), JSON-RPC v1, `daemon install|uninstall|status|logs` (LaunchAgent, `launchctl bootstrap gui/$UID`).
- M3.2 FSEvents incremental (spike S5): persisted event IDs per volume UUID, replay with `sinceWhen`, fallbacks for `MustScanSubDirs` / `RootChanged` / dropped events / UUID change.
- M3.3 Rollups & retention (merge-forward property tests), forecast (Theil–Sen + interval, honest "insufficient data").
- M3.4 Alerts (rules, debounce, quiet hours, osascript notifications, `alerts`), autoclean (safe + enabled + caps + summary notification), power-aware throttling.
- Spike S6: FDA identity of the daemon binary (signed vs unsigned, brew upgrade).
- **AC:** integration tests with a fake clock and fake FSEvents source; on the real machine, a week of running with idle CPU < 1 %, RSS < 100 MB (measured by `spacetrail doctor` reading its own rusage history).

## Phase 4 — Live detection

- M4.1 Hot paths: FSEvents file events → rate-limited coalesced `lstat` → per-directory byte deltas over 1m/10m/1h ring buffers (bounded last-size LRU).
- M4.2 Top writers: `proc_pid_rusage` polling every 2 s, per-pid deltas, process-name resolution, aggregate by app bundle (spike S7).
- M4.3 `watch` + TUI Live (sparklines), correlation text.
- **AC:** a test helper process writes 2 GB into a sandbox dir; `watch --json` names that process and that dir as the top entries within 10 s.

## Phase 5 — Packaging & launch

- `dist` config (arm64 + x86_64; universal via `lipo` post-step if `dist` doesn't support it natively), Homebrew tap formula, crates.io publish, signing + notarization job gated on secrets, README + `vhs` GIF + FAQ, CONTRIBUTING, `docs/CONTRIBUTING_CLEANERS.md`, CODE_OF_CONDUCT, SECURITY, issue/PR templates, labels, `v0.1.0`.
- **AC:** `brew install <tap>/spacetrail` works on a clean macOS VM/user account.

---

## Decisions (answered 2026-10-01)

| # | Topic | Decision |
|---|---|---|
| Q1 | Git remote | **Local only for now.** The remote is added later; CI (M0.2) is written but not run until then. |
| Q2 | Bundle / agent identifier | **`io.github.<owner>.spacetrail`**. `<owner>` is fixed when the remote exists; it's centralized in one constant. |
| Q3 | `manual` risk items | **Both modes, selectable**: `cleaners.manual_mode = "explain"` (default, measure and explain only) or `"trash_per_item"` (TUI only, one item at a time, Trash only, typed confirmation of the item name; never permanent, never `--yes`). Overridable per session from the TUI Cleanup screen. |
| Q4 | Toolchain | **`rustup update stable`** approved; edition 2024, MSRV 1.85. |
| Q5 | crates.io name | **Don't publish until Phase 5.** |
| Q6 | Project code roots | **All of them, selectable when needed**: default `projects.roots = ["~/Documents/dev", "~/Code", "~/Developer", "~/Projects", "~/src", "~/dev"]`; `--roots <PATH>...` per run; `projects.scope = "roots" \| "home"` (whole home, slower, always excluding `~/Library` and cloud storage); TUI root picker. |
| Q7 | Full Disk Access for the first real scan | **Scan without FDA first**, then show what was unreadable. |
| Q8 | HTML report charts | **Vendored uPlot** (MIT, inlined). |

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| **TCC identity of unsigned binaries**: FDA is granted per path for unsigned binaries, and Homebrew upgrades change the Cellar path, so the daemon may silently lose FDA after `brew upgrade`. | Daemon scans become incomplete | Spike S6. Point the LaunchAgent at the stable `opt` symlink path. `doctor` and the daemon detect FDA loss and alert. Developer ID signing gives a stable designated requirement. **Strong argument for getting the Apple Developer ID before v0.1.** |
| APFS accounting is inherently fuzzy (clones, snapshots, purgeable, sealed system volume) | Users distrust numbers | Container-level reconciliation table. Never hide the unattributed remainder. Explicit "why Finder differs" FAQ. |
| FSEvents edge cases (dropped events, history purge, volume UUID change) | Stale history | Every ambiguous flag → rescan of the affected root. Periodic full scan (default every 6 h) as a backstop. |
| A bug in the executor deletes the wrong thing | Catastrophic | Single executor, clippy-enforced. Deny-list + containment + TOCTOU + no-symlink-follow. Proptests. Trash by default. Sandboxed tests only. Real `clean --apply` only after you approve a dry-run. |
| Tool-native commands change behavior (`docker`, `brew`, `conda`) | Cleaner breaks | Versioned rules, `ok_exit_codes`, captured output in the journal, `cleaners test` with stubbed commands in CI. |
| 1M files < 60 s target on a full 405 GB data volume (4.4M inodes) | Slow first scan | Parallel walker + `getattrlistbulk` spike. A full-disk scan is expected to be several minutes; the 60 s target is per 1M files. Progress UI. |
| macOS 26 behavior changes vs macOS 13 (min target) | Untested on old OS | CI on the oldest available runner. Document "tested on". |
| GitHub Intel macOS runners being phased out | No native x86_64 CI tests | Cross-compile check + Rosetta-run of tests on arm64 if feasible. |

## Out of scope for v0.1
GUI, MCP server, Linux, duplicate finder, clone-aware accounting, `fs_usage` root mode (documented only).
