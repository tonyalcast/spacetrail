//! The single executor: the only code in spacetrail that removes anything
//! (CLAUDE.md, rule 2).
//!
//! Every removal goes to the Trash, is checked right before it happens
//! (protected locations, symlinks, mount points, cloud-only files, app
//! bundles), and is written to a journal so it can be undone.

use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use spacetrail_macos::dataless;

/// Where removed items go. The real one is the Finder Trash; tests use a folder.
pub trait TrashBackend {
    /// Moves `path` (never following a symlink) and returns its new location.
    ///
    /// # Errors
    /// Any I/O or Foundation failure.
    fn trash(&self, path: &Path) -> io::Result<PathBuf>;
}

/// The macOS Trash.
#[derive(Debug, Clone, Copy, Default)]
pub struct FinderTrash;

impl TrashBackend for FinderTrash {
    fn trash(&self, path: &Path) -> io::Result<PathBuf> {
        spacetrail_macos::trash::move_to_trash(path)
    }
}

/// Why spacetrail refuses to remove something.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// Not an absolute, normalized path.
    #[error("not a valid absolute path")]
    InvalidPath,
    /// Nothing there.
    #[error("it no longer exists")]
    Missing,
    /// A location spacetrail never removes.
    #[error("{0} is protected: spacetrail never removes it")]
    Protected(String),
    /// Inside macOS-managed areas.
    #[error(
        "it belongs to macOS or is shared by all users ({0}); remove it with the app that installed it"
    )]
    SystemArea(String),
    /// Inside an app, framework or bundle.
    #[error(
        "it is part of {0}; removing pieces of an app breaks it (move the whole app to the Trash instead)"
    )]
    InsideBundle(String),
    /// A different volume mounted here.
    #[error("it is a mounted volume")]
    MountPoint,
    /// Stored in the cloud, not on this Mac.
    #[error("it is a cloud-only file and takes no space on this Mac")]
    CloudOnly,
    /// Already in the Trash.
    #[error("it is already in the Trash")]
    InTrash,
    /// Changed since the user confirmed.
    #[error("it changed since you confirmed; look at it again")]
    Changed,
}

/// A removal recorded in the journal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrashRecord {
    /// Unique id (time + inode).
    pub id: String,
    /// Unix seconds.
    pub at: i64,
    /// Where it was.
    pub original: PathBuf,
    /// Where it is now.
    pub trashed_to: PathBuf,
    /// Allocated bytes at removal time (as measured by the caller).
    pub bytes: u64,
    /// "file", "folder" or "symlink".
    pub kind: String,
    /// Who asked (e.g. "app: oldest files").
    pub source: String,
    /// Set when the item was put back.
    #[serde(default)]
    pub restored: bool,
    /// "trash" (undoable), "permanent" or "command".
    #[serde(default = "trash_method")]
    pub method: String,
}

fn trash_method() -> String {
    "trash".to_owned()
}

/// A checked path ready to be removed.
#[derive(Debug, Clone)]
pub struct Checked {
    /// Canonical location (parent resolved, leaf not followed).
    pub path: PathBuf,
    /// "file", "folder" or "symlink".
    pub kind: &'static str,
    dev: u64,
    ino: u64,
}

