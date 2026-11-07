//! Read-only suggestions: what could be freed, how, and at what risk.
//!
//! Nothing here deletes anything. The planner detects, measures, filters by
//! age, removes double counting between overlapping rules and reports which
//! apps must be closed first.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use spacetrail_core::{ScanOptions, ScanProgress, parse_duration, parse_size, scan};
use spacetrail_macos::MacPlatform;
use spacetrail_macos::process::{self, ProcessInfo};

use crate::pattern::{self, Excludes};
use crate::schema::{Action, AgeBasis, Cleaner, Risk};

/// Where a suggestion comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A built-in TOML cleaner.
    Builtin,
    /// A user TOML cleaner.
    User,
    /// A built-in dynamic detector (projects, worktrees, Docker…).
    Detector,
}

/// One measured thing inside a suggestion.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Item {
    /// Absolute path.
    pub path: PathBuf,
    /// Allocated bytes that would be freed.
    pub bytes: u64,
    /// Newest modification time inside (Unix seconds), 0 if unknown.
    pub newest_mtime: i64,
    /// Extra context (e.g. "branch feature-x, last touched 40 days ago").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The tool's own way to remove this item (absolute program path), used
    /// instead of the Trash (e.g. `ollama rm llama3:8b`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argv: Option<Vec<String>>,
    /// Files that belong with the item and go with it (e.g. an AVD's `.ini`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub companions: Vec<PathBuf>,
    /// Why this item must not be removed (e.g. the active Node version).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protected: Option<String>,
}

/// How to free the space.
#[derive(Debug, Clone, Default, Serialize)]
pub struct How {
    /// One-line instruction.
    pub summary: String,
    /// Commands to run (argv arrays, shown, never executed here).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<Vec<String>>,
    /// Longer manual steps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// How spacetrail itself can carry out a suggestion (decided here, never by the UI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Exec {
    /// spacetrail does not run it (manual risk, or nothing it can do).
    None,
    /// Move each item to the Trash (or delete it permanently if the user asks).
    RemoveItems,
    /// Run the tool's own cleanup; `argv[0]` is an absolute path.
    Command {
        /// Program and arguments.
        argv: Vec<String>,
    },
    /// `git worktree remove` each item (git refuses when there are uncommitted changes).
    RemoveWorktrees,
}

/// A recommendation.
#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    /// Cleaner or detector id.
    pub id: String,
    /// Human name.
    pub name: String,
    /// What it is.
    pub description: String,
    /// Category, e.g. `developer/js`.
    pub category: String,
    /// Risk class.
    pub risk: Risk,
    /// Why this risk class.
    pub risk_reason: String,
    /// What happens after cleaning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regenerates: Option<String>,
    /// Docs URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    /// Bytes that could be freed (sum of items, or the tool's own estimate).
    pub bytes: u64,
    /// `false` when `bytes` is an upper bound (tool-native cleanups).
    pub exact: bool,
    /// Measured items, largest first.
    pub items: Vec<Item>,
    /// How to free it.
    pub how: How,
    /// Apps that must be closed first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    /// Every app that must be closed while cleaning (re-checked when running).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub apps_to_close: Vec<String>,
    /// Extra hints.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Origin.
    pub source: Source,
    /// How spacetrail can carry it out.
    pub exec: Exec,
}

impl Suggestion {
    /// Recomputes `bytes` from items and sorts them, largest first.
    pub fn finish(&mut self) {
        self.items
            .sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
        self.bytes = self.items.iter().map(|i| i.bytes).sum();
    }
}

/// System facts the planner needs; injectable for tests.
#[derive(Debug)]
pub struct Context {
    /// The user's home (or a sandbox home in tests).
    pub home: PathBuf,
    /// Allowed environment variables.
    pub env: Vec<(String, String)>,
    /// Current time (Unix seconds).
    pub now: i64,
    /// Running processes.
    pub processes: Vec<ProcessInfo>,
    /// Directories searched for commands.
    pub path_dirs: Vec<PathBuf>,
}

