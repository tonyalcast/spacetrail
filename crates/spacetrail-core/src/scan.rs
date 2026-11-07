//! Parallel, read-only directory walker.
//!
//! Guarantees:
//! - only `lstat`/`readdir`: never opens files, never follows symlinks;
//! - cloud-only (dataless) files and directories are counted but never read;
//! - hard links are counted once per `(dev, inode)`;
//! - does not cross into other filesystems unless asked;
//! - unreadable directories are reported (unknown size), never counted as 0;
//! - memory is bounded by the number of directories plus the top-N files.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use rayon::prelude::*;

use crate::platform::Platform;
use crate::tree::{BuildNode, DirState, DirTree, Stats};

/// Options controlling a scan.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Stay on the filesystem of the scan root (default `true`).
    pub one_file_system: bool,
    /// How many of the largest files to keep (default 200).
    pub top_files: usize,
    /// Worker threads; `None` uses one per CPU.
    pub threads: Option<usize>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            one_file_system: true,
            top_files: 200,
            threads: None,
        }
    }
}

/// Live counters that another thread can read while a scan runs, plus a
/// cancellation flag.
#[derive(Debug, Default)]
pub struct ScanProgress {
    /// Directories listed so far.
    pub dirs: AtomicU64,
    /// Non-directory entries seen so far.
    pub files: AtomicU64,
    /// Allocated bytes counted so far.
    pub bytes: AtomicU64,
    cancel: AtomicBool,
}

impl ScanProgress {
    /// Asks the running scan to stop; the result is marked as cancelled.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// A large file found during the scan.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TopFile {
    /// Allocated bytes.
    pub alloc: u64,
    /// Full path.
    pub path: PathBuf,
    /// Apparent size in bytes.
    pub apparent: u64,
    /// Last modification time, if available.
    pub modified: Option<SystemTime>,
}

/// A directory whose content could not be listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    /// Full path.
    pub path: PathBuf,
    /// Error kind returned by `readdir` (usually `PermissionDenied`).
    pub kind: ErrorKind,
    /// Raw OS error. On macOS `EPERM` (1) means a privacy (TCC) denial that
    /// Full Disk Access lifts; `EACCES` (13) means ordinary Unix permissions
    /// (owned by root or another user).
    pub errno: Option<i32>,
}

impl Unreadable {
    /// `true` if the denial comes from macOS privacy protection (TCC), which
    /// granting Full Disk Access would lift.
    #[must_use]
    pub fn is_privacy_protected(&self) -> bool {
        self.errno == Some(1)
    }
}

/// Outcome of a scan.
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// The scanned root as given.
    pub root: PathBuf,
    /// Directory aggregates.
    pub tree: DirTree,
    /// Largest files, biggest first.
    pub top_files: Vec<TopFile>,
    /// Directories that could not be listed.
    pub unreadable: Vec<Unreadable>,
    /// Directories skipped because they are on another filesystem.
    pub skipped_mounts: Vec<PathBuf>,
    /// Wall-clock start.
    pub started_at: SystemTime,
    /// Wall-clock duration.
    pub elapsed: Duration,
    /// `true` if [`ScanProgress::cancel`] stopped the scan early.
    pub cancelled: bool,
}

/// Errors that prevent a scan from starting.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// The root cannot be accessed.
    #[error("cannot access {path}: {source}")]
    Root {
        /// Root path.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The root is not a directory.
    #[error("{0} is not a directory")]
    NotADirectory(PathBuf),
    /// The worker pool could not be created.
    #[error("cannot start scanner threads: {0}")]
    ThreadPool(String),
}

const HARDLINK_SHARDS: usize = 64;

struct Shared<'a> {
    platform: &'a dyn Platform,
    opts: &'a ScanOptions,
    progress: &'a ScanProgress,
    root_dev: u64,
    hardlinks: Vec<Mutex<HashSet<(u64, u64)>>>,
    top: Mutex<BinaryHeap<Reverse<TopFile>>>,
    /// Smallest size currently in a full `top` heap; lets most files skip the lock.
    top_floor: AtomicU64,
    unreadable: Mutex<Vec<Unreadable>>,
    skipped_mounts: Mutex<Vec<PathBuf>>,
}

impl Shared<'_> {
    /// Returns `true` the first time an inode is seen.
    fn first_link(&self, meta: &Metadata) -> bool {
        let key = (meta.dev(), meta.ino());
        let shard = (key.1 as usize ^ key.0 as usize) % HARDLINK_SHARDS;
        self.hardlinks[shard]
            .lock()
            .expect("hardlink set poisoned")
            .insert(key)
    }

    fn offer_top_file(&self, path: impl FnOnce() -> PathBuf, meta: &Metadata, alloc: u64) {
        let n = self.opts.top_files;
        if n == 0 || alloc <= self.top_floor.load(Ordering::Relaxed) {
            return;
        }
        let mut heap = self.top.lock().expect("top-files heap poisoned");
        if heap.len() >= n && heap.peek().is_some_and(|Reverse(min)| min.alloc >= alloc) {
            return;
        }
        heap.push(Reverse(TopFile {
            alloc,
            path: path(),
            apparent: meta.len(),
            modified: meta.modified().ok(),
        }));
        if heap.len() > n {
            heap.pop();
        }
        if heap.len() >= n {
            if let Some(Reverse(min)) = heap.peek() {
                self.top_floor.store(min.alloc, Ordering::Relaxed);
            }
        }
    }
}

/// Allocated bytes of an entry.
fn allocated(meta: &Metadata) -> u64 {
    meta.blocks().saturating_mul(512)
}