fn lower(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

fn under(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

/// Runs every safety check on `path` (with `home` as the user's home).
///
/// # Errors
/// The reason spacetrail refuses.
pub fn check(path: &Path, home: &Path) -> Result<Checked, Refusal> {
    check_in(path, home, None)
}

/// [`check`], optionally treating `sandbox` (tests only) as a normal user
/// area even though temp dirs live under `/private` on macOS.
fn check_in(path: &Path, home: &Path, sandbox: Option<&Path>) -> Result<Checked, Refusal> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(Refusal::InvalidPath);
    }
    let leaf = path.file_name().ok_or(Refusal::InvalidPath)?;
    let parent = path.parent().ok_or(Refusal::InvalidPath)?;
    let real_parent = parent.canonicalize().map_err(|_| Refusal::Missing)?;
    let real = real_parent.join(leaf);
    let meta = fs::symlink_metadata(&real).map_err(|_| Refusal::Missing)?;
    let home = home.canonicalize().unwrap_or_else(|_| home.to_owned());
    let (p, h) = (lower(&real), lower(&home));

    // Exact locations that are never removed.
    let mut exact: Vec<String> = [
        "/",
        "/users",
        "/applications",
        "/library",
        "/system",
        "/volumes",
        "/private",
        "/opt",
        "/usr",
        "/usr/local",
        "/private/var",
        "/private/tmp",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    exact.push(h.clone());
    for d in [
        "Library",
        "Documents",
        "Desktop",
        "Downloads",
        "Pictures",
        "Movies",
        "Music",
        "Public",
        "Applications",
        "Developer",
        ".Trash",
        ".ssh",
        ".config",
        ".local",
    ] {
        exact.push(lower(&home.join(d)));
    }
    if exact.contains(&p) {
        return Err(Refusal::Protected(real.display().to_string()));
    }
    // Areas never touched.
    let in_sandbox = sandbox.is_some_and(|sb| {
        under(
            &p,
            &lower(&sb.canonicalize().unwrap_or_else(|_| sb.to_owned())),
        )
    });
    for area in [
        "/system", "/bin", "/sbin", "/cores", "/library", "/usr", "/private",
    ] {
        if under(&p, area) && !under(&p, "/usr/local") && !in_sandbox {
            return Err(Refusal::SystemArea(area.to_owned()));
        }
    }
    for cloud in [
        "Library/CloudStorage",
        "Library/Mobile Documents",
        "Library/Keychains",
    ] {
        if under(&p, &lower(&home.join(cloud))) {
            return Err(Refusal::Protected(cloud.to_owned()));
        }
    }
    if under(&p, &lower(&home.join(".Trash"))) {
        return Err(Refusal::InTrash);
    }
    if let Some(bundle) = real.ancestors().skip(1).find(|a| {
        a.extension().is_some_and(|e| {
            ["app", "framework", "bundle", "kext", "plugin", "appex"]
                .contains(&e.to_string_lossy().to_lowercase().as_str())
        })
    }) {
        return Err(Refusal::InsideBundle(
            bundle
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ));
    }
    if dataless::is_dataless(&meta) {
        return Err(Refusal::CloudOnly);
    }
    let parent_dev = fs::symlink_metadata(&real_parent)
        .map_err(|_| Refusal::Missing)?
        .dev();
    if meta.is_dir() && meta.dev() != parent_dev {
        return Err(Refusal::MountPoint);
    }
    let kind = if meta.file_type().is_symlink() {
        "symlink"
    } else if meta.is_dir() {
        "folder"
    } else {
        "file"
    };
    Ok(Checked {
        path: real,
        kind,
        dev: meta.dev(),
        ino: meta.ino(),
    })
}

/// Append-only JSON-lines journal of removals.
#[derive(Debug, Clone)]
pub struct Journal {
    path: PathBuf,
}

impl Journal {
    /// A journal stored at `path` (created on first write).
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// All records, oldest first (later lines override earlier ones by id).
    #[must_use]
    pub fn records(&self) -> Vec<TrashRecord> {
        let Ok(f) = fs::File::open(&self.path) else {
            return Vec::new();
        };
        let mut out: Vec<TrashRecord> = Vec::new();
        for line in io::BufReader::new(f).lines().map_while(Result::ok) {
            if let Ok(r) = serde_json::from_str::<TrashRecord>(&line) {
                match out.iter_mut().find(|x| x.id == r.id) {
                    Some(x) => *x = r,
                    None => out.push(r),
                }
            }
        }
        out
    }

    fn append(&self, r: &TrashRecord) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(r).map_err(io::Error::other)?)
    }
}

