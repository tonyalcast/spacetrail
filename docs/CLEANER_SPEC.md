# Cleaner specification (schema v1)

A **cleaner** is a declarative TOML file that tells spacetrail how to **detect** a tool, **measure** what it uses, **explain** it, and optionally **clean** it, with a risk class. Cleaners never contain code. Built-in cleaners live in `cleaners/` and are embedded in the binary. User and community cleaners live in `~/.config/spacetrail/cleaners/*.toml`.

Validate any file with:

```
spacetrail cleaners test path/to/my-cleaner.toml
```

That command checks the schema, resolves every path inside a sandbox (`SPACETRAIL_HOME` = temp dir, populated from the cleaner's `[[test.fixture]]`), runs the planner, and prints the plan. It never deletes anything outside the sandbox.

---

## 1. Top-level fields

| Field | Type | Req. | Description |
|---|---|---|---|
| `schema` | int | ✔ | Must be `1` |
| `id` | string | ✔ | `kebab-case`, unique, `[a-z0-9-]{3,64}`. User files override built-ins with the same id only if `override = true` |
| `version` | string | ✔ | SemVer of the rule itself (stored in the journal) |
| `name` | string | ✔ | Human-readable name |
| `description` | string | ✔ | What this data is (1–3 sentences) |
| `risk` | enum | ✔ | `safe` \| `review` \| `manual`, see §2 |
| `risk_reason` | string | ✔ | **Why** this risk class. Required so that reviewers can challenge it. |
| `category` | string | ✔ | Slash-separated, e.g. `developer/xcode`, `system/logs`, `ai/models` |
| `regenerates` | string | ✔ for safe/review | What happens after cleaning and how the data comes back |
| `docs` | string | — | URL to authoritative docs |
| `platforms` | array | — | default `["macos"]` |
| `override` | bool | — | default `false` |
| `authors` | array | — | GitHub handles |

## 2. Risk classes

| Risk | Meaning | Allowed actions | Autoclean | Confirmation |
|---|---|---|---|---|
| `safe` | Fully regenerable automatically, no user data, negligible impact | `delete_contents`, `trash`, `command` | eligible (if enabled in config) | `--apply` (interactive) or `--apply --yes` |
| `review` | Regenerable but costly, or might contain something wanted | `delete_contents`, `trash`, `command` | **never** | interactive only; `--yes` requires `--cleaner <id>` explicitly |
| `manual` | User data or expensive assets | `manual` only in TOML; the app can remove items the user picks one by one, through the tool's own uninstaller (built-in per-cleaner logic) or the Trash, never permanently. If `cleaners.manual_mode = "trash_per_item"`, the TUI also offers per-item Trash (typed confirmation, never permanent) | **never** | n/a, spacetrail only measures and explains |

The validator rejects combinations outside this table (e.g. a `manual` cleaner with a `command` action).

## 3. `[detect]` — when does this cleaner apply?

All listed conditions are **OR**-ed within a key and **AND**-ed across keys. If `[detect]` is absent, the cleaner applies when any `[measure]` path exists.

| Key | Type | Notes |
|---|---|---|
| `any_path_exists` | array of path patterns | |
| `command_exists` | array of strings | looked up on `PATH` (+ `/opt/homebrew/bin`, `/usr/local/bin`); never executed |
| `app_bundle_id` | array of strings | resolved via LaunchServices; never launched |

## 4. `[measure]` — what to size

| Key | Type | Notes |
|---|---|---|
| `paths` | array of path patterns | each match is one plan **item** (sized recursively, hard links deduped, dataless = 0) |
| `exclude` | array of globs | relative to the matched root or absolute |
| `min_item_bytes` | size string | hide items smaller than this (default `"0"`) |
| `group_by` | enum | `item` (default) \| `cleaner` (show one aggregated line) |

## 5. Path patterns

- `~` → home (or `$SPACETRAIL_HOME/home` in sandbox/tests).
- `${VAR}` / `${VAR:-default}` → environment variable with fallback, e.g. `${GRADLE_USER_HOME:-~/.gradle}`. Only variables in an allow-list are expanded (`HOME`, `TMPDIR`, `XDG_*`, `GRADLE_USER_HOME`, `CARGO_HOME`, `RUSTUP_HOME`, `GOPATH`, `GOCACHE`, `GOMODCACHE`, `NUGET_PACKAGES`, `OLLAMA_MODELS`, `HF_HOME`, `npm_config_cache`, `PNPM_HOME`, `BUN_INSTALL`, `CONDA_PREFIX`, `PUB_CACHE`, …). The list lives in code and is reviewed.
- Globs: `*`, `?`, `[abc]`, `**` (recursive, **only allowed in `exclude`** and in `[project]` roots) via `globset`. Matching is case-insensitive (APFS default).
- A pattern must have a non-wildcard prefix of at least 2 components below `~` or `/` (e.g. `~/Library/Caches/*` ✔, `~/*` ✘). That prefix is the cleaner's **declared root**, used for the containment check (ARCHITECTURE §8).
- A glob component with a literal part (e.g. `~/Downloads/*.dmg`) needs only one literal component before it.
- The literal prefix (declared root) is canonicalized once (`/var` → `/private/var`); below it, matching never resolves a symlink.
- Paths of `safe`/`review` cleaners that hit the deny-list are rejected **at validation time**. Measure-only paths of `manual` cleaners may point at protected locations (e.g. simulator runtimes under `/System/Library/AssetsV2`), since they are never deleted.

## 6. `[preconditions]`

| Key | Type | Notes |
|---|---|---|
| `process_not_running` | array of strings | process name (`p_comm`) or bundle id; checked at plan *and* at execution time |
| `command_exists` | array of strings | |
| `min_free_bytes_after` | — | reserved |

If a precondition fails, the item stays in the plan as **blocked** with the reason (e.g. "Xcode is running").

## 7. `[[action]]` — how to clean

Multiple actions run in order. Each has a `type`:

### `delete_contents`
Removes the matched items (the *children* produced by the `paths` patterns). It never removes the declared root itself.

| Key | Type | Default |
|---|---|---|
| `paths` | array of path patterns | `[measure].paths` |
| `older_than` | duration (`"7d"`, `"12h"`, `"0d"`) | none (no age filter) |
| `age_basis` | `mtime` \| `atime` \| `newest_mtime` | `newest_mtime` for directories (newest mtime of anything inside, computed during measurement), `mtime` for files |
| `exclude` | array of globs | `[]` |
| `method` | `trash` \| `permanent` | `trash`. `permanent` is only honored when the user passes `--permanent` *or* (safe + autoclean + `autoclean.permanent = true` in config) |

### `trash`
Like `delete_contents` but always to Trash (`--permanent` is ignored). Use it for anything the user might want back (Downloads, installers).

### `command`
Runs an external tool's own cleanup, which is often more correct than deleting files (it updates the tool's indexes).

