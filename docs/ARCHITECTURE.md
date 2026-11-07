# spacetrail — Architecture

Status: **draft for approval** (Phase 0). Covers the crate layout, data model, SQLite schema, the daemon ↔ CLI/TUI protocol, and which macOS APIs we use and why.

---

## 1. Baseline decisions

| Topic | Decision | Why |
|---|---|---|
| Rust edition | **2024** | It's the current edition, so a new project has nothing to migrate. Its defaults help with the amount of FFI we do: `unsafe extern` blocks and `unsafe_op_in_unsafe_fn` warn-by-default make unsafe boundaries explicit, and the stricter RPIT capture and `if let` scoping rules remove some footguns. |
| MSRV | **1.85** (first release with edition 2024), checked in CI. `rust-toolchain.toml` pins `stable` for development. | Lowest version edition 2024 allows. Raise it deliberately, not by accident. |
| Minimum macOS | **13 Ventura** | Every API we need is available there: `SF_DATALESS` (10.15), `NSURLVolumeAvailableCapacityForImportantUsageKey` (10.13), `proc_pid_rusage` V4 (10.13), FSEvents file events (10.7), `launchctl bootstrap` (10.10). 13 is also the oldest release still getting security updates in practice. Developed and tested on macOS 26. |
| Async runtime | **None.** Plain threads + `crossbeam-channel`. | The daemon has a handful of long-lived jobs (sampler, scanner, FSEvents run loop, socket server, live poller). Threads keep idle CPU near zero, keep backtraces simple, and drop a big dependency tree. A future GUI can still talk to the socket from any runtime. |
| Units | Allocated bytes by default; apparent size also stored. Decimal (SI) display by default to match Finder; `--binary` switches to IEC. | Matches what users see in Finder. |

## 2. Crate layout

```
spacetrail/
├── Cargo.toml                # [workspace], shared lints, [workspace.dependencies]
├── rust-toolchain.toml
├── clippy.toml               # disallowed_methods (deletion APIs), see §8
├── deny.toml                 # cargo-deny: licenses + advisories
├── crates/
│   ├── spacetrail-core/      # platform-agnostic: model, walker, aggregation, diff, forecast, paths, sizes
│   ├── spacetrail-macos/     # all unsafe/FFI: statfs, NSURL volume keys, APFS volumes, SF_DATALESS, firmlinks,
│   │                         # TCC probe, FSEvents, proc_pid_rusage, NSFileManager trash, launchd, notifications
│   ├── spacetrail-store/     # rusqlite (bundled): schema, migrations, writes, queries, rollups/retention
│   ├── spacetrail-cleaners/  # TOML schema, loader, validator, planner, executor (the ONLY deleter), journal, undo
│   ├── spacetrail-daemon/    # library: scheduler, monitors, alerts, autoclean, JSON-RPC server
│   ├── spacetrail-proto/     # JSON-RPC request/response types shared by daemon + clients (serde only)
│   └── spacetrail-cli/       # `spacetrail` binary: clap commands, ratatui TUI, HTML report, `daemon run`
├── cleaners/                 # built-in *.toml rules, embedded via include_str! (build.rs generates the list)
├── fixtures/                 # fixture-tree descriptions used by cleaner integration tests
├── docs/
└── xtask/                    # gen-fixture-tree, bench data, release helpers
```

Two changes from the brief:
- **`spacetrail-proto`** is split out so the future GUI and MCP server can depend on the wire types without pulling in the daemon.
- **There is no separate daemon binary.** The daemon is `spacetrail daemon run` from the same binary. That gives one artifact to sign, notarize, and grant Full Disk Access to. The FDA point matters: one TCC grant covers both the CLI and the agent.

### Dependency direction (enforced by Cargo; no cycles)

```
cli ──► daemon ──► cleaners ──► store ──► core
 │        │           │                    ▲
 │        └──► proto  └──► macos ──────────┘   (macos implements core's Platform trait)
 └──► proto, store, macos, core
```

`core` has no OS-specific code. It defines a `Platform` trait:

```rust
pub trait Platform: Send + Sync {
    fn volume_stats(&self, path: &Path) -> Result<VolumeStats>;
    fn container_breakdown(&self) -> Result<Vec<VolumeUsage>>;   // APFS: all volumes in container
    fn path_aliases(&self) -> &PathAliases;                      // firmlinks → display paths
    fn is_dataless(&self, meta: &Metadata) -> bool;
    fn probe_full_disk_access(&self) -> FdaStatus;
    fn unattributed_hints(&self) -> Result<Vec<UnattributedItem>>; // swap, sleepimage, TM snapshots, …
}
```

A future Linux port implements this trait. Tests use `FakePlatform`.

### Key dependencies (kept deliberately small)

| Crate | Use | Note |
|---|---|---|
| `clap` (derive) | CLI | standard |
| `ratatui` + `crossterm` | TUI | `TestBackend` for snapshot tests |
| `rusqlite` (`bundled`) | store | known SQLite version, no system dependency |
| `serde`, `serde_json`, `toml` | config, rules, RPC | |
| `rayon` | parallel walk | work-stealing `scope`; **custom walker** instead of `jwalk`, see §4 |
| `crossbeam-channel` | daemon plumbing | |
| `objc2`, `objc2-foundation` | NSURL resource values, `NSFileManager trashItem` | **not** the `trash` crate, because we need the resulting Trash URL for undo |
| `core-foundation` + thin hand-written FSEvents FFI | FSEvents | ~200 lines. Avoids the unmaintained `fsevent-sys`. |
| `libc` | statfs, lstat flags, proc_*, getattrlistbulk | |
| `globset` | cleaner path globs / excludes | |
| `time` | timestamps, durations | lighter than chrono |
| `tracing` + `tracing-subscriber` | logging | file appender for the daemon |
| `thiserror` (libs), `anyhow` (bin) | errors | |
| dev: `insta`, `tempfile`, `proptest`, `criterion` | tests/bench | |
| HTML report | **vendored uPlot** (MIT, ~50 KB) inlined via `include_str!` | Interactive, offline, no CDN. The license notice goes in the report. |

## 3. Domain model (spacetrail-core)

```rust
struct VolumeStats { total, free, available_important, available_opportunistic, purgeable_est, used }
struct VolumeUsage { name, role /*System|Data|VM|Preboot|Recovery|Update|Other*/, consumed }

struct DirAgg {           // one per directory during a scan
    path_id, parent, name,
    alloc_bytes, apparent_bytes,      // recursive totals
    files, dirs,                      // recursive counts
    unreadable_children: u32,         // EPERM/EACCES encountered below
    dataless_files: u64, dataless_logical_bytes: u64,  // cloud-only, never opened
}
struct TopFile { path, alloc_bytes, apparent_bytes, mtime }   // bounded min-heap, N=200

struct ScanResult {
    root, started_at, finished_at, volume: VolumeStats,
    tree: DirTree,                 // arena of DirAgg
    top_files: Vec<TopFile>,
    unreadable: Vec<(PathBuf, io::ErrorKind)>,   // "unknown size", never 0
    skipped_mounts: Vec<PathBuf>,
    hardlinks_deduped: u64,
}

struct Attribution {              // what `status` shows
    container_used,
    by_volume: Vec<VolumeUsage>,  // System, Preboot, Recovery, VM, Data …
    data_scanned, data_unreadable_dirs, data_unattributed, // = Data consumed − scanned
    hints: Vec<UnattributedItem>, // TM local snapshots, sleepimage, purgeable…
}
```

Pure, unit-tested logic in `core`: size math, hard-link dedupe, tree aggregation, diff (growers/shrinkers with abs + %), forecast (Theil–Sen slope, bootstrap interval, refuses to predict when the trend is flat or negative or the data is too noisy), human size formatting, duration parsing (`7d`, `24h`).

## 4. Scanner