/// Errors of [`trash`] and [`restore`].
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    /// Refused by a safety check.
    #[error("spacetrail won't remove this: {0}")]
    Refused(#[from] Refusal),
    /// The move itself failed.
    #[error("could not move it: {0}")]
    Io(#[from] io::Error),
    /// Unknown journal id.
    #[error("no such removal in the journal")]
    UnknownId,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Checks `path`, re-checks it immediately before acting (TOCTOU), moves it
/// to the Trash and records it.
///
/// # Errors
/// A refusal or the move's failure.
pub fn trash(
    path: &Path,
    home: &Path,
    backend: &dyn TrashBackend,
    journal: &Journal,
    bytes: u64,
    source: &str,
) -> Result<TrashRecord, ExecError> {
    trash_in(path, home, None, backend, journal, bytes, source)
}

fn trash_in(
    path: &Path,
    home: &Path,
    sandbox: Option<&Path>,
    backend: &dyn TrashBackend,
    journal: &Journal,
    bytes: u64,
    source: &str,
) -> Result<TrashRecord, ExecError> {
    let first = check_in(path, home, sandbox)?;
    // Re-validate right before the move: same inode, still passes every check.
    let again = check_in(path, home, sandbox)?;
    if (again.dev, again.ino) != (first.dev, first.ino) || again.path != first.path {
        return Err(Refusal::Changed.into());
    }
    let trashed_to = backend.trash(&again.path)?;
    let record = TrashRecord {
        id: format!("{}-{}", now(), again.ino),
        at: now(),
        original: again.path,
        trashed_to,
        bytes,
        kind: again.kind.to_owned(),
        source: source.to_owned(),
        restored: false,
        method: trash_method(),
    };
    journal.append(&record)?;
    Ok(record)
}

/// Puts a trashed item back where it was, if that place is still free.
///
/// # Errors
/// Unknown id, the original location is taken, or the item left the Trash.
pub fn restore(id: &str, journal: &Journal) -> Result<TrashRecord, ExecError> {
    let mut r = journal
        .records()
        .into_iter()
        .find(|r| r.id == id && !r.restored && r.method == "trash")
        .ok_or(ExecError::UnknownId)?;
    if fs::symlink_metadata(&r.original).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "something already exists at the original location",
        )
        .into());
    }
    if fs::symlink_metadata(&r.trashed_to).is_err() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "it is no longer in the Trash").into());
    }
    fs::rename(&r.trashed_to, &r.original)?;
    r.restored = true;
    journal.append(&r)?;
    Ok(r)
}

/// Deletes `path` permanently after the same checks as [`trash`]. Not undoable.
///
/// # Errors
/// A refusal or the deletion's failure.
pub fn remove_permanently(
    path: &Path,
    home: &Path,
    journal: &Journal,
    bytes: u64,
    source: &str,
) -> Result<TrashRecord, ExecError> {
    remove_permanently_in(path, home, None, journal, bytes, source)
}

fn remove_permanently_in(
    path: &Path,
    home: &Path,
    sandbox: Option<&Path>,
    journal: &Journal,
    bytes: u64,
    source: &str,
) -> Result<TrashRecord, ExecError> {
    let first = check_in(path, home, sandbox)?;
    let again = check_in(path, home, sandbox)?;
    if (again.dev, again.ino) != (first.dev, first.ino) || again.path != first.path {
        return Err(Refusal::Changed.into());
    }
    // std's remove_dir_all never follows symlinks (it works relative to open
    // directory handles), and a symlink leaf is removed as a link.
    #[expect(
        clippy::disallowed_methods,
        reason = "the single executor, after every safety check"
    )]
    if again.kind == "folder" {
        fs::remove_dir_all(&again.path)?;
    } else {
        fs::remove_file(&again.path)?;
    }
    let record = TrashRecord {
        id: format!("{}-{}", now(), again.ino),
        at: now(),
        original: again.path,
        trashed_to: PathBuf::new(),
        bytes,
        kind: again.kind.to_owned(),
        source: source.to_owned(),
        restored: false,
        method: "permanent".to_owned(),
    };
    journal.append(&record)?;
    Ok(record)
}

/// Programs never run, whatever a rule says (they would run arbitrary code).
const FORBIDDEN_PROGRAMS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "csh",
    "tcsh",
    "env",
    "osascript",
    "perl",
    "ruby",
    "node",
    "deno",
    "eval",
    "xargs",
    "sudo",
    "rm",
    "python",
    "python3",
];

/// Result of running a tool's cleanup command.
#[derive(Debug, Clone, Serialize)]
pub struct CommandOutcome {
    /// Exit success.
    pub ok: bool,
    /// Exit code, if any.
    pub code: Option<i32>,
    /// Last lines of stdout/stderr.
    pub output: String,
}