| Key | Type | Notes |
|---|---|---|
| `argv` | array of strings | **argv, never a shell string.** `argv[0]` resolved like `command_exists` |
| `timeout` | duration | default `"10m"` |
| `ok_exit_codes` | array of int | default `[0]` |
| `estimate` | `measure` \| `none` | bytes shown in the plan; `measure` = `[measure]` total (an upper bound, labelled "≈") |
| `env` | table | extra env vars (allow-listed keys only) |

Command actions are **non-undoable** and are shown that way in the plan. **Only built-in cleaners may run commands by default.** User/community cleaners with a `command` action load fine, but the action is disabled until `cleaners.allow_commands = ["<id>", …]` is set in config (per cleaner, not global).

### `manual`
No execution. Shows instructions.

| Key | Type | Notes |
|---|---|---|
| `instructions` | string (markdown) | step-by-step |
| `suggested_commands` | array of arrays (argv) | displayed, copyable, **never executed** |

## 8. `[project]` — project-artifact cleaners (kondo/npkill style)

Instead of fixed paths, these find projects under code roots, selectable when needed:

- `config.toml → projects.roots`, default `["~/Documents/dev", "~/Code", "~/Developer", "~/Projects", "~/src", "~/dev"]` (missing roots are ignored);
- per run: `spacetrail clean --roots <PATH>...` (replaces the configured list);
- `projects.scope = "home"` (or `--scope home`) searches the whole home directory: slower, and it always skips `~/Library`, `~/.Trash`, cloud storage and dataless trees;
- the TUI Cleanup screen has a root picker.