- **Custom parallel walker** on `rayon::scope`: one task per directory, children are spawned as tasks, and leaf results fold upward into the arena. It's custom (not `jwalk`) because we need per-entry control: `lstat` flags (dataless), device checks, hard-link sets, TCC errors kept as data, and **no per-file allocation retained**.
- Per entry: `symlink_metadata` (lstat). We never follow symlinks and never `open()` regular files. That guarantees no cloud downloads: `lstat` doesn't materialize dataless files (verified in spike S1).
- `alloc = st_blocks * 512`. Files with `st_nlink > 1` go through a sharded `HashSet<(dev, ino)>`; the first one seen wins.
- `st_dev != root_dev` → skip it and record it in `skipped_mounts`.
- `st_flags & SF_DATALESS` → count in `dataless_*` and contribute 0 allocated.
- `EPERM`/`EACCES` on `readdir` → record as unreadable. The parent shows "+ unknown".
- **"Whole disk"** means: scan `/System/Volumes/Data` once, map display paths through the firmlink table (`/System/Volumes/Data/Users/x` → `/Users/x`), and report the other APFS volumes (System, VM, Preboot, Recovery) as fixed rows from `diskutil apfs list -plist` (or the IOKit equivalent, spike S3). Then container used = Σ volumes, and Data unattributed = Data consumed − scanned − known hints.
- **APFS clones**: counted at full allocated size for each clone. This is a documented over-count, and the UI says so.
- Memory: the arena holds one `DirAgg` per directory (~120 B + name). That's ~60–80 MB for a 500k-directory full disk in the CLI. Only a thresholded subset is persisted. The daemon's FSEvents rescans touch subtrees only.
- Phase 1 includes a `getattrlistbulk` spike (S4). If it's ≥ 1.5× faster, it becomes the enumeration backend behind the same walker interface.

## 5. SQLite schema (spacetrail-store)

WAL mode, `busy_timeout = 5000`, `foreign_keys = ON`. Migrations are numbered SQL files applied with `PRAGMA user_version`. Timestamps are Unix seconds (`INTEGER`). Sizes are `INTEGER` bytes.

```sql
-- ── volumes & cheap samples ──────────────────────────────────────────────
CREATE TABLE volumes (
  id INTEGER PRIMARY KEY, uuid TEXT UNIQUE NOT NULL, name TEXT, mount TEXT, role TEXT
);
CREATE TABLE volume_samples (            -- raw, every 5 min (daemon) / per command
  volume_id INTEGER NOT NULL REFERENCES volumes(id), ts INTEGER NOT NULL,
  total INTEGER, used INTEGER, free INTEGER, avail_important INTEGER, purgeable INTEGER,
  container_used INTEGER,
  PRIMARY KEY (volume_id, ts)
) WITHOUT ROWID;
CREATE TABLE volume_samples_hourly (volume_id, bucket INTEGER, used_min, used_max, used_avg, free_min, n,
  PRIMARY KEY (volume_id, bucket)) WITHOUT ROWID;
CREATE TABLE volume_samples_daily  (/* same shape */ PRIMARY KEY (volume_id, bucket)) WITHOUT ROWID;

-- ── tree snapshots ───────────────────────────────────────────────────────
CREATE TABLE paths (                     -- normalized, display form (firmlinks resolved)
  id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES paths(id),
  path TEXT UNIQUE NOT NULL, depth INTEGER NOT NULL, tracked INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE scans (
  id INTEGER PRIMARY KEY, root_path_id INTEGER NOT NULL REFERENCES paths(id),
  started_at INTEGER, finished_at INTEGER, kind TEXT /* full|incremental|on_demand */,
  scanned_alloc INTEGER, files INTEGER, dirs INTEGER, unreadable INTEGER, dataless INTEGER,
  volume_used INTEGER, tier TEXT NOT NULL DEFAULT 'raw' /* raw|daily|weekly after rollup */
);
-- Change-only storage: a row exists when a path's size moved by ≥ max(1 MB, 0.5 %) since the
-- last stored row for that path, or when the path appears/disappears. Size-at-scan(S) = latest
-- row with scan_id ≤ S. This keeps DB growth far under 50 MB/yr (most dirs don't change).
CREATE TABLE dir_sizes (
  path_id INTEGER NOT NULL REFERENCES paths(id), scan_id INTEGER NOT NULL REFERENCES scans(id),
  alloc INTEGER NOT NULL, apparent INTEGER NOT NULL, files INTEGER, gone INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (path_id, scan_id)
) WITHOUT ROWID;
CREATE INDEX dir_sizes_by_scan ON dir_sizes(scan_id);
CREATE TABLE top_files (scan_id INTEGER, path TEXT, alloc INTEGER, mtime INTEGER,
  PRIMARY KEY (scan_id, path)) WITHOUT ROWID;
CREATE TABLE unreadable (scan_id INTEGER, path_id INTEGER, errno INTEGER, PRIMARY KEY (scan_id, path_id));

-- ── FSEvents state ───────────────────────────────────────────────────────
CREATE TABLE fsevents_state (volume_uuid TEXT PRIMARY KEY, last_event_id INTEGER, updated_at INTEGER);

-- ── cleaning journal ─────────────────────────────────────────────────────
CREATE TABLE clean_actions (
  id INTEGER PRIMARY KEY, started_at INTEGER, finished_at INTEGER,
  trigger TEXT /* interactive|autoclean */, dry_run INTEGER, bytes_freed INTEGER, status TEXT
);
CREATE TABLE clean_items (
  id INTEGER PRIMARY KEY, action_id INTEGER REFERENCES clean_actions(id),
  cleaner_id TEXT, cleaner_version TEXT, risk TEXT, method TEXT /* trash|permanent|command */,
  original_path TEXT, trash_path TEXT, bytes INTEGER, outcome TEXT /* done|skipped|failed|restored */,
  reason TEXT, command_argv TEXT /* JSON */, command_exit INTEGER, ts INTEGER
);

-- ── alerts ───────────────────────────────────────────────────────────────
CREATE TABLE alerts (id INTEGER PRIMARY KEY, ts INTEGER, rule_id TEXT, severity TEXT,
  message TEXT, payload TEXT /* JSON */, delivered INTEGER, suppressed_reason TEXT);
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);   -- schema info, daemon heartbeat, etc.
```