/// Runs `argv` (absolute program path, no shell) with a minimal environment
/// and a time limit. Only the executor may spawn processes.
///
/// # Errors
/// Invalid program, spawn failure or timeout.
pub fn run_command(
    argv: &[String],
    timeout: std::time::Duration,
) -> Result<CommandOutcome, ExecError> {
    run_command_with(argv, timeout, &[])
}

/// [`run_command`] with extra directories on `PATH` (so tools written as
/// `#!/usr/bin/env node` scripts find the interpreter they need).
///
/// # Errors
/// Invalid program, spawn failure or timeout.
pub fn run_command_with(
    argv: &[String],
    timeout: std::time::Duration,
    search: &[PathBuf],
) -> Result<CommandOutcome, ExecError> {
    use std::process::{Command, Stdio};
    let prog = argv.first().map(Path::new).ok_or(Refusal::InvalidPath)?;
    let base = prog
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if !prog.is_absolute()
        || !prog.is_file()
        || FORBIDDEN_PROGRAMS.contains(&base.as_str())
        || base.starts_with("python")
    {
        return Err(Refusal::Protected(format!("program {}", prog.display())).into());
    }
    let mut path = String::new();
    if let Some(dir) = prog.parent() {
        path.push_str(&dir.display().to_string());
        path.push(':');
    }
    for d in search.iter().filter(|d| d.is_dir()) {
        path.push_str(&d.display().to_string());
        path.push(':');
    }
    path.push_str("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin");
    let out_file = tempfile_path("out");
    let stdout = fs::File::create(&out_file)?;
    let stderr = stdout.try_clone()?;
    #[expect(
        clippy::disallowed_methods,
        reason = "the single executor runs tool cleanups"
    )]
    let mut cmd = Command::new(prog);
    // A neutral working directory: apps start in "/" (read-only), and some
    // tools change behavior based on project files in the current folder
    // (e.g. corepack refuses `yarn` under a package.json that asks for pnpm).
    let tmp = std::env::temp_dir();
    let workdir = tmp.join(format!("spacetrail-run-{}-{}", std::process::id(), now()));
    fs::create_dir_all(&workdir)?;
    cmd.args(&argv[1..])
        .current_dir(&workdir)
        .env_clear()
        .env("PATH", path)
        .env("TMPDIR", &tmp)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    if let Some(h) = std::env::var_os("HOME") {
        cmd.env("HOME", h);
    }
    let mut child = cmd.spawn()?;
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the command took too long and was stopped",
            )
            .into());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let text = fs::read_to_string(&out_file).unwrap_or_default();
    // Clean up our own scratch files (never user data).
    #[expect(
        clippy::disallowed_methods,
        reason = "removes the executor's own temp files"
    )]
    {
        let _ = fs::remove_file(&out_file);
        let _ = fs::remove_dir(&workdir);
    }
    let tail: Vec<&str> = text.lines().rev().take(12).collect();
    Ok(CommandOutcome {
        ok: status.success(),
        code: status.code(),
        output: tail.into_iter().rev().collect::<Vec<_>>().join("\n"),
    })
}

fn tempfile_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("spacetrail-{tag}-{}-{}", std::process::id(), now()))
}

/// Options for running suggestions.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Delete permanently instead of moving to the Trash.
    pub permanent: bool,
    /// Who asked (journal).
    pub source: String,
}

/// What happened to one suggestion.
#[derive(Debug, Clone, Serialize)]
pub struct StepResult {
    /// Suggestion id.
    pub id: String,
    /// Suggestion name.
    pub name: String,
    /// "trash", "permanent", "command", "uninstall", "worktree" or "skipped".
    pub method: String,
    /// At least one action succeeded and none failed.
    pub ok: bool,
    /// Items or commands that succeeded.
    pub done: usize,
    /// Items or commands that failed or were skipped.
    pub failed: usize,
    /// Bytes freed (measured where possible) or moved to the Trash.
    pub bytes: u64,
    /// Human messages (errors, skipped reasons, command output).
    pub messages: Vec<String>,
    /// Journal ids that can be undone.
    pub undo_ids: Vec<String>,
    /// Items to send again when retrying.
    pub retry_items: Vec<PathBuf>,
}