| Key | Type | Notes |
|---|---|---|
| `markers` | array of file names | project root detected when any marker exists, e.g. `["package.json"]` |
| `artifacts` | array of relative dir names | removable artifact dirs, e.g. `["node_modules"]` |
| `stale_after` | duration | default `"60d"` |
| `stale_basis` | `git_last_commit` \| `newest_source_mtime` | default: git if `.git` exists (read via `git log -1 --format=%ct`, a built-in read-only command), else newest mtime outside artifacts |
| `max_depth` | int | default 6 below each root |
| `skip_dirs` | array | always includes `.git`, the artifact dirs themselves, and hidden dirs |

The walker does not descend into an artifact dir after it's found, and it doesn't treat nested projects inside artifacts as projects.

## 9. `[[test.fixture]]` — required for contributions

Describes a fake file tree created inside the sandbox home, plus the expected plan. `spacetrail cleaners test` and the integration test suite both use it.

```toml
[[test.fixture]]
name = "basic"
files = [
  { path = "~/Library/Developer/Xcode/DerivedData/App-abc/Build/x.o", size = "5MB", age = "1d" },
  { path = "~/Library/Developer/Xcode/DerivedData/ModuleCache.noindex/m.pcm", size = "1MB" },
]
running_processes = []          # simulated for preconditions
expect.items = 2
expect.bytes_at_least = "6MB"
expect.blocked = 0
```

## 10. Validation rules (summary)

1. Schema + types; unknown keys are **errors** (typos must not silently disable safety).
2. Risk/action compatibility (§2).
3. Path pattern rules + deny-list (§5).
4. Every cleaner has ≥ 1 `[[test.fixture]]` (built-ins: enforced in CI; user files: warning).
5. `command` argv[0] must not be a shell (`sh`, `bash`, `zsh`, `fish`, `env`, `osascript`, `python*`, `perl`, `ruby`, `node`, …). This is a hard error, even for built-ins.

---

## Example 1 — `safe`, file deletion with precondition

```toml
schema = 1
id = "xcode-derived-data"
version = "1.0.0"
name = "Xcode DerivedData"
description = "Build intermediates, module caches and indexes produced by Xcode builds."
risk = "safe"
risk_reason = "Contains only build outputs; Xcode recreates everything on the next build. No user-authored data."
category = "developer/xcode"
regenerates = "Automatically on the next build (that first build and indexing will be slower)."
docs = "https://developer.apple.com/documentation/xcode/build-system"

[detect]
any_path_exists = ["~/Library/Developer/Xcode/DerivedData"]

[measure]
paths = ["~/Library/Developer/Xcode/DerivedData/*"]

[preconditions]
process_not_running = ["Xcode", "com.apple.dt.Xcode"]

[[action]]
type = "delete_contents"
paths = ["~/Library/Developer/Xcode/DerivedData/*"]
older_than = "0d"

[[test.fixture]]
name = "two-projects"
files = [
  { path = "~/Library/Developer/Xcode/DerivedData/App-abc/Build/Intermediates.noindex/a.o", size = "40MB" },
  { path = "~/Library/Developer/Xcode/DerivedData/Lib-def/Index.noindex/DataStore/u", size = "10MB" },
]
expect.items = 2
expect.bytes_at_least = "50MB"

[[test.fixture]]
name = "blocked-when-xcode-running"
files = [{ path = "~/Library/Developer/Xcode/DerivedData/App-abc/x", size = "1MB" }]
running_processes = ["Xcode"]
expect.items = 1
expect.blocked = 1
```

## Example 2 — `review`, tool-native command

