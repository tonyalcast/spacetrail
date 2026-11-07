//! `spacetrail scan` and `spacetrail top`: measure a directory tree (the whole
//! data volume by default) and explain where the space goes.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use spacetrail_core::{
    DirState, NodeId, Reconciliation, ScanOptions, ScanProgress, ScanResult, Stats, scan,
};
use spacetrail_macos::{MacPlatform, apfs, whole_disk_root};

use super::Ctx;
use crate::render::{Align, bar, count, duration, pct, table};

/// What to show after scanning.
#[derive(Debug, Clone, Copy)]
pub(crate) struct View {
    /// Folder tree depth below the root.
    pub(crate) depth: usize,
    /// Hide folders smaller than this percentage of the root.
    pub(crate) min_pct: f64,
    /// Number of largest files to list.
    pub(crate) files: usize,
}

/// Runs a scan, showing live progress on an interactive stderr.
fn scan_with_progress(ctx: &Ctx, root: &Path, files: usize) -> anyhow::Result<ScanResult> {
    let progress = ScanProgress::default();
    let opts = ScanOptions {
        top_files: files.max(50),
        ..ScanOptions::default()
    };
    let show = !ctx.json && std::io::stderr().is_terminal();
    let done = AtomicBool::new(false);
    std::thread::scope(|s| {
        if show {
            s.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    eprint!(
                        "\r\x1b[2KScanning {} … {} files, {}",
                        ctx.display(root),
                        count(progress.files.load(Ordering::Relaxed)),
                        ctx.style.bytes(progress.bytes.load(Ordering::Relaxed)),
                    );
                    std::thread::sleep(Duration::from_millis(200));
                }
                eprint!("\r\x1b[2K");
            });
        }
        let r = scan(root, &MacPlatform, &opts, &progress);
        done.store(true, Ordering::Relaxed);
        r.map_err(anyhow::Error::from)
    })
}

#[derive(Debug, Serialize)]
struct Report {
    root: String,
    whole_disk: bool,
    started_at_unix: u64,
    elapsed_secs: f64,
    totals: Stats,
    reconciliation: Option<Reconciliation>,
    unreadable: Unreadables,
    skipped_mounts: Vec<String>,
    folders: Folder,
    largest_files: Vec<FileRow>,
}

