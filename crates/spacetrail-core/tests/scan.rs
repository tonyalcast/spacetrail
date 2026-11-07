//! Walker behavior on real temporary trees.

use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use proptest::prelude::*;
use spacetrail_core::{
    DirState, GenericPlatform, NodeId, Platform, ScanOptions, ScanProgress, ScanResult, scan,
};

fn run(root: &Path) -> ScanResult {
    run_with(root, &GenericPlatform, &ScanOptions::default())
}

fn run_with(root: &Path, platform: &dyn Platform, opts: &ScanOptions) -> ScanResult {
    scan(root, platform, opts, &ScanProgress::default()).expect("scan succeeds")
}

fn write(path: &Path, len: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![7u8; len]).unwrap();
}

fn alloc(path: &Path) -> u64 {
    fs::symlink_metadata(path).unwrap().blocks() * 512
}

/// Sequential reference implementation: unique-inode allocated bytes of
/// everything under `root` (root included), without following symlinks.
fn reference_alloc(root: &Path) -> u64 {
    fn go(p: &Path, seen: &mut HashSet<(u64, u64)>) -> u64 {
        let m = fs::symlink_metadata(p).unwrap();
        let mut total = 0;
        if m.is_dir() || m.nlink() <= 1 || seen.insert((m.dev(), m.ino())) {
            total += m.blocks() * 512;
        }
        if m.is_dir() {
            for e in fs::read_dir(p).unwrap() {
                total += go(&e.unwrap().path(), seen);
            }
        }
        total
    }
    go(root, &mut HashSet::new())
}

#[test]
fn totals_match_reference_and_counts_are_right() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    write(&r.join("a/one.bin"), 10_000);
    write(&r.join("a/b/two.bin"), 50_000);
    write(&r.join("c/three.bin"), 1);
    fs::create_dir(r.join("empty")).unwrap();

    let res = run(r);
    let root = res.tree.root();
    assert_eq!(root.total.alloc, reference_alloc(r));
    assert_eq!(root.total.files, 3);
    assert_eq!(root.total.dirs, 4); // a, a/b, c, empty
    let a = res.tree.find(&r.join("a")).unwrap();
    assert_eq!(res.tree.get(a).total.files, 2);
    assert_eq!(
        res.tree.get(a).total.alloc,
        alloc(&r.join("a"))
            + alloc(&r.join("a/b"))
            + alloc(&r.join("a/one.bin"))
            + alloc(&r.join("a/b/two.bin"))
    );
    assert!(res.unreadable.is_empty());
    assert!(!res.cancelled);
}

#[test]
fn hard_links_are_counted_once() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    write(&r.join("x/data.bin"), 1_000_000);
    fs::create_dir_all(r.join("y")).unwrap();
    fs::hard_link(r.join("x/data.bin"), r.join("y/link1.bin")).unwrap();
    fs::hard_link(r.join("x/data.bin"), r.join("y/link2.bin")).unwrap();

    let res = run(r);
    let root = res.tree.root();
    assert_eq!(root.total.alloc, reference_alloc(r));
    assert_eq!(root.total.files, 1);
    assert_eq!(root.total.hardlink_dupes, 2);
}

#[test]
fn symlinks_are_never_followed_and_loops_terminate() {
    let outside = tempfile::tempdir().unwrap();
    write(&outside.path().join("big.bin"), 2_000_000);

    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    write(&r.join("d/small.bin"), 100);
    symlink(r, r.join("d/loop")).unwrap();
    symlink(outside.path(), r.join("to_outside")).unwrap();
    symlink(outside.path().join("big.bin"), r.join("big_link")).unwrap();

    let res = run(r);
    let root = res.tree.root();
    assert_eq!(root.total.alloc, reference_alloc(r));
    assert!(
        root.total.alloc < 2_000_000,
        "symlink target must not be counted"
    );
    assert_eq!(root.total.files, 4); // small.bin + 3 symlinks
    assert_eq!(root.total.dirs, 1);
    assert!(res.top_files.iter().all(|f| f.path.ends_with("small.bin")));
}

/// Restores permissions so the temp dir can be cleaned up even if the test fails.
struct Unlock(PathBuf);
impl Drop for Unlock {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

#[test]
fn unreadable_directories_are_reported_not_zeroed() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    write(&r.join("ok/file.bin"), 1_000);
    write(&r.join("secret/hidden.bin"), 5_000_000);
    let secret = r.join("secret");
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
    let _unlock = Unlock(secret.clone());
    if fs::read_dir(&secret).is_ok() {
        eprintln!("running as root: permission test is meaningless, skipping");
        return;
    }

    let res = run(r);
    assert_eq!(res.unreadable.len(), 1);
    assert_eq!(res.unreadable[0].path, secret);
    assert_eq!(res.unreadable[0].kind, std::io::ErrorKind::PermissionDenied);
    // Plain Unix permissions → EACCES, not a privacy (TCC) denial.
    assert!(!res.unreadable[0].is_privacy_protected());
    let id = res.tree.find(&secret).unwrap();
    assert_eq!(res.tree.get(id).state, DirState::Unreadable);
    assert_eq!(res.tree.root().total.unreadable_dirs, 1);
    assert_eq!(res.tree.root().total.files, 1);
}

/// Marks specific inodes as cloud-only, like macOS `SF_DATALESS`.
#[derive(Debug)]
struct FakeCloud(HashSet<(u64, u64)>);

