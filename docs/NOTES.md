# Engineering notes & spike findings

Findings from experiments on real macOS behavior. Every entry: date, machine, what was tested, result, consequence for the design. Do not guess — add an entry.

---

## 2026-10-01 — Environment baseline (author's machine, read-only)

**Machine:** Apple Silicon (arm64), macOS 26, a single APFS container.

**Toolchain:** the installed rustc was too old for edition 2024 (needs ≥ 1.85). A `rust-toolchain.toml` pinned to `stable` will make rustup fetch a current toolchain on first build.

**APFS container usage (`diskutil apfs list`, no root needed):**

| Volume | Role | Consumed |
|---|---|---|
| Macintosh HD | System (sealed, read-only) | 12.6 GB |
| Preboot | Preboot | 10.8 GB |
| Recovery | Recovery | 2.6 GB |
| Data | Data | 405.3 GB |
| VM | VM (swap) | **33.3 GB** |

`df -h` showed the Data volume nearly full. `/private/var/vm/sleepimage` is a few GB. `tmutil listlocalsnapshots /` returned no local snapshots.

**Consequences:**
- `statfs` per-volume "used" ≠ container used. "Whole disk" accounting must be **container-level**: sum of every volume's consumed capacity, then break down Data by scan and list the other volumes as fixed rows. `diskutil apfs list -plist` works without root and gives per-volume consumption. A Phase 1 spike will check whether we can get the same numbers via `statfs`/IOKit without spawning `diskutil`.
- The VM volume (33 GB of swap on a machine with less RAM than that) is a real, user-visible consumer. It is not deletable, but it is explainable: memory pressure from Docker/VMs/LLMs. It belongs in the "unattributed → explained" breakdown, and the Live view should correlate it with memory-hungry processes later.

**Firmlinks (`/usr/share/firmlinks`):** 19 entries including `/Users`, `/Applications`, `/Library`, `/private`, `/opt`, `/usr/local`, `/Volumes`, several `/System/Library/*` asset dirs. Plan: scan `/System/Volumes/Data` once, don't cross devices, and present paths without the `/System/Volumes/Data` prefix (via the firmlink table).

**Dev tooling** (calibrates the built-in catalog): a typical developer Mac has Docker, Xcode, JS package managers (npm/yarn/pnpm/bun), Python (pip/conda/uv), Rust, Go, .NET, JVM build tools (often wrapper-only, not on PATH), local LLM runtimes, Android SDK and several Electron/VS Code-based editors. Tools can be absent from PATH while their data dirs still exist, so detection checks both.

## 2026-10-01 — Name availability

| Check | Result | Consequence |
|---|---|---|
| crates.io `spacetrail` | 404 (free) | Reserve early (publish 0.0.0 placeholder?) — ask author |
| GitHub `github.com/spacetrail` | 200 (**taken** user/org) | Host the repo under the author's account/org |
| Homebrew core `spacetrail` | 404 (free) | Tap is fine; core later is possible |
| Homebrew core `st` | 200 (**taken**, a statistics CLI) | **No `st` alias** |
| crates.io `st` | 200 (taken) | Confirms no `st` alias |

## Open spikes (to be filled in during the phases)

- [ ] S1 (Phase 1): `st_flags & SF_DATALESS` on iCloud Drive + a File Provider (`~/Library/CloudStorage/*`) — confirm `lstat` alone never materializes the file.
- [ ] S2 (Phase 1): TCC — which `~/Library` subdirs return `EPERM` without FDA on macOS 26; how to detect FDA reliably (probe `~/Library/Safari/Bookmarks.plist` / TCC.db readability).
- [ ] S3 (Phase 1): Volume capacity — `NSURLVolumeAvailableCapacityForImportantUsageKey` vs `statfs` vs `diskutil`; how to get purgeable.
- [ ] S4 (Phase 1): Walker throughput — `readdir`+`lstat` vs `getattrlistbulk` on ~1M files.
- [ ] S5 (Phase 3): FSEvents without root — are events for TCC-protected paths delivered? `sinceWhen` replay after reboot? Event-ID stability per volume UUID.
- [ ] S6 (Phase 3): TCC identity of the daemon binary when unsigned vs signed; does FDA survive a `brew upgrade` (Cellar path changes)?
- [ ] S7 (Phase 4): `proc_pid_rusage(RUSAGE_INFO_V4)` for own-uid processes, including `com.apple.Virtualization.VirtualMachine` (Docker's VM).
- [ ] S8 (Phase 2): `NSFileManager trashItemAtURL:resultingItemURL:` — returned URL is stable for undo; behavior on external volumes / name collisions.

## 2026-10-01 — M1.1 walker throughput (author's machine, M-series)

`cargo bench -p spacetrail-core --bench scan` with `SPACETRAIL_BENCH_FILES=200000` (empty files, 100 per dir, 10 subdirs per dir, warm cache): **170 ms per scan ≈ 1.17 M files/s**. Extrapolated, 1 M files take ≈ 0.9 s. The brief's target (≥ 1 M files in < 60 s) is met with a wide margin on synthetic trees. Real trees (deeper, larger dirs, cold cache) are measured in M1.3. `getattrlistbulk` (S4) is therefore an optimization, not a requirement.

Design note (dataless directories): a cloud-only *directory* is not listed at all. On File Provider domains, `readdir` on a dataless directory can itself trigger materialization of the listing. Spike S1 must also evaluate `setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_PROCESS, IOPOL_MATERIALIZE_DATALESS_FILES_OFF)` as a process-wide belt-and-braces guarantee: with it, any accidental access fails with an error instead of downloading.