fn measured(paths: &[&Path]) -> u64 {
    paths
        .iter()
        .filter_map(|p| crate::suggest::measure(p))
        .map(|m| m.bytes)
        .sum()
}

/// Carries out one suggestion as decided by the planner.
///
/// `only` restricts it to these items (must belong to the suggestion). For
/// `manual` ("your call") suggestions it is required: the user picks each item
/// explicitly, and they are never deleted permanently. Apps that must be
/// closed are checked again here.
pub fn run_suggestion(
    s: &crate::suggest::Suggestion,
    ctx: &crate::suggest::Context,
    backend: &dyn TrashBackend,
    journal: &Journal,
    opts: &RunOptions,
    only: Option<&[PathBuf]>,
) -> StepResult {
    use crate::schema::Risk;
    use crate::suggest::Exec;
    let mut r = StepResult {
        id: s.id.clone(),
        name: s.name.clone(),
        method: "skipped".into(),
        ok: false,
        done: 0,
        failed: 0,
        bytes: 0,
        messages: Vec::new(),
        undo_ids: Vec::new(),
        retry_items: Vec::new(),
    };
    let manual = s.risk == Risk::Manual;
    let items: Vec<&crate::suggest::Item> = s
        .items
        .iter()
        .filter(|i| only.is_none_or(|o| o.contains(&i.path)))
        .collect();
    if manual && only.is_none_or(<[PathBuf]>::is_empty) {
        r.messages.push(
            "Pick the items you want to remove: “your call” items are never removed in bulk."
                .into(),
        );
        return r;
    }
    let open = ctx.running(&s.apps_to_close);
    if !open.is_empty() {
        r.failed = items.len().max(1);
        r.retry_items = items.iter().map(|i| i.path.clone()).collect();
        r.messages.push(format!(
            "Skipped: close {} first, then retry.",
            open.join(", ")
        ));
        return r;
    }
    let permanent = opts.permanent && !manual;
    let source = format!("{} · {}", opts.source, s.id);
    match &s.exec {
        Exec::None => r
            .messages
            .push("spacetrail can't do this one for you; see the instructions.".into()),
        Exec::RemoveItems => {
            r.method = if permanent { "permanent" } else { "trash" }.into();
            for item in items {
                if let Some(why) = &item.protected {
                    r.messages
                        .push(format!("{}: kept ({why}).", item.path.display()));
                    continue;
                }
                if let Some(argv) = &item.argv {
                    // The tool's own uninstaller (ollama rm, rustup toolchain uninstall…).
                    r.method = "uninstall".into();
                    let label = item
                        .note
                        .clone()
                        .unwrap_or_else(|| item.path.display().to_string());
                    match run_command_with(
                        argv,
                        std::time::Duration::from_secs(30 * 60),
                        &ctx.path_dirs,
                    ) {
                        Ok(out) if out.ok => {
                            r.done += 1;
                            r.bytes += item.bytes;
                            let _ = journal.append(&TrashRecord {
                                id: format!("{}-u{}", now(), r.done),
                                at: now(),
                                original: item.path.clone(),
                                trashed_to: PathBuf::new(),
                                bytes: item.bytes,
                                kind: "uninstall".into(),
                                source: source.clone(),
                                restored: false,
                                method: "command".into(),
                            });
                        }
                        Ok(out) => {
                            r.failed += 1;
                            r.retry_items.push(item.path.clone());
                            r.messages.push(format!(
                                "{label}: {}",
                                out.output.lines().last().unwrap_or("failed")
                            ));
                        }
                        Err(e) => {
                            r.failed += 1;
                            r.retry_items.push(item.path.clone());
                            r.messages.push(format!("{label}: {e}"));
                        }
                    }
                    continue;
                }
                let res = if permanent {
                    remove_permanently(&item.path, &ctx.home, journal, item.bytes, &source)
                } else {
                    trash(&item.path, &ctx.home, backend, journal, item.bytes, &source)
                };
                match res {
                    Ok(rec) => {
                        r.done += 1;
                        r.bytes += item.bytes;
                        if rec.method == "trash" {
                            r.undo_ids.push(rec.id);
                        }
                        for c in &item.companions {
                            let res = if permanent {
                                remove_permanently(c, &ctx.home, journal, 0, &source)
                            } else {
                                trash(c, &ctx.home, backend, journal, 0, &source)
                            };
                            match res {
                                Ok(rec) if rec.method == "trash" => r.undo_ids.push(rec.id),
                                Ok(_) => {}
                                Err(e) => r.messages.push(format!("{}: {e}", c.display())),
                            }
                        }
                    }
                    Err(ExecError::Refused(Refusal::Missing)) => {
                        r.done += 1;
                        r.messages
                            .push(format!("{}: already gone.", item.path.display()));
                    }
                    Err(e) => {
                        r.failed += 1;
                        r.retry_items.push(item.path.clone());
                        r.messages.push(format!("{}: {e}", item.path.display()));
                    }
                }
            }
        }
        Exec::Command { argv } => {
            r.method = "command".into();
            let paths: Vec<&Path> = s.items.iter().map(|i| i.path.as_path()).collect();
            let before = measured(&paths);
            match run_command_with(
                argv,
                std::time::Duration::from_secs(15 * 60),
                &ctx.path_dirs,
            ) {
                Ok(out) if out.ok => {
                    r.done = 1;
                    // Measure what really left the disk instead of trusting the estimate.
                    r.bytes = before.saturating_sub(measured(&paths));
                    if r.bytes == 0 {
                        r.messages.push("The tool finished but nothing measurable was freed (it had nothing to remove, or it frees space gradually, like Docker).".into());
                    }
                    if !out.output.trim().is_empty() {
                        r.messages.push(out.output);
                    }
                }
                Ok(out) => {
                    r.failed = 1;
                    r.messages.push(format!(
                        "The command failed (exit {}): {}",
                        out.code.map_or("?".into(), |c| c.to_string()),
                        out.output
                    ));
                }
                Err(e) => {
                    r.failed = 1;
                    r.messages.push(e.to_string());
                }
            }
            let _ = journal.append(&TrashRecord {
                id: format!("{}-cmd", now()),
                at: now(),
                original: PathBuf::from(argv.join(" ")),
                trashed_to: PathBuf::new(),
                bytes: r.bytes,
                kind: "command".into(),
                source: source.clone(),
                restored: false,
                method: "command".into(),
            });
        }
        Exec::RemoveWorktrees => {
            r.method = "worktree".into();
            let git = ctx
                .which("git")
                .unwrap_or_else(|| PathBuf::from("/usr/bin/git"));
            for item in items {
                if let Err(e) = check(&item.path, &ctx.home) {
                    r.failed += 1;
                    r.messages.push(format!("{}: {e}", item.path.display()));
                    continue;
                }
                let p = item.path.display().to_string();
                let argv = vec![
                    git.display().to_string(),
                    "-C".into(),
                    p.clone(),
                    "worktree".into(),
                    "remove".into(),
                    p.clone(),
                ];
                match run_command_with(&argv, std::time::Duration::from_secs(120), &ctx.path_dirs) {
                    Ok(out) if out.ok => {
                        r.done += 1;
                        r.bytes += item.bytes;
                        let _ = journal.append(&TrashRecord {
                            id: format!("{}-wt-{}", now(), r.done),
                            at: now(),
                            original: item.path.clone(),
                            trashed_to: PathBuf::new(),
                            bytes: item.bytes,
                            kind: "folder".into(),
                            source: source.clone(),
                            restored: false,
                            method: "command".into(),
                        });
                    }
                    Ok(out) => {
                        r.failed += 1;
                        r.retry_items.push(item.path.clone());
                        r.messages.push(format!(
                            "{p}: git refused: {}",
                            out.output.lines().last().unwrap_or("")
                        ));
                    }
                    Err(e) => {
                        r.failed += 1;
                        r.retry_items.push(item.path.clone());
                        r.messages.push(format!("{p}: {e}"));
                    }
                }
            }
        }
    }
    r.ok = r.done > 0 && r.failed == 0;
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// Moves items into a folder instead of the real Trash.
    struct FolderTrash(PathBuf);
    impl TrashBackend for FolderTrash {
        fn trash(&self, path: &Path) -> io::Result<PathBuf> {
            fs::create_dir_all(&self.0)?;
            let dest = self.0.join(format!(
                "{}-{}",
                now(),
                path.file_name().unwrap().to_string_lossy()
            ));
            fs::rename(path, &dest)?;
            Ok(dest)
        }
    }

    struct Sandbox {
        tmp: tempfile::TempDir,
        home: PathBuf,
        trash: FolderTrash,
        journal: Journal,
    }

    impl Sandbox {
        fn check(&self, p: &Path) -> Result<Checked, Refusal> {
            check_in(p, &self.home, Some(self.tmp.path()))
        }
        fn trash(&self, p: &Path) -> Result<TrashRecord, ExecError> {
            trash_in(
                p,
                &self.home,
                Some(self.tmp.path()),
                &self.trash,
                &self.journal,
                0,
                "test",
            )
        }
    }

    fn sandbox() -> Sandbox {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join("Library/Caches")).unwrap();
        let trash = FolderTrash(tmp.path().join("trash"));
        let journal = Journal::new(tmp.path().join("journal.jsonl"));
        Sandbox {
            tmp,
            home,
            trash,
            journal,
        }
    }

    #[test]
    fn trash_and_undo_round_trip() {
        let s = sandbox();
        let f = s.home.join("Downloads-old.dmg");
        fs::write(&f, b"installer").unwrap();
        let r = s.trash(&f).unwrap();
        assert!(!f.exists());
        assert!(r.trashed_to.exists());
        assert_eq!(s.journal.records().len(), 1);
        let back = restore(&r.id, &s.journal).unwrap();
        assert!(back.restored);
        assert_eq!(fs::read(&f).unwrap(), b"installer");
        assert!(matches!(
            restore(&r.id, &s.journal),
            Err(ExecError::UnknownId)
        ));
    }

    #[test]
    fn protected_locations_are_refused() {
        let s = sandbox();
        for p in [s.home.clone(), s.home.join("Library")] {
            assert!(matches!(s.check(&p), Err(Refusal::Protected(_))), "{p:?}");
        }
        assert!(matches!(
            check(Path::new("/System/Library"), &s.home),
            Err(Refusal::SystemArea(_)) | Err(Refusal::Protected(_))
        ));
        assert!(matches!(
            check(Path::new("/usr/bin/true"), &s.home),
            Err(Refusal::SystemArea(_))
        ));
        assert!(check(Path::new("/"), &s.home).is_err());
        // Without the test sandbox, the temp dir itself (under /private) is refused.
        assert!(matches!(
            check(&s.home.join("Library/Caches"), &s.home),
            Err(Refusal::SystemArea(_))
        ));
        assert!(matches!(
            check(Path::new("relative/x"), &s.home),
            Err(Refusal::InvalidPath)
        ));
        assert!(matches!(
            check(&s.home.join("Library/../Library"), &s.home),
            Err(Refusal::InvalidPath)
        ));
    }

    #[test]
    fn files_inside_apps_are_refused() {
        let s = sandbox();
        let inner = s.home.join("Applications2/Tool.app/Contents/MacOS/tool");
        fs::create_dir_all(inner.parent().unwrap()).unwrap();
        fs::write(&inner, b"x").unwrap();
        assert!(matches!(s.check(&inner), Err(Refusal::InsideBundle(_))));
        assert!(
            s.check(&s.home.join("Applications2/Tool.app")).is_ok(),
            "the whole app is fine"
        );
    }

    #[test]
    fn symlinks_are_trashed_not_their_target() {
        let s = sandbox();
        let target = s.home.join("Library/Caches/real");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("keep.txt"), b"keep").unwrap();
        let link = s.home.join("Library/Caches/link");
        symlink(&target, &link).unwrap();
        let r = s.trash(&link).unwrap();
        assert_eq!(r.kind, "symlink");
        assert!(target.join("keep.txt").exists(), "target untouched");
        assert!(fs::symlink_metadata(&link).is_err());
    }

    #[test]
    fn symlinked_parent_is_resolved_before_checks() {
        let s = sandbox();
        // ~/shortcut → ~/Library: removing ~/shortcut/Caches must be checked as ~/Library/Caches.
        symlink(s.home.join("Library"), s.home.join("shortcut")).unwrap();
        let c = s.check(&s.home.join("shortcut/Caches")).unwrap();
        assert!(c.path.ends_with("Library/Caches"));
        assert!(matches!(
            s.check(&s.home.join("shortcut")),
            Ok(Checked {
                kind: "symlink",
                ..
            })
        ));
    }

    #[test]
    fn permanent_removal_checks_and_journals() {
        let s = sandbox();
        let d = s.home.join("Library/Caches/old");
        fs::create_dir_all(d.join("x")).unwrap();
        fs::write(d.join("x/f"), b"1").unwrap();
        let r =
            remove_permanently_in(&d, &s.home, Some(s.tmp.path()), &s.journal, 1, "test").unwrap();
        assert_eq!(r.method, "permanent");
        assert!(!d.exists());
        assert!(
            matches!(restore(&r.id, &s.journal), Err(ExecError::UnknownId)),
            "permanent is not undoable"
        );
        assert!(
            remove_permanently_in(
                &s.home.join("Library"),
                &s.home,
                Some(s.tmp.path()),
                &s.journal,
                0,
                "t"
            )
            .is_err()
        );
    }

    #[test]
    fn manual_items_need_explicit_picks_and_never_go_permanent() {
        use crate::schema::Risk;
        use crate::suggest::{Context, Exec, How, Item, Source, Suggestion};
        let s = sandbox();
        let a = s.home.join("Library/Caches/model-a");
        fs::create_dir_all(&a).unwrap();
        let sug = Suggestion {
            id: "m".into(),
            name: "m".into(),
            description: String::new(),
            category: String::new(),
            risk: Risk::Manual,
            risk_reason: String::new(),
            regenerates: None,
            docs: None,
            bytes: 0,
            exact: true,
            items: vec![Item {
                path: a.clone(),
                ..Default::default()
            }],
            how: How::default(),
            blocked_by: Vec::new(),
            apps_to_close: Vec::new(),
            notes: Vec::new(),
            source: Source::Builtin,
            exec: Exec::RemoveItems,
        };
        let ctx = Context {
            home: s.home.clone(),
            env: Vec::new(),
            now: 0,
            processes: Vec::new(),
            path_dirs: Vec::new(),
        };
        let opts = RunOptions {
            permanent: true,
            source: "t".into(),
        };
        let r = run_suggestion(&sug, &ctx, &s.trash, &s.journal, &opts, None);
        assert_eq!(r.done, 0, "no bulk removal of manual items");
        assert!(a.exists());
        // With an explicit pick it runs, and the checks refuse because the sandbox
        // lives under /private: what matters is that nothing was deleted permanently.
        let r = run_suggestion(
            &sug,
            &ctx,
            &s.trash,
            &s.journal,
            &opts,
            Some(std::slice::from_ref(&a)),
        );
        assert_eq!(r.method, "trash", "manual never goes permanent");
    }

    #[test]
    fn commands_need_absolute_non_shell_programs() {
        let ok = run_command(
            &["/usr/bin/true".into()],
            std::time::Duration::from_secs(10),
        )
        .unwrap();
        assert!(ok.ok);
        let fail = run_command(
            &["/usr/bin/false".into()],
            std::time::Duration::from_secs(10),
        )
        .unwrap();
        assert!(!fail.ok);
        for bad in [
            vec!["true".to_owned()],
            vec!["/bin/sh".into(), "-c".into(), "echo".into()],
            vec!["/bin/rm".into(), "-rf".into(), "/tmp/x".into()],
        ] {
            assert!(
                run_command(&bad, std::time::Duration::from_secs(5)).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn already_trashed_and_missing_are_refused() {
        let s = sandbox();
        fs::create_dir_all(s.home.join(".Trash/x")).unwrap();
        assert!(matches!(
            s.check(&s.home.join(".Trash/x")),
            Err(Refusal::InTrash)
        ));
        assert!(matches!(
            s.check(&s.home.join("nope")),
            Err(Refusal::Missing)
        ));
    }
}
