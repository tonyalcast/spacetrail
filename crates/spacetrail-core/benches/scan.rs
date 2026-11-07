//! Scanner throughput on a generated tree.
//!
//! `SPACETRAIL_BENCH_FILES` sets the number of files (default 100 000).
//! To bench an existing tree instead (e.g. one made with
//! `cargo xtask gen-tree`), set `SPACETRAIL_BENCH_DIR`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use spacetrail_core::{GenericPlatform, ScanOptions, ScanProgress, scan};

const FILES_PER_DIR: usize = 100;
const DIRS_PER_DIR: usize = 10;

/// Creates `files` empty files (no data blocks, so it is cheap on a full disk) spread over a balanced tree of directories.
fn generate(root: &Path, files: usize) {
    let mut remaining = files;
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    while remaining > 0 {
        let dir = queue
            .pop_front()
            .expect("queue never empties while filling");
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..FILES_PER_DIR.min(remaining) {
            std::fs::write(dir.join(format!("f{i}")), b"").unwrap();
        }
        remaining = remaining.saturating_sub(FILES_PER_DIR);
        for d in 0..DIRS_PER_DIR {
            queue.push_back(dir.join(format!("d{d}")));
        }
    }
}

fn bench_scan(c: &mut Criterion) {
    let (_guard, root, files): (Option<tempfile::TempDir>, PathBuf, u64) =
        if let Ok(dir) = std::env::var("SPACETRAIL_BENCH_DIR") {
            let p = PathBuf::from(dir);
            let files = scan(
                &p,
                &GenericPlatform,
                &ScanOptions::default(),
                &ScanProgress::default(),
            )
            .unwrap()
            .tree
            .root()
            .total
            .files;
            (None, p, files)
        } else {
            let n: usize = std::env::var("SPACETRAIL_BENCH_FILES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100_000);
            let tmp = tempfile::tempdir().unwrap();
            generate(tmp.path(), n);
            let p = tmp.path().to_path_buf();
            (Some(tmp), p, n as u64)
        };

    let mut group = c.benchmark_group("scan");
    group.throughput(Throughput::Elements(files));
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));
    group.bench_function(format!("{files}_files"), |b| {
        b.iter(|| {
            let r = scan(
                &root,
                &GenericPlatform,
                &ScanOptions::default(),
                &ScanProgress::default(),
            )
            .unwrap();
            assert!(r.tree.root().total.files >= files);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_scan);
criterion_main!(benches);