**What gets persisted per scan:** directories with alloc ≥ `persist_threshold` (default 50 MB) up to `persist_depth` (default 8), **plus** the always-tracked set: every top-level home directory, `~/Library/*`, `~/Library/Containers/*`, `~/Library/Application Support/*`, `/Applications`, and every path a cleaner references (`paths.tracked = 1`). Tracked paths always get a row on the first scan and on change.

**Retention / rollups (configurable):** volume samples: raw 7 d → hourly 90 d → daily forever. Tree scans: all for 7 d → one per day (the last of the day) for 90 d → one per week after that, as `scans.tier`. Because dir_sizes are change-only, dropping a scan means *merging* its rows forward into the next kept scan, never just deleting them. That code gets property tests: "size-at(T) is identical before and after rollup for every kept T".

**Budget estimate:** ~3–5k persisted paths × ~5 % changing per scan × 4 scans/day ≈ 1k rows/day × ~30 B ≈ 11 MB/yr in the worst steady case, plus the paths table. That's comfortably under 50 MB/yr. It gets verified with a 1-year synthetic history in the Phase 3 tests.

## 6. Daemon ↔ clients protocol (spacetrail-proto)

- **Transport:** Unix domain socket at `$DATA_DIR/daemon.sock`. The directory is `0700` and the socket is `0600`. Each connection checks `getpeereid()` and rejects any uid other than the owner.
- **Framing:** newline-delimited JSON (one JSON-RPC 2.0 message per line).
- **Versioning:** the first call must be `hello {client, protocol: 1}`. The reply carries `{daemon_version, protocol, capabilities[]}`. Breaking changes bump `protocol`. Additive fields are allowed within a version (unknown fields are ignored).
- **Methods (v1):**

| Method | Purpose |
|---|---|
| `hello` | handshake |
| `status.get` | volume stats, attribution, growth 24h/7d/30d, forecast, daemon health |
| `scan.request {path?, kind}` | enqueue a scan → returns `job_id` |
| `job.get {job_id}` | progress |
| `live.subscribe {window}` / `live.unsubscribe` | server pushes `live.update` notifications (hot paths, top writers) every 1 s |
| `alerts.list {since?}` | alert history |
| `autoclean.run_now {dry_run}` | trigger a safe-only autoclean |
| `daemon.reload_config` | reload config |

