//! Development tasks for spacetrail. Run with `cargo xtask <task>`.

use anyhow::bail;

const USAGE: &str = "usage: cargo xtask <task>

tasks:
  check-safety              fail if `disallowed_methods` is allowed outside the executor
  gen-tree <DIR> [FILES]    create a synthetic tree of empty files (default 100000)";

fn main() -> anyhow::Result<()> {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("check-safety") => check_safety(),
        Some("gen-tree") => {
            let Some(dir) = std::env::args().nth(2) else {
                bail!("usage: cargo xtask gen-tree <DIR> [FILES]");
            };
            let files = std::env::args()
                .nth(3)
                .map(|n| n.parse())
                .transpose()?
                .unwrap_or(100_000);
            gen_tree(std::path::Path::new(&dir), files)
        }
        _ => {
            eprintln!("{USAGE}");
            bail!("unknown or missing task");
        }
    }
}

/// The only places allowed to opt out of `clippy::disallowed_methods`: the
/// cleaners executor (deletion + cleaner commands) and the fixed allow-list
/// of read-only system probes.
const ALLOWED: &[&str] = &[
    "crates/spacetrail-cleaners/src/executor",
    "crates/spacetrail-macos/src/probe.rs",
];

/// Enforces CLAUDE.md safety rule 2: deletion/process APIs may only be
/// unlocked inside the cleaners executor.
fn check_safety() -> anyhow::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root");
    let mut offenders = Vec::new();
    visit(&root.join("crates"), &mut |path| {
        let rel = path.strip_prefix(root).unwrap_or(path);
        if ALLOWED.iter().any(|a| rel.starts_with(a)) || rel.extension().is_none_or(|e| e != "rs") {
            return;
        }
        if let Ok(text) = std::fs::read_to_string(path) {
            for (i, line) in text.lines().enumerate() {
                if line.contains("disallowed_methods") {
                    offenders.push(format!("{}:{}", rel.display(), i + 1));
                }
            }
        }
    })?;
    // Removal primitives: only the executor may call the Trash; nobody may
    // call Foundation's permanent delete.
    visit(&root.join("crates"), &mut |path| {
        let rel = path.strip_prefix(root).unwrap_or(path);
        if rel.extension().is_none_or(|e| e != "rs") {
            return;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let trash_ok = rel.starts_with("crates/spacetrail-cleaners/src/executor")
            || rel.starts_with("crates/spacetrail-macos/src/trash.rs");
        for (i, line) in text.lines().enumerate() {
            let calls_trash = line.contains("move_to_trash(") || line.contains("trashItemAtURL");
            if (calls_trash && !trash_ok) || line.contains("removeItemAtURL") {
                offenders.push(format!("{}:{} (removal primitive)", rel.display(), i + 1));
            }
        }
    })?;
    visit(&root.join("apps"), &mut |path| {
        let rel = path.strip_prefix(root).unwrap_or(path);
        if rel.extension().is_none_or(|e| e != "rs") {
            return;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        for (i, line) in text.lines().enumerate() {
            if line.contains("move_to_trash(")
                || line.contains("removeItemAtURL")
                || line.contains("disallowed_methods")
            {
                offenders.push(format!(
                    "{}:{} (apps must go through the executor)",
                    rel.display(),
                    i + 1
                ));
            }
        }
    })?;
    if offenders.is_empty() {
        println!("check-safety: ok");
        Ok(())
    } else {
        bail!(
            "safety violations (deletion/process APIs outside {ALLOWED:?} or the executor):\n  {}",
            offenders.join("\n  ")
        )
    }
}

fn visit(dir: &std::path::Path, f: &mut dyn FnMut(&std::path::Path)) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            visit(&path, f)?;
        } else {
            f(&path);
        }
    }
    Ok(())
}

/// Creates `files` empty files in a balanced tree (100 files and 10 subdirs
/// per directory). Refuses to write into an existing non-empty directory.
fn gen_tree(root: &std::path::Path, files: usize) -> anyhow::Result<()> {
    if root.exists() && std::fs::read_dir(root)?.next().is_some() {
        bail!("{} exists and is not empty", root.display());
    }
    let mut remaining = files;
    let mut dirs = 0usize;
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    while remaining > 0 {
        let dir = queue
            .pop_front()
            .expect("queue never empties while filling");
        std::fs::create_dir_all(&dir)?;
        dirs += 1;
        for i in 0..100.min(remaining) {
            std::fs::write(dir.join(format!("f{i}")), b"")?;
        }
        remaining = remaining.saturating_sub(100);
        for d in 0..10 {
            queue.push_back(dir.join(format!("d{d}")));
        }
    }
    println!(
        "created {files} files in {dirs} directories under {}",
        root.display()
    );
    Ok(())
}