impl Context {
    /// Facts from the running system.
    #[must_use]
    pub fn from_system(home: PathBuf) -> Self {
        let mut path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
            path_dirs.push(PathBuf::from(extra));
        }
        path_dirs.push(home.join(".cargo/bin"));
        path_dirs.push(home.join(".bun/bin"));
        path_dirs.push(home.join(".local/bin"));
        // GUI apps get a minimal PATH: add the usual tool locations.
        for extra in [
            "/opt/miniconda3/bin",
            "/opt/anaconda3/bin",
            "/usr/local/go/bin",
            "/usr/local/share/dotnet",
            "/Applications/Docker.app/Contents/Resources/bin",
        ] {
            path_dirs.push(PathBuf::from(extra));
        }
        for rel in [
            "miniconda3/bin",
            "anaconda3/bin",
            "go/bin",
            ".dotnet",
            "Library/pnpm",
        ] {
            path_dirs.push(home.join(rel));
        }
        // nvm: newest installed Node first.
        if let Ok(rd) = std::fs::read_dir(home.join(".nvm/versions/node")) {
            let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path().join("bin")).collect();
            // Numeric version order (v9 < v22), newest first.
            v.sort_by_key(|p| {
                p.parent()
                    .and_then(|d| d.file_name())
                    .map(|n| {
                        n.to_string_lossy()
                            .trim_start_matches('v')
                            .split('.')
                            .map(|x| x.parse::<u32>().unwrap_or(0))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            });
            path_dirs.extend(v.into_iter().rev());
        }
        Self {
            env: pattern::ALLOWED_VARS
                .iter()
                .filter_map(|k| std::env::var(k).ok().map(|v| ((*k).to_owned(), v)))
                .collect(),
            now: now(),
            processes: process::list(),
            path_dirs,
            home: home.canonicalize().unwrap_or(home),
        }
    }

    /// Looks up an allowed environment variable.
    #[must_use]
    pub fn var(&self, k: &str) -> Option<String> {
        self.env
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
    }

    /// `~`/variable expansion followed by globbing.
    #[must_use]
    pub fn resolve(&self, pattern: &str) -> Vec<PathBuf> {
        pattern::expand_vars(pattern, &self.home, &|k| self.var(k))
            .map(|p| pattern::glob(&p))
            .unwrap_or_default()
    }

    /// Expands a pattern without globbing (for excludes).
    #[must_use]
    pub fn expand(&self, pattern: &str) -> Option<String> {
        pattern::expand_vars(pattern, &self.home, &|k| self.var(k))
    }

    /// `true` if an executable named `cmd` is on the search path.
    #[must_use]
    pub fn command_exists(&self, cmd: &str) -> bool {
        self.which(cmd).is_some()
    }

    /// Absolute path of the executable named `cmd`, if found.
    #[must_use]
    pub fn which(&self, cmd: &str) -> Option<PathBuf> {
        if cmd.starts_with('/') {
            return Path::new(cmd).is_file().then(|| PathBuf::from(cmd));
        }
        self.path_dirs
            .iter()
            .map(|d| d.join(cmd))
            .find(|p| p.is_file())
    }

    /// Which of `names` are running.
    #[must_use]
    pub fn running(&self, names: &[String]) -> Vec<String> {
        process::running(names, &self.processes)
            .into_iter()
            .map(str::to_owned)
            .collect()
    }
}

/// Current Unix time in seconds.
#[must_use]
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Size facts for one path.
#[derive(Debug, Clone, Copy, Default)]
pub struct Measured {
    /// Allocated bytes.
    pub bytes: u64,
    /// Newest mtime inside.
    pub newest_mtime: i64,
    /// Own mtime of the path.
    pub mtime: i64,
}

/// Measures a file or directory without following symlinks.
#[must_use]
pub fn measure(path: &Path) -> Option<Measured> {
    use std::os::unix::fs::MetadataExt;
    let md = std::fs::symlink_metadata(path).ok()?;
    if md.file_type().is_symlink() {
        return None;
    }
    if !md.is_dir() {
        return Some(Measured {
            bytes: md.blocks() * 512,
            newest_mtime: md.mtime(),
            mtime: md.mtime(),
        });
    }
    let opts = ScanOptions {
        top_files: 0,
        ..ScanOptions::default()
    };
    let res = scan(path, &MacPlatform, &opts, &ScanProgress::default()).ok()?;
    let t = res.tree.root().total;
    Some(Measured {
        bytes: t.alloc,
        newest_mtime: t.newest_mtime,
        mtime: md.mtime(),
    })
}