/// Scans `root` recursively.
///
/// The root itself is resolved with `stat` (a symlinked root is followed, like
/// `du -H`); nothing below it is ever followed.
///
/// # Errors
/// Fails only if the root is inaccessible or not a directory; problems below
/// the root are recorded in the result instead.
pub fn scan(
    root: &Path,
    platform: &dyn Platform,
    opts: &ScanOptions,
    progress: &ScanProgress,
) -> Result<ScanResult, ScanError> {
    let meta = fs::metadata(root).map_err(|source| ScanError::Root {
        path: root.to_owned(),
        source,
    })?;
    if !meta.is_dir() {
        return Err(ScanError::NotADirectory(root.to_owned()));
    }
    let mut builder = rayon::ThreadPoolBuilder::new()
        .thread_name(|i| format!("spacetrail-scan-{i}"))
        // Recursion depth follows directory depth; give deep trees room.
        .stack_size(16 * 1024 * 1024);
    if let Some(t) = opts.threads {
        builder = builder.num_threads(t);
    }
    let pool = builder
        .build()
        .map_err(|e| ScanError::ThreadPool(e.to_string()))?;

    let shared = Shared {
        platform,
        opts,
        progress,
        root_dev: meta.dev(),
        hardlinks: (0..HARDLINK_SHARDS)
            .map(|_| Mutex::new(HashSet::new()))
            .collect(),
        top: Mutex::new(BinaryHeap::new()),
        top_floor: AtomicU64::new(0),
        unreadable: Mutex::new(Vec::new()),
        skipped_mounts: Mutex::new(Vec::new()),
    };

    let started_at = SystemTime::now();
    let clock = Instant::now();
    let root_node = pool.install(|| walk_dir(&shared, root.to_owned(), root.into(), &meta));
    let elapsed = clock.elapsed();

    let mut top_files: Vec<TopFile> = shared
        .top
        .into_inner()
        .expect("top-files heap poisoned")
        .into_iter()
        .map(|Reverse(f)| f)
        .collect();
    top_files.sort_by(|a, b| b.alloc.cmp(&a.alloc).then_with(|| a.path.cmp(&b.path)));
    let mut unreadable = shared.unreadable.into_inner().expect("poisoned");
    unreadable.sort_by(|a, b| a.path.cmp(&b.path));
    let mut skipped_mounts = shared.skipped_mounts.into_inner().expect("poisoned");
    skipped_mounts.sort();

    Ok(ScanResult {
        root: root.to_owned(),
        tree: root_node.into_tree(),
        top_files,
        unreadable,
        skipped_mounts,
        started_at,
        elapsed,
        cancelled: progress.cancelled(),
    })
}

fn walk_dir(shared: &Shared<'_>, path: PathBuf, name: OsString, meta: &Metadata) -> BuildNode {
    let mut node = BuildNode {
        name,
        own: Stats {
            alloc: allocated(meta),
            newest_mtime: meta.mtime(),
            apparent: meta.len(),
            ..Stats::default()
        },
        ..BuildNode::default()
    };
    if shared.progress.cancelled() {
        return node;
    }
    if shared.platform.is_dataless(meta) {
        node.state = DirState::Dataless;
        node.own.dataless = 1;
        return node;
    }
    let entries = match fs::read_dir(&path) {
        Ok(rd) => rd,
        Err(e) => {
            if e.kind() == ErrorKind::PermissionDenied {
                node.state = DirState::Unreadable;
                node.own.unreadable_dirs = 1;
                shared
                    .unreadable
                    .lock()
                    .expect("poisoned")
                    .push(Unreadable {
                        path,
                        kind: e.kind(),
                        errno: e.raw_os_error(),
                    });
            } else {
                node.state = DirState::Failed;
                node.own.errors += 1;
            }
            return node;
        }
    };

    let mut subdirs: Vec<(PathBuf, OsString, Metadata)> = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            node.own.errors += 1;
            continue;
        };
        // On Unix, DirEntry::metadata does not traverse symlinks (fstatat + AT_SYMLINK_NOFOLLOW).
        let Ok(m) = entry.metadata() else {
            node.own.errors += 1;
            continue;
        };
        if m.is_dir() {
            if shared.opts.one_file_system && m.dev() != shared.root_dev {
                shared
                    .skipped_mounts
                    .lock()
                    .expect("poisoned")
                    .push(entry.path());
                continue;
            }
            subdirs.push((entry.path(), entry.file_name(), m));
            continue;
        }
        if shared.platform.is_dataless(&m) {
            node.own.dataless += 1;
            node.own.dataless_bytes += m.len();
            continue;
        }
        if m.nlink() > 1 && !shared.first_link(&m) {
            node.own.hardlink_dupes += 1;
            continue;
        }
        let alloc = allocated(&m);
        node.own.alloc += alloc;
        node.own.apparent += m.len();
        node.own.files += 1;
        node.own.newest_mtime = node.own.newest_mtime.max(m.mtime());
        if m.is_file() {
            shared.offer_top_file(|| entry.path(), &m, alloc);
        }
    }

    let p = shared.progress;
    p.dirs.fetch_add(1, Ordering::Relaxed);
    p.files.fetch_add(node.own.files, Ordering::Relaxed);
    p.bytes.fetch_add(node.own.alloc, Ordering::Relaxed);

    node.children = subdirs
        .into_par_iter()
        .map(|(path, name, m)| walk_dir(shared, path, name, &m))
        .collect();
    node
}
