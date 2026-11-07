//! Development tasks for spacetrail. Run with `cargo xtask <task>`.

use anyhow::bail;

const USAGE: &str = "usage: cargo xtask <task>

tasks:
  check-safety   fail if `disallowed_methods` is allowed outside the executor";

fn main() -> anyhow::Result<()> {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("check-safety") => check_safety(),
        _ => {
            eprintln!("{USAGE}");
            bail!("unknown or missing task");
        }
    }
}

/// The only place allowed to opt out of `clippy::disallowed_methods`.
const EXECUTOR_DIR: &str = "crates/spacetrail-cleaners/src/executor";

/// Enforces CLAUDE.md safety rule 2: deletion/process APIs may only be
/// unlocked inside the cleaners executor.
fn check_safety() -> anyhow::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root");
    let mut offenders = Vec::new();
    visit(&root.join("crates"), &mut |path| {
        let rel = path.strip_prefix(root).unwrap_or(path);
        if rel.starts_with(EXECUTOR_DIR) || rel.extension().is_none_or(|e| e != "rs") {
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
    if offenders.is_empty() {
        println!("check-safety: ok");
        Ok(())
    } else {
        bail!(
            "`disallowed_methods` used outside {EXECUTOR_DIR}:\n  {}",
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