/// "3 days ago", "5 months ago".
#[must_use]
pub fn age_text(now: i64, then: i64) -> String {
    if then <= 0 {
        return "unknown age".into();
    }
    let days = (now - then).max(0) / 86_400;
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=59 => format!("{days} days ago"),
        60..=729 => format!("{} months ago", days / 30),
        _ => format!("{} years ago", days / 365),
    }
}

/// Keeps track of measured paths so overlapping rules don't count twice.
#[derive(Debug, Default)]
pub struct Claims {
    claimed: Vec<(PathBuf, u64)>,
}

impl Claims {
    /// Adjusts an item for previous claims: `None` if it lies inside a
    /// claimed path, otherwise its bytes minus claimed paths inside it.
    #[must_use]
    pub fn adjust(&self, path: &Path, bytes: u64) -> Option<u64> {
        if self.claimed.iter().any(|(c, _)| path.starts_with(c)) {
            return None;
        }
        let inside: u64 = self
            .claimed
            .iter()
            .filter(|(c, _)| c.starts_with(path))
            .map(|(_, b)| *b)
            .sum();
        Some(bytes.saturating_sub(inside))
    }

    /// Records a path as accounted for.
    pub fn claim(&mut self, path: PathBuf, bytes: u64) {
        self.claimed.push((path, bytes));
    }

    /// `true` if `path` is inside an already claimed path.
    #[must_use]
    pub fn covers(&self, path: &Path) -> bool {
        self.claimed.iter().any(|(c, _)| path.starts_with(c))
    }
}

/// Literal depth of a cleaner's most specific pattern (more specific rules
/// claim their paths first).
fn specificity(c: &Cleaner) -> usize {
    c.measure
        .paths
        .iter()
        .map(|p| {
            p.split('/')
                .take_while(|s| !s.contains(['*', '?', '[', '{']))
                .count()
        })
        .max()
        .unwrap_or(0)
}

/// Orders cleaners so the most specific claim their paths first.
pub fn order_by_specificity(cleaners: &mut [(Cleaner, Source)]) {
    cleaners.sort_by_key(|(c, _)| std::cmp::Reverse(specificity(c)));
}

