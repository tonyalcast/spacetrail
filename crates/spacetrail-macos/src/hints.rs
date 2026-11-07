//! Known consumers of space outside what a user-level scan can see or
//! attribute: swap, the hibernation image, local Time Machine snapshots.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::probe::{self, Probe};

/// Hibernation image path.
pub const SLEEPIMAGE: &str = "/private/var/vm/sleepimage";

/// Allocated bytes of the hibernation image, if present.
#[must_use]
pub fn sleepimage_bytes() -> Option<u64> {
    fs::symlink_metadata(Path::new(SLEEPIMAGE))
        .ok()
        .map(|m| m.blocks() * 512)
}

/// Names of local Time Machine snapshots on the boot volume.
/// Their size is not reported by any public tool; they count as purgeable.
///
/// # Errors
/// Fails if `tmutil` cannot be run.
pub fn local_snapshots() -> std::io::Result<Vec<String>> {
    let out = probe::run(Probe::TmutilListLocalSnapshots)?;
    Ok(parse_snapshots(&String::from_utf8_lossy(&out)))
}

fn parse_snapshots(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("com.apple."))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tmutil_output() {
        let out = "Snapshots for disk /:\ncom.apple.TimeMachine.2026-09-30-101010.local\ncom.apple.os.update-ABC\n";
        assert_eq!(parse_snapshots(out).len(), 2);
        assert!(parse_snapshots("Snapshots for disk /:\n").is_empty());
    }
}
