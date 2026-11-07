//! Read-only experiments on the real machine (see docs/NOTES.md).
//!
//! cargo run -p spacetrail-macos --example spikes -- s1|s2|s3

use std::path::{Path, PathBuf};

use spacetrail_core::{DirState, ScanOptions, ScanProgress, human, scan};
use spacetrail_macos::{MacPlatform, apfs, dataless, fda, hints, volume, whole_disk_root};

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME set"))
}

/// S1: dataless files under iCloud Drive and File Provider roots.
fn s1() {
    spacetrail_macos::prevent_materialization().expect("iopolicy");
    for rel in ["Library/Mobile Documents", "Library/CloudStorage"] {
        let root = home().join(rel);
        let Ok(res) = scan(
            &root,
            &MacPlatform,
            &ScanOptions::default(),
            &ScanProgress::default(),
        ) else {
            println!("{rel}: not scannable");
            continue;
        };
        let t = res.tree.root().total;
        let dataless_dirs: Vec<_> = res
            .tree
            .ids()
            .filter(|id| res.tree.get(*id).state == DirState::Dataless)
            .collect();
        println!(
            "{rel}: local {} in {} files; cloud-only entries {} ({} logical, files) ; dataless dirs {}; unreadable {} ; {:?}",
            human(t.alloc),
            t.files,
            t.dataless,
            human(t.dataless_bytes),
            dataless_dirs.len(),
            res.unreadable.len(),
            res.elapsed
        );
        // Re-check: a sample of dataless dirs must still be dataless after the scan.
        let still = dataless_dirs
            .iter()
            .take(50)
            .filter(|id| {
                std::fs::symlink_metadata(res.tree.path(**id))
                    .map(|m| dataless::is_dataless(&m))
                    .unwrap_or(false)
            })
            .count();
        println!(
            "  re-check: {still}/{} sampled dataless dirs still dataless",
            dataless_dirs.len().min(50)
        );
    }
}

/// S2: what is unreadable in ~/Library without Full Disk Access.
fn s2() {
    println!("FDA probe: {:?}", fda::probe(&home()));
    let root = home().join("Library");
    let res = scan(
        &root,
        &MacPlatform,
        &ScanOptions::default(),
        &ScanProgress::default(),
    )
    .unwrap();
    println!(
        "~/Library: {} readable, {} files, {} unreadable dirs, {:?}",
        human(res.tree.root().total.alloc),
        res.tree.root().total.files,
        res.unreadable.len(),
        res.elapsed
    );
    for u in &res.unreadable {
        println!(
            "  {} ({:?})",
            u.path.strip_prefix(home()).unwrap_or(&u.path).display(),
            u.kind
        );
    }
}

/// S3: capacity numbers from every source.
fn s3() {
    for p in ["/", "/System/Volumes/Data"] {
        let s = volume::statfs(Path::new(p)).unwrap();
        let v = volume::volume_stats(Path::new(p)).unwrap();
        println!(
            "{p}: dev {} total {} used {} free(statfs) {} important {:?} opportunistic {:?} purgeable≈{:?}",
            s.mounted_from,
            human(v.total),
            human(v.used),
            human(v.free),
            v.available_important.map(human).map(|h| h.to_string()),
            v.available_opportunistic.map(human).map(|h| h.to_string()),
            v.purgeable_estimate().map(human).map(|h| h.to_string()),
        );
    }
    let c = apfs::container_of(&whole_disk_root()).unwrap();
    println!(
        "container {}: total {} free {} used {} overhead {}",
        c.device,
        human(c.total),
        human(c.free),
        human(c.used()),
        human(c.overhead())
    );
    for v in &c.volumes {
        println!(
            "  {:<22} {:?} {:>9} {:?}",
            v.name,
            v.role,
            human(v.consumed).to_string(),
            v.mount
        );
    }
    println!(
        "sleepimage: {:?}",
        hints::sleepimage_bytes().map(human).map(|h| h.to_string())
    );
    println!("local snapshots: {:?}", hints::local_snapshots());
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("s1") => s1(),
        Some("s2") => s2(),
        Some("s3") => s3(),
        _ => eprintln!("usage: spikes s1|s2|s3"),
    }
}