/// Evaluates one TOML cleaner. Returns `None` when it doesn't apply or
/// finds nothing worth showing.
#[must_use]
pub fn evaluate(
    c: &Cleaner,
    source: Source,
    ctx: &Context,
    claims: &mut Claims,
    min_bytes: u64,
) -> Option<Suggestion> {
    let d = &c.detect;
    if !d.any_path_exists.is_empty()
        && !d.any_path_exists.iter().any(|p| !ctx.resolve(p).is_empty())
    {
        return None;
    }
    if !d.command_exists.is_empty() && !d.command_exists.iter().any(|x| ctx.command_exists(x)) {
        return None;
    }

    let remove = c.remove_spec();
    let older_than = remove
        .and_then(|(r, _)| r.older_than.as_deref())
        .and_then(|s| parse_duration(s).ok())
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let basis = remove.and_then(|(r, _)| r.age_basis);
    let mut excludes: Vec<String> = c.measure.exclude.clone();
    if let Some((r, _)) = remove {
        excludes.extend(r.exclude.iter().cloned());
    }
    let excludes = Excludes::new(
        &excludes
            .iter()
            .filter_map(|e| ctx.expand(e))
            .collect::<Vec<_>>(),
    );
    let min_item = c
        .measure
        .min_item_bytes
        .as_deref()
        .and_then(|s| parse_size(s).ok())
        .unwrap_or(0);

    let mut seen = HashSet::new();
    let mut items = Vec::new();
    let mut skipped_recent = 0u64;
    for p in &c.measure.paths {
        for path in ctx.resolve(p) {
            if !seen.insert(path.clone()) || excludes.matches(&path) {
                continue;
            }
            let Some(m) = measure(&path) else { continue };
            if let Some(age) = older_than {
                let ts = match basis {
                    Some(AgeBasis::Mtime) => m.mtime,
                    _ => m.newest_mtime,
                };
                if ctx.now - ts < age {
                    skipped_recent += m.bytes;
                    continue;
                }
            }
            let Some(bytes) = claims.adjust(&path, m.bytes) else {
                continue;
            };
            if bytes < min_item.max(1) {
                continue;
            }
            items.push(Item {
                path,
                bytes,
                newest_mtime: m.newest_mtime,
                note: None,
                ..Default::default()
            });
        }
    }
    for i in &items {
        claims.claim(i.path.clone(), i.bytes);
    }

    let mut s = Suggestion {
        id: c.id.clone(),
        name: c.name.clone(),
        description: c.description.trim().to_owned(),
        category: c.category.clone(),
        risk: c.risk,
        risk_reason: c.risk_reason.trim().to_owned(),
        regenerates: c.regenerates.clone(),
        docs: c.docs.clone(),
        bytes: 0,
        // Tool-native cleanups (e.g. `pnpm store prune`) may free less than measured.
        exact: !c
            .actions
            .iter()
            .any(|a| matches!(a, Action::Command { .. })),
        items,
        how: how_for(c, ctx),
        blocked_by: ctx.running(&c.preconditions.process_not_running),
        apps_to_close: c.preconditions.process_not_running.clone(),
        notes: Vec::new(),
        source,
        exec: exec_for(c, ctx),
    };
    if skipped_recent > 0 {
        s.notes.push(format!(
            "{} recently used data is kept by the age filter",
            spacetrail_core::human(skipped_recent)
        ));
    }
    for cmd in &c.preconditions.command_exists {
        if !ctx.command_exists(cmd) {
            s.notes.push(format!(
                "`{cmd}` is not on PATH; delete the folders instead"
            ));
        }
    }
    s.finish();
    if !s.blocked_by.is_empty() {
        s.notes
            .push(format!("close {} first", s.blocked_by.join(", ")));
    }
    (s.bytes >= min_bytes).then_some(s)
}

/// Tool command when the tool is installed, otherwise removing the items.
fn exec_for(c: &Cleaner, ctx: &Context) -> Exec {
    if c.risk == Risk::Manual {
        return Exec::None;
    }
    for a in &c.actions {
        if let Action::Command { argv, .. } = a
            && let Some(prog) = argv.first().and_then(|p| ctx.which(p))
        {
            let mut full = vec![prog.display().to_string()];
            full.extend(argv.iter().skip(1).cloned());
            return Exec::Command { argv: full };
        }
    }
    if c.remove_spec().is_some() {
        Exec::RemoveItems
    } else {
        Exec::None
    }
}

