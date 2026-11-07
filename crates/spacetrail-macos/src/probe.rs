//! Read-only system probes.
//!
//! Apart from the cleaners executor, this is the only place allowed to spawn
//! processes, and only from a fixed allow-list of read-only commands with
//! absolute paths and a minimal environment. Nothing here modifies the system.

use std::io;
use std::process::{Command, Stdio};

/// The complete list of commands spacetrail runs to *read* system state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Probe {
    /// APFS containers and per-volume usage, as a property list.
    DiskutilApfsList,
    /// Local Time Machine snapshots on the boot volume.
    TmutilListLocalSnapshots,
}

impl Probe {
    fn argv(self) -> &'static [&'static str] {
        match self {
            Self::DiskutilApfsList => &["/usr/sbin/diskutil", "apfs", "list", "-plist"],
            Self::TmutilListLocalSnapshots => &["/usr/bin/tmutil", "listlocalsnapshots", "/"],
        }
    }
}

/// Runs a probe and returns its stdout. Fails if the command exits non-zero.
pub(crate) fn run(probe: Probe) -> io::Result<Vec<u8>> {
    let argv = probe.argv();
    #[expect(
        clippy::disallowed_methods,
        reason = "fixed allow-list of read-only system probes"
    )]
    let mut cmd = Command::new(argv[0]);
    let out = cmd
        .args(&argv[1..])
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(io::Error::other(format!(
            "{} exited with {}: {}",
            argv.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}
