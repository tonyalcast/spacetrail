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
- [x] S6: TCC identity of a signed app across rebuilds (see 2026-11-05 entry). Still open: `brew upgrade` path changes for the CLI/daemon.
- [ ] S7 (Phase 4): `proc_pid_rusage(RUSAGE_INFO_V4)` for own-uid processes, including `com.apple.Virtualization.VirtualMachine` (Docker's VM).
- [ ] S8 (Phase 2): `NSFileManager trashItemAtURL:resultingItemURL:` — returned URL is stable for undo; behavior on external volumes / name collisions.

## 2026-10-01 — M1.1 walker throughput (author's machine, M-series)

`cargo bench -p spacetrail-core --bench scan` with `SPACETRAIL_BENCH_FILES=200000` (empty files, 100 per dir, 10 subdirs per dir, warm cache): **170 ms per scan ≈ 1.17 M files/s**. Extrapolated, 1 M files take ≈ 0.9 s. The brief's target (≥ 1 M files in < 60 s) is met with a wide margin on synthetic trees. Real trees (deeper, larger dirs, cold cache) are measured in M1.3. `getattrlistbulk` (S4) is therefore an optimization, not a requirement.

Design note (dataless directories): a cloud-only *directory* is not listed at all. On File Provider domains, `readdir` on a dataless directory can itself trigger materialization of the listing. Spike S1 must also evaluate `setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_PROCESS, IOPOL_MATERIALIZE_DATALESS_FILES_OFF)` as a process-wide belt-and-braces guarantee: with it, any accidental access fails with an error instead of downloading.

## 2026-10-01 — M1.2 spikes S1–S3 (author's machine, no Full Disk Access)

Run with `cargo run --release -p spacetrail-macos --example spikes -- s1|s2|s3`.

**S3 — capacity sources.** All of these agree to within rounding:

| Source | `/` and `/System/Volumes/Data` |
|---|---|
| `statfs` total | 494 GB (container size) |
| `statfs` used = `(f_blocks − f_bfree)·bsize` | 440 GB, the **container** used, identical for every volume of the container (not per-volume!) |
| `statfs` free (`f_bavail`) | 54.7 GB |
| `NSURLVolumeAvailableCapacityForImportantUsageKey` | 56.2 GB (≈ what Finder shows) |
| `…ForOpportunisticUsageKey` | 44.9 GB |
| purgeable estimate (important − free) | 1.47 GB |
| `diskutil apfs list -plist` container used / overhead | 440 GB / 160 MB not attributed to any volume |

Per-volume usage is only available from `diskutil apfs list -plist` (no root needed). Data 405 GB, System 12.6 GB, Preboot 10.8 GB, **VM 8.59 GB**, Recovery 2.6 GB, Update 6 MB. The sealed system volume `disk3s1` is mounted at `/` as snapshot `disk3s1s1`, so the mount lookup also matches `<volume>s<N>`.

**Big finding: the VM (swap) volume went from 33.3 GB to 8.59 GB within ~1 hour** (free space went from ~29 GiB to 54.7 GB) with no user cleanup. Swap is the most volatile consumer on this machine, and very likely part of the "disk fills up uncontrollably" feeling. Consequences: (1) `status` must show the VM volume prominently, with an explanation; (2) the daemon's 5-minute volume samples must store per-volume consumption, so that history can tell swap growth from data growth; (3) Phase 4 should correlate swap growth with memory-hungry processes (Docker VM, Ollama/LM Studio, browsers).

**S2 — TCC without FDA.** The FDA probe correctly reports `Denied` (on `TCC.db`). Scanning `~/Library` read 207 GB in 1.0 M files in 10.1 s and hit **438 unreadable directories**. Most are `Group Containers/group.*` (several third-party apps), plus `Mail`, `Messages`, `Safari`, `Photos`, `Mobile Documents` (iCloud Drive), `Application Support/{AddressBook, CloudDocs, FileProvider, Knowledge, com.apple.TCC}`, `Biome`, `Trial`, `Suggestions`, `Metadata/CoreSpotlight`, several `Caches/com.apple.*`. `readdir` returns `EPERM` (→ `PermissionDenied`), never an empty listing, so "unknown size" detection works as designed.

**S1 — dataless.** Inconclusive without FDA: `~/Library/Mobile Documents` itself is TCC-protected, and `~/Library/CloudStorage` is empty on this machine. `setiopolicy_np(…MATERIALIZE_DATALESS_FILES_OFF)` succeeds (unit test). Re-run S1 after FDA is granted.

**S4 — `getattrlistbulk`:** deferred. The plain walker already does ~1 M real files in ~10 s (S2), well within budget.

## 2026-11-05 — S6: a signed .app owns its permissions (author's machine, macOS 26)

Setup: a minimal app bundle `target/spike/Spacetrail.app` (`CFBundleIdentifier = io.github.tonyalcast.spacetrail`, `LSUIElement`) wrapping the `fda_probe` example. It's signed with a **self-signed "Code Signing" certificate** `spacetrail-dev`, created in Keychain Access (login keychain, 10-year validity, trusted for code signing), using `codesign --force --options runtime --sign spacetrail-dev --identifier io.github.tonyalcast.spacetrail`, and launched with `open -W` (LaunchServices, so the app is its own responsible process).

| Build | Binary hash | Result |
|---|---|---|
| 1 | 87dac4… | FDA `Denied` (expected) |
| 2 | 158f30… | Read `~/Documents`, `~/Desktop`, `~/Downloads` OK; FDA `Denied`. After this run **spacetrail appeared in System Settings → Full Disk Access on its own (toggle off)**; the author switched it on. |
| 3 | 84dfd5… (different code) | **FDA `Granted`**: the grant survives rebuilds |
| CLI from Terminal | — | FDA still `Denied`: the grant belongs only to the app |

Findings:
1. With a stable signing identity, the designated requirement is `identifier "io.github.tonyalcast.spacetrail" and certificate leaf = H"…"`. TCC keys the grant on it, so **rebuilds keep Full Disk Access**. An ad-hoc signature would lose it on every build.
2. **No API can prompt for Full Disk Access.** But once the app *tries* to read an FDA-protected location (e.g. `TCC.db`), macOS registers it in the Full Disk Access list with the toggle off, so the user only has to flip a switch. The app's onboarding should do exactly that: probe → open `x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles` → poll the probe until granted.
3. Folder consents (Documents/Desktop/Downloads) were readable on first launch. Whether macOS showed its "would like to access…" prompts is unconfirmed; re-test on a fresh bundle id if the onboarding needs to rely on them.
4. Consequence for distribution: the desktop app must ship with a stable signature (self-signed for local builds, Developer ID for releases); grants do not carry over between differently-signed builds.