- **The CLI works without the daemon.** The CLI reads SQLite directly (WAL makes concurrent reads safe) and runs scans in-process. The socket is only *required* for `watch` and the TUI Live screen, which need ring buffers that only exist in the daemon. Writes from both sides go through `spacetrail-store`, which serializes them with `BEGIN IMMEDIATE`.

## 7. macOS APIs used and why

| Need | API | Why this one |
|---|---|---|
| Per-file sizes, flags | `lstat` (`st_blocks`, `st_flags`, `st_dev`, `st_ino`, `st_nlink`) | Cheap. Never opens files, so it never triggers File Provider downloads. |
| Fast enumeration (spike) | `getattrlistbulk(2)` | One syscall returns many entries with attributes |
| Dataless detection | `st_flags & SF_DATALESS` (0x40000000) | Official marker for File Provider / iCloud placeholders |
| Volume capacity | `statfs` + `NSURL` resource keys `volumeAvailableCapacityForImportantUsage` / `…OpportunisticUsage` / `volumeTotalCapacity` (objc2-foundation) | The "important usage" figure includes purgeable space and is what Finder shows. `statfs` free doesn't. We show both and explain the difference. |
| APFS per-volume usage | `diskutil apfs list -plist` (spike S3: IOKit `IOMedia`/APFS properties as a no-subprocess alternative) | Required to attribute System/VM/Preboot/Recovery |
| Firmlinks | parse `/usr/share/firmlinks` | Display-path mapping, avoids double counting |
| Local TM snapshots | `tmutil listlocalsnapshots /` (read), `tmutil thinlocalsnapshots` (cleaner) | No public API |
| Full Disk Access probe | try to open a known TCC-protected path (e.g. `~/Library/Safari`, `~/Library/Application Support/com.apple.TCC/TCC.db`) | There's no API to query TCC for yourself. Probing is the standard technique. |
| Incremental + hot paths | FSEvents (`FSEventStreamCreate` with `kFSEventStreamCreateFlagFileEvents`, `…WatchRoot`, `sinceWhen`, `FSEventsGetCurrentEventId`, `FSEventsCopyUUIDForDevice`) | Kernel-maintained history allows replay after a restart. No root needed. |
| Per-process writes | `proc_listpids` + `proc_pid_rusage(RUSAGE_INFO_V4).ri_diskio_byteswritten` | Works for same-uid processes without root. `fs_usage` (root) is documented as an advanced alternative only. |
| Trash with undo | `-[NSFileManager trashItemAtURL:resultingItemURL:error:]` | Returns the real location in Trash, which undo needs |
| Safe permanent delete | `openat(O_NOFOLLOW│O_DIRECTORY)` + `unlinkat` recursive, relative to dir fds | Closes symlink and TOCTOU races (see §8) |
| Notifications | `osascript -e 'display notification …'` (argv, not shell) | Works without an app bundle. A proper `UNUserNotificationCenter` comes with the GUI. |
| Agent install | LaunchAgent plist + `launchctl bootstrap gui/$UID` / `bootout` / `print` | Standard per-user agent. `ProcessType=Background`, `LowPriorityIO=true`, `Nice=10`. |
| Power state | `IOPSCopyPowerSourcesInfo` / `NSProcessInfo.lowPowerModeEnabled` | Pause heavy scans on battery or Low Power Mode |

## 8. Cleaning executor & safety (spacetrail-cleaners)

All deletion goes through `Executor::execute(plan)`. Nothing else in the workspace can delete:

- `clippy.toml` `disallowed-methods`: `std::fs::remove_dir_all`, `std::fs::remove_dir`, `std::fs::remove_file`, `libc::unlink*`, `libc::rmdir`, `std::process::Command::new` (command execution is also centralized). Only `spacetrail-cleaners::executor` carries a scoped `#[expect(clippy::disallowed_methods)]`, plus one read-only exception: `spacetrail-macos/src/probe.rs`, which can spawn only a fixed enum of read-only commands (`diskutil apfs list -plist`, `tmutil listlocalsnapshots /`) with absolute paths and a cleared environment. `cargo xtask check-safety` fails CI if the attribute appears anywhere else.