impl FakeCloud {
    fn new(paths: &[PathBuf]) -> Self {
        Self(
            paths
                .iter()
                .map(|p| {
                    let m = fs::symlink_metadata(p).unwrap();
                    (m.dev(), m.ino())
                })
                .collect(),
        )
    }
}

impl Platform for FakeCloud {
    fn is_dataless(&self, meta: &Metadata) -> bool {
        self.0.contains(&(meta.dev(), meta.ino()))
    }
}

#[test]
fn dataless_entries_are_counted_but_not_read() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    write(&r.join("icloud/doc.pages"), 300_000);
    write(&r.join("icloud/local.txt"), 10);
    write(&r.join("icloud/folder/inner.bin"), 900_000);
    let platform = FakeCloud::new(&[r.join("icloud/doc.pages"), r.join("icloud/folder")]);

    let res = run_with(r, &platform, &ScanOptions::default());
    let total = res.tree.root().total;
    assert_eq!(total.dataless, 2);
    assert_eq!(total.dataless_bytes, 300_000);
    assert_eq!(total.files, 1, "only local.txt counts as a local file");
    let folder = res.tree.find(&r.join("icloud/folder")).unwrap();
    assert_eq!(res.tree.get(folder).state, DirState::Dataless);
    assert_eq!(
        res.tree.children(folder).len(),
        0,
        "dataless dir must not be listed"
    );
    assert!(res.top_files.iter().all(|f| !f.path.ends_with("inner.bin")));
}

#[test]
fn top_files_are_bounded_and_sorted() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    for i in 1..=20 {
        write(&r.join(format!("d{}/f{i}.bin", i % 3)), i * 10_000);
    }
    let opts = ScanOptions {
        top_files: 5,
        ..ScanOptions::default()
    };
    let res = run_with(r, &GenericPlatform, &opts);
    assert_eq!(res.top_files.len(), 5);
    assert!(res.top_files.windows(2).all(|w| w[0].alloc >= w[1].alloc));
    assert!(res.top_files[0].path.ends_with("f20.bin"));
}

#[test]
fn cancelled_scan_is_marked() {
    let tmp = tempfile::tempdir().unwrap();
    write(&tmp.path().join("a/b/c.bin"), 10);
    let progress = ScanProgress::default();
    progress.cancel();
    let res = scan(
        tmp.path(),
        &GenericPlatform,
        &ScanOptions::default(),
        &progress,
    )
    .unwrap();
    assert!(res.cancelled);
}

#[test]
fn root_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("f");
    write(&file, 1);
    let p = ScanProgress::default();
    let o = ScanOptions::default();
    assert!(scan(&file, &GenericPlatform, &o, &p).is_err());
    assert!(scan(&tmp.path().join("missing"), &GenericPlatform, &o, &p).is_err());
}

/// A random tree: files given as (dir index, size), dirs as parent index,
/// plus hard links from an existing file into a dir.
#[derive(Debug, Clone)]
struct Spec {
    dir_parents: Vec<usize>,
    files: Vec<(usize, usize)>,
    links: Vec<(usize, usize)>,
}

fn spec() -> impl Strategy<Value = Spec> {
    (1usize..12)
        .prop_flat_map(|ndirs| {
            (
                (0..ndirs).map(|i| 0..=i).collect::<Vec<_>>(),
                prop::collection::vec((0..=ndirs, 0usize..40_000), 0..30),
                prop::collection::vec((0usize..30, 0..=ndirs), 0..8),
            )
        })
        .prop_map(|(dir_parents, files, links)| Spec {
            dir_parents,
            files,
            links,
        })
}

fn materialize(root: &Path, s: &Spec) -> Vec<PathBuf> {
    // dir 0 is the root; dir i+1 has parent dir_parents[i] (always an earlier dir).
    let mut dirs = vec![root.to_path_buf()];
    for (i, &parent) in s.dir_parents.iter().enumerate() {
        let d = dirs[parent].join(format!("d{i}"));
        fs::create_dir(&d).unwrap();
        dirs.push(d);
    }
    let mut files = Vec::new();
    for (i, &(dir, size)) in s.files.iter().enumerate() {
        let p = dirs[dir].join(format!("f{i}"));
        fs::write(&p, vec![1u8; size]).unwrap();
        files.push(p);
    }
    for (i, &(file, dir)) in s.links.iter().enumerate() {
        if let Some(src) = files.get(file) {
            fs::hard_link(src, dirs[dir].join(format!("l{i}"))).unwrap();
        }
    }
    dirs
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn walker_matches_reference(s in spec()) {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = materialize(tmp.path(), &s);
        let res = run(tmp.path());

        prop_assert_eq!(res.tree.root().total.alloc, reference_alloc(tmp.path()));
        prop_assert_eq!(res.tree.root().total.files, s.files.len() as u64);
        prop_assert_eq!(res.tree.root().total.dirs, s.dir_parents.len() as u64);
        prop_assert_eq!(res.tree.len(), dirs.len());
        // Every directory's total equals the sum of its own stats and its children's totals.
        for id in res.tree.ids() {
            let n = res.tree.get(id);
            let kids: u64 = res.tree.children(id).map(|c| res.tree.get(c).total.alloc).sum();
            prop_assert_eq!(n.total.alloc, n.own.alloc + kids);
        }
        prop_assert_eq!(res.tree.find(tmp.path()), Some(NodeId::ROOT));
    }
}