```toml
schema = 1
id = "docker-build-cache"
version = "1.0.0"
name = "Docker build cache"
description = "BuildKit layer cache stored inside Docker Desktop's VM disk image."
risk = "review"
risk_reason = "Regenerable, but rebuilding images afterwards can take a long time and re-download base layers."
category = "containers/docker"
regenerates = "On the next `docker build` (slower first build, network downloads)."
docs = "https://docs.docker.com/reference/cli/docker/builder/prune/"

[detect]
command_exists = ["docker"]

[measure]
# Space lives inside Docker.raw; we report the image file's *allocated* size for context
# and rely on `docker system df` (run by the planner as a built-in read-only probe) for the estimate.
paths = ["~/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw"]
group_by = "cleaner"

[preconditions]
command_exists = ["docker"]

[[action]]
type = "command"
argv = ["docker", "builder", "prune", "--force"]
timeout = "15m"
estimate = "none"

[[action]]
type = "manual"
instructions = """
Docker Desktop does not always shrink `Docker.raw` immediately after pruning.
If the file stays large, open Docker Desktop → Settings → Resources → Disk usage limit,
or run *Troubleshoot → Clean / Purge data* (this deletes **all** images, containers and volumes).
"""

[[test.fixture]]
name = "docker-present"
files = [{ path = "~/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw", size = "2GB", sparse = true }]
commands = ["docker"]          # simulated on PATH inside the sandbox (a stub that records argv)
expect.items = 1
expect.commands = [["docker", "builder", "prune", "--force"]]
```

## Example 3 — `manual`, measure & explain only

```toml
schema = 1
id = "ollama-models"
version = "1.0.0"
name = "Ollama models"
description = "Local LLM weights downloaded by Ollama (blobs + manifests)."
risk = "manual"
risk_reason = "Each model is several GB to re-download and may be customised (Modelfiles); only the user knows which ones are still needed."
category = "ai/models"
docs = "https://github.com/ollama/ollama/blob/main/docs/faq.md"

[detect]
any_path_exists = ["${OLLAMA_MODELS:-~/.ollama/models}"]
command_exists = ["ollama"]

[measure]
paths = ["${OLLAMA_MODELS:-~/.ollama/models}/blobs"]
group_by = "cleaner"

[[action]]
type = "manual"
instructions = """
List installed models and their sizes, then remove the ones you no longer use.
Removing a model through Ollama also cleans up unreferenced blobs.
"""
suggested_commands = [
  ["ollama", "list"],
  ["ollama", "rm", "<model:tag>"],
]

[[test.fixture]]
name = "some-models"
files = [
  { path = "~/.ollama/models/blobs/sha256-aaa", size = "4GB", sparse = true },
  { path = "~/.ollama/models/manifests/registry.ollama.ai/library/llama3/latest", size = "1KB" },
]
expect.items = 1
expect.bytes_at_least = "4GB"
```

## Example 4 (bonus) — project artifacts

```toml
schema = 1
id = "project-node-modules"
version = "1.0.0"
name = "Stale node_modules"
description = "Installed npm/yarn/pnpm/bun dependencies in projects you have not touched recently."
risk = "review"
risk_reason = "Reinstallable from the lockfile, but needs network and time; local patches (patch-package) live elsewhere."
category = "developer/js"
regenerates = "`npm install` / `pnpm install` / `yarn` / `bun install` in the project."

[project]
markers = ["package.json"]
artifacts = ["node_modules"]
stale_after = "60d"

[[action]]
type = "trash"

[[test.fixture]]
name = "one-stale-one-fresh"
files = [
  { path = "~/Code/old/package.json", size = "1KB", age = "200d" },
  { path = "~/Code/old/node_modules/x/index.js", size = "30MB", age = "200d" },
  { path = "~/Code/new/package.json", size = "1KB", age = "1d" },
  { path = "~/Code/new/node_modules/y/index.js", size = "30MB", age = "1d" },
]
expect.items = 1
```

> Fixture `sparse = true` creates the file with `ftruncate` so a "4 GB" fixture costs no disk; the test harness then checks *apparent* size for such files, since allocated size will be ~0.