Pipeline for every item:

1. **Resolve.** Expand variables and globs from the rule → candidate paths. Canonicalize the *parent* with `realpath`, keep the leaf name, and never resolve a leaf symlink.
2. **Containment.** The canonical path must sit strictly inside a canonicalized declared root of that rule (the glob's non-wildcard prefix), and must not *equal* that root unless the action is `delete_contents` of its children.
3. **Deny-list** (hard-coded, not configurable): `/`, `$HOME`, `~/Library`, `~/Documents|Desktop|Pictures|Movies|Music|Downloads` (roots), `~/Library/CloudStorage/**`, `~/Library/Mobile Documents/**`, `/System/**`, `/Applications` (root), `/usr/**` (except `/usr/local/**` child items named by a rule), `/Library` (root), `/private` (roots), `/Volumes` (root), any mount point. Property-tested with generated paths that include `..`, symlinks, unicode normalization, and case variants (APFS is case-insensitive by default).
4. **Pre-execution re-validation (TOCTOU).** Open the parent with `O_NOFOLLOW|O_DIRECTORY`, `fstatat(AT_SYMLINK_NOFOLLOW)` the leaf, and check that dev/ino match the plan. A symlink leaf → trash or unlink *the link only*. Any dataless descendant found during measurement → skip the item with the reason "contains cloud-only files".
5. **Preconditions** (process not running, command exists) are re-checked right before execution.
6. **Method.** `trash` (default) → `NSFileManager`, and record `trash_path`. `permanent` (opt-in) → recursive `unlinkat` over dir fds. `command` → `Command` with an argv array, an env allow-list, a timeout, and captured stdout/stderr.
7. **Journal.** Write a `clean_items` row per item (done/skipped/failed). Write bytes freed as measured *before* deletion; the post-run volume delta is shown alongside.
8. **Caps.** `max_bytes` / `max_items` per run, mandatory for autoclean.

**Undo:** for `trash` items, move `trash_path` back to `original_path`. Refuse if the destination now exists or the trash item is gone. Commands and permanent deletes are marked non-undoable in the plan *before* the user confirms.

## 9. Config & paths

```
$SPACETRAIL_HOME           (if set, everything below lives under it: config/, data/, cleaners/)
config:   ${XDG_CONFIG_HOME:-~/.config}/spacetrail/config.toml
cleaners: ${XDG_CONFIG_HOME:-~/.config}/spacetrail/cleaners/*.toml
data:     ~/Library/Application Support/spacetrail/{spacetrail.db, daemon.sock, logs/}
agent:    ~/Library/LaunchAgents/io.github.<owner>.spacetrail.agent.plist   (label in one constant)
```

`SPACETRAIL_HOME` also redefines `~` for cleaner path expansion **in tests and in `cleaners test`**. That way a sandboxed cleaner test can never resolve to the real home directory. A debug assertion in the executor refuses to run under `cfg(test)` unless `SPACETRAIL_HOME` is set.

## 10. TUI & report architecture

- The TUI reads through a `DataSource` trait: `DbSource` (direct SQLite), `DaemonSource` (socket, used for Live), and `FakeSource` (for tests). Rendering is pure: `fn render(state, frame)`. `insta` snapshots `TestBackend` buffers at fixed sizes (80×24, 120×40).
- Charts: ratatui `Chart` with Braille markers, one y-axis, date labels. A fixed series palette (Okabe–Ito: orange, sky blue, bluish green, vermillion) plus line glyph or label differences, so meaning never relies on color alone.
- `report --html`: the same query layer → a JSON blob embedded in a template with inlined CSS/JS (uPlot) → one file, with `prefers-color-scheme` light/dark.

## 11. Future-proofing

- GUI/MCP talk to `spacetrail-proto` over the socket. Methods that propose cleanups return *plans*. Applying a plan requires a separate, user-confirmed call (the MCP server will never get an auto-apply method).
- Linux: implement `Platform`, swap FSEvents for inotify/fanotify behind a `ChangeSource` trait, and swap the Trash implementation for the freedesktop spec.