#[derive(Debug, Serialize)]
struct Unreadables {
    /// macOS privacy (TCC) denials: lifted by Full Disk Access.
    privacy_protected: Vec<String>,
    /// Ordinary permissions: owned by root or another user.
    other_owner: u64,
    other_owner_examples: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Folder {
    path: String,
    alloc: u64,
    apparent: u64,
    files: u64,
    state: DirState,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<Folder>,
}

#[derive(Debug, Serialize)]
struct FileRow {
    path: String,
    alloc: u64,
    apparent: u64,
    modified_unix: Option<u64>,
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn folder(
    ctx: &Ctx,
    res: &ScanResult,
    id: NodeId,
    view: View,
    depth: usize,
    root_alloc: u64,
) -> Folder {
    let n = res.tree.get(id);
    let children = if depth < view.depth {
        res.tree
            .children_by_size(id)
            .into_iter()
            .filter(|c| {
                let a = res.tree.get(*c).total.alloc;
                let share = a as f64 * 100.0 / root_alloc.max(1) as f64;
                a > 0 && share >= view.min_pct
            })
            .map(|c| folder(ctx, res, c, view, depth + 1, root_alloc))
            .collect()
    } else {
        Vec::new()
    };
    Folder {
        path: display_root_aware(ctx, res, id),
        alloc: n.total.alloc,
        apparent: n.total.apparent,
        files: n.total.files,
        state: n.state,
        children,
    }
}

/// The data volume root reads better as `/`.
fn display_root_aware(ctx: &Ctx, res: &ScanResult, id: NodeId) -> String {
    let p = res.tree.path(id);
    if p == Path::new(spacetrail_macos::DATA_ROOT) {
        "/".to_owned()
    } else {
        ctx.display(&p)
    }
}

fn build(ctx: &Ctx, res: &ScanResult, whole_disk: bool, view: View) -> Report {
    let totals = res.tree.root().total;
    let reconciliation = whole_disk
        .then(|| apfs::container_of(&res.root).ok())
        .flatten()
        .and_then(|c| {
            c.data_volume()
                .map(|d| Reconciliation::new(d.consumed, totals.alloc))
        });
    let (privacy, other): (Vec<_>, Vec<_>) = res
        .unreadable
        .iter()
        .partition(|u| u.is_privacy_protected());
    Report {
        root: display_root_aware(ctx, res, NodeId::ROOT),
        whole_disk,
        started_at_unix: unix(res.started_at),
        elapsed_secs: res.elapsed.as_secs_f64(),
        totals,
        reconciliation,
        unreadable: Unreadables {
            privacy_protected: privacy.iter().map(|u| ctx.display(&u.path)).collect(),
            other_owner: other.len() as u64,
            other_owner_examples: other
                .iter()
                .take(20)
                .map(|u| ctx.display(&u.path))
                .collect(),
        },
        skipped_mounts: res.skipped_mounts.iter().map(|p| ctx.display(p)).collect(),
        folders: folder(ctx, res, NodeId::ROOT, view, 0, totals.alloc),
        largest_files: res
            .top_files
            .iter()
            .take(view.files)
            .map(|f| FileRow {
                path: ctx.display(&f.path),
                alloc: f.alloc,
                apparent: f.apparent,
                modified_unix: f.modified.map(unix),
            })
            .collect(),
    }
}

fn tree_rows(
    ctx: &Ctx,
    f: &Folder,
    root_alloc: u64,
    prefix: &str,
    last: bool,
    top: bool,
    rows: &mut Vec<Vec<String>>,
) {
    let (branch, child_prefix) = if top {
        (String::new(), String::new())
    } else if last {
        (format!("{prefix}└─ "), format!("{prefix}   "))
    } else {
        (format!("{prefix}├─ "), format!("{prefix}│  "))
    };
    let name = if top {
        f.path.clone()
    } else {
        Path::new(&f.path)
            .file_name()
            .map_or_else(|| f.path.clone(), |n| n.to_string_lossy().into_owned())
    };
    let note = match f.state {
        DirState::Complete => "",
        DirState::Unreadable => "  (unreadable: size unknown)",
        DirState::Dataless => "  (cloud-only)",
        DirState::Failed => "  (error)",
    };
    rows.push(vec![
        ctx.style.bytes(f.alloc),
        pct(f.alloc, root_alloc),
        bar(f.alloc, root_alloc, 16),
        format!("{branch}{name}{note}"),
    ]);
    for (i, c) in f.children.iter().enumerate() {
        tree_rows(
            ctx,
            c,
            root_alloc,
            &child_prefix,
            i + 1 == f.children.len(),
            false,
            rows,
        );
    }
}

/// Groups paths by parent folder: `(parent, count)`, most frequent first.
fn group_by_parent(paths: &[String]) -> Vec<(String, usize)> {
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    for p in paths {
        let parent = Path::new(p)
            .parent()
            .map_or_else(|| p.clone(), |x| x.display().to_string());
        *groups.entry(parent).or_default() += 1;
    }
    let mut v: Vec<_> = groups.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

fn render(ctx: &Ctx, r: &Report) -> String {
    let b = |n| ctx.style.bytes(n);
    let t = &r.totals;
    let mut o = String::new();
    let _ = writeln!(
        o,
        "Scanned {}{} in {} · {} files · {} folders\n",
        r.root,
        if r.whole_disk { " (data volume)" } else { "" },
        duration(Duration::from_secs_f64(r.elapsed_secs)),
        count(t.files),
        count(t.dirs)
    );
    let mut rows = vec![vec!["Measured".to_owned(), b(t.alloc), String::new()]];
    let privacy = r.unreadable.privacy_protected.len();
    if privacy > 0 {
        rows.push(vec![
            "Privacy-protected".into(),
            format!("{} folders", count(privacy as u64)),
            "size unknown; grant Full Disk Access (see `spacetrail doctor`)".into(),
        ]);
    }
    if r.unreadable.other_owner > 0 {
        rows.push(vec![
            "Not yours".into(),
            format!("{} folders", count(r.unreadable.other_owner)),
            "owned by root or other users; size unknown".into(),
        ]);
    }
    if t.dataless > 0 {
        rows.push(vec![
            "Cloud-only".into(),
            format!("{} items", count(t.dataless)),
            format!(
                "{} of files kept in the cloud, 0 B on this disk",
                b(t.dataless_bytes)
            ),
        ]);
    }
    if let Some(rc) = &r.reconciliation {
        if rc.unattributed > 0 {
            rows.push(vec![
                "Not measured".into(),
                b(rc.unattributed),
                format!(
                    "data volume uses {}; the rest is in unreadable folders, APFS metadata and snapshots",
                    b(rc.data_volume)
                ),
            ]);
        } else if rc.overcounted > 0 {
            rows.push(vec![
                "Over-counted".into(),
                b(rc.overcounted),
                format!(
                    "measured more than the data volume's {} (APFS clones share blocks)",
                    b(rc.data_volume)
                ),
            ]);
        }
    }
    if t.hardlink_dupes > 0 {
        rows.push(vec![
            "Hard links".into(),
            format!("{} extra", count(t.hardlink_dupes)),
            "counted once".into(),
        ]);
    }
    o.push_str(&table(&rows, &[Align::Left, Align::Right, Align::Left], 2));

    let _ = writeln!(o, "\nLargest folders");
    let mut rows = Vec::new();
    tree_rows(ctx, &r.folders, r.folders.alloc, "", true, true, &mut rows);
    o.push_str(&table(
        &rows,
        &[Align::Right, Align::Right, Align::Left, Align::Left],
        2,
    ));

    if !r.largest_files.is_empty() {
        let _ = writeln!(o, "\nLargest files");
        let rows: Vec<_> = r
            .largest_files
            .iter()
            .map(|f| vec![b(f.alloc), f.path.clone()])
            .collect();
        o.push_str(&table(&rows, &[Align::Right, Align::Left], 2));
    }

    if privacy > 0 {
        let _ = writeln!(o, "\nPrivacy-protected folders (by location)");
        let rows: Vec<_> = group_by_parent(&r.unreadable.privacy_protected)
            .into_iter()
            .take(10)
            .map(|(parent, n)| vec![count(n as u64), parent])
            .collect();
        o.push_str(&table(&rows, &[Align::Right, Align::Left], 2));
    }
    o
}

pub(crate) fn run(ctx: &Ctx, path: Option<PathBuf>, view: View) -> anyhow::Result<()> {
    let whole_disk = path.is_none();
    let root = path.unwrap_or_else(whole_disk_root);
    let res = scan_with_progress(ctx, &root, view.files)?;
    let report = build(ctx, &res, whole_disk, view);
    ctx.emit(&report, || render(ctx, &report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_unreadable_by_parent() {
        let paths = vec![
            "~/Library/Group Containers/a".to_owned(),
            "~/Library/Group Containers/b".to_owned(),
            "~/Library/Mail".to_owned(),
        ];
        assert_eq!(
            group_by_parent(&paths),
            vec![
                ("~/Library/Group Containers".to_owned(), 2),
                ("~/Library".to_owned(), 1)
            ]
        );
    }
}
