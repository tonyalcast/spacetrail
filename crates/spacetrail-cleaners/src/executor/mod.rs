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
        .find(|r| r.id == id && !r.restored)
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
