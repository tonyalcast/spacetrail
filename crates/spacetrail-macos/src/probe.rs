//! Read-only system probes.
//!
//! Apart from the cleaners executor, this is the only place allowed to spawn
//! processes, and only from a fixed allow-list of read-only commands with
//! absolute paths and a minimal environment. Nothing here modifies the system.

use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The complete list of commands spacetrail runs to *read* system state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Probe {
    /// APFS containers and per-volume usage, as a property list.
    DiskutilApfsList,
    /// Local Time Machine snapshots on the boot volume.
    TmutilListLocalSnapshots,
    /// Docker disk usage summary (images, containers, volumes, build cache).
    DockerSystemDf,
}

/// Where Docker Desktop / Homebrew install the `docker` CLI.
const DOCKER_CANDIDATES: &[&str] = &[
    "/usr/local/bin/docker",
    "/opt/homebrew/bin/docker",
    "/Applications/Docker.app/Contents/Resources/bin/docker",
];

impl Probe {
    fn argv(self) -> Option<Vec<&'static str>> {
        Some(match self {
            Self::DiskutilApfsList => vec!["/usr/sbin/diskutil", "apfs", "list", "-plist"],
            Self::TmutilListLocalSnapshots => vec!["/usr/bin/tmutil", "listlocalsnapshots", "/"],
            Self::DockerSystemDf => {
                let docker = DOCKER_CANDIDATES.iter().find(|p| Path::new(p).exists())?;
                vec![docker, "system", "df", "--format", "{{json .}}"]
            }
        })
    }

    /// Extra environment a probe needs (the environment is otherwise cleared).
    fn needs_home(self) -> bool {
        matches!(self, Self::DockerSystemDf)
    }
}

/// Maximum time a probe may run before it is killed.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Runs a probe and returns its stdout. Fails if the command is not
/// installed, exits non-zero, or exceeds the timeout.
pub(crate) fn run(probe: Probe) -> io::Result<Vec<u8>> {
    let argv = probe.argv().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{probe:?}: command not installed"),
        )
    })?;
    #[expect(
        clippy::disallowed_methods,
        reason = "fixed allow-list of read-only system probes"
    )]
    let mut cmd = Command::new(argv[0]);
    cmd.args(&argv[1..])
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if probe.needs_home()
        && let Some(home) = std::env::var_os("HOME")
    {
        cmd.env("HOME", home);
    }
    let mut child = cmd.spawn()?;
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).map(|_| buf)
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{} timed out", argv.join(" ")),
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let out = reader
        .join()
        .map_err(|_| io::Error::other("reader panicked"))??;
    if status.success() {
        Ok(out)
    } else {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        Err(io::Error::other(format!(
            "{} exited with {status}: {}",
            argv.join(" "),
            err.trim()
        )))
    }
}