fn how_for(c: &Cleaner, ctx: &Context) -> How {
    let commands: Vec<Vec<String>> = c
        .commands()
        .into_iter()
        .filter(|argv| {
            argv.first().is_some_and(|p| ctx.command_exists(p)) || c.risk == Risk::Manual
        })
        .collect();
    let summary = if let Some((r, to_trash)) = c.remove_spec() {
        let age = r
            .older_than
            .as_deref()
            .filter(|a| *a != "0d" && *a != "0")
            .map(|a| format!(" not used in the last {a}"))
            .unwrap_or_default();
        if !commands.is_empty() {
            format!("Run the command below (or delete the items{age})")
        } else if to_trash {
            format!("Move the items{age} to the Trash")
        } else {
            format!("Delete the items{age}; they are rebuilt when needed")
        }
    } else if !commands.is_empty() && c.risk != Risk::Manual {
        "Run the command below".to_owned()
    } else {
        "Decide item by item (instructions below)".to_owned()
    };
    How {
        summary,
        commands,
        instructions: c.instructions(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(home: &Path) -> Context {
        Context {
            home: home.to_owned(),
            env: Vec::new(),
            now: now(),
            processes: Vec::new(),
            path_dirs: Vec::new(),
        }
    }

    fn cleaner(paths: &str, extra: &str) -> Cleaner {
        Cleaner::from_toml(&format!(
            r#"
schema = 1
id = "t"
version = "1"
name = "T"
description = "d"
risk = "safe"
risk_reason = "r"
category = "c"
regenerates = "g"
[measure]
paths = [{paths}]
[[action]]
type = "delete_contents"
{extra}
"#
        ))
        .unwrap()
    }

    fn write(p: &Path, len: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![1u8; len]).unwrap();
    }

    #[test]
    fn measures_items_in_sandbox_home() {
        let tmp = tempfile::tempdir().unwrap();
        let h = tmp.path();
        write(&h.join("Library/Caches/A/x"), 100_000);
        write(&h.join("Library/Caches/B/y"), 300_000);
        let c = cleaner(r#""~/Library/Caches/*""#, "");
        let s = evaluate(&c, Source::Builtin, &ctx(h), &mut Claims::default(), 1).unwrap();
        assert_eq!(s.exec, Exec::RemoveItems);
        assert_eq!(s.items.len(), 2);
        assert!(s.items[0].path.ends_with("B"), "largest first");
        assert!(s.bytes >= 400_000);
    }

    #[test]
    fn age_filter_keeps_recent_items() {
        let tmp = tempfile::tempdir().unwrap();
        let h = tmp.path();
        write(&h.join("Library/Logs/app/new.log"), 10_000);
        let c = cleaner(r#""~/Library/Logs/*""#, r#"older_than = "7d""#);
        assert!(evaluate(&c, Source::Builtin, &ctx(h), &mut Claims::default(), 1).is_none());
        let mut later = ctx(h);
        later.now += 30 * 86_400;
        let s = evaluate(&c, Source::Builtin, &later, &mut Claims::default(), 1).unwrap();
        assert_eq!(s.items.len(), 1);
    }

    #[test]
    fn overlapping_rules_do_not_double_count() {
        let tmp = tempfile::tempdir().unwrap();
        let h = tmp.path();
        write(&h.join("Library/Caches/Yarn/v6/pkg"), 500_000);
        write(&h.join("Library/Caches/Other/x"), 100_000);
        let mut claims = Claims::default();
        let yarn = cleaner(r#""~/Library/Caches/Yarn""#, "");
        let general = cleaner(r#""~/Library/Caches/*""#, "");
        let a = evaluate(&yarn, Source::Builtin, &ctx(h), &mut claims, 1).unwrap();
        let b = evaluate(&general, Source::Builtin, &ctx(h), &mut claims, 1).unwrap();
        assert_eq!(b.items.len(), 1, "Yarn is already claimed");
        assert!(b.items[0].path.ends_with("Other"));
        assert!(a.bytes >= 500_000);
    }

    #[test]
    fn claims_subtract_nested_items() {
        let mut c = Claims::default();
        c.claim(PathBuf::from("/a/b/c"), 30);
        assert_eq!(c.adjust(Path::new("/a/b"), 100), Some(70));
        assert_eq!(c.adjust(Path::new("/a/b/c/d"), 10), None);
        assert_eq!(c.adjust(Path::new("/z"), 5), Some(5));
    }

    #[test]
    fn symlinked_items_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let h = tmp.path();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("big"), 1_000_000);
        std::fs::create_dir_all(h.join("Library/Caches")).unwrap();
        std::os::unix::fs::symlink(outside.path(), h.join("Library/Caches/link")).unwrap();
        let c = cleaner(r#""~/Library/Caches/*""#, "");
        assert!(evaluate(&c, Source::Builtin, &ctx(h), &mut Claims::default(), 1).is_none());
    }

    #[test]
    fn ages() {
        assert_eq!(age_text(100 * 86_400, 99 * 86_400), "yesterday");
        assert_eq!(age_text(100 * 86_400, 0), "unknown age");
        assert_eq!(age_text(400 * 86_400, 100 * 86_400), "10 months ago");
    }
}
