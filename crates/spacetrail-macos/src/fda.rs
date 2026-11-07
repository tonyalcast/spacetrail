//! Full Disk Access (TCC) detection.
//!
//! There is no API to ask TCC about our own process; the standard technique
//! is to try to read a location that only FDA unlocks.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Result of the Full Disk Access probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FdaStatus {
    /// A protected location was readable.
    Granted,
    /// A protected location returned a permission error.
    Denied {
        /// The location that was refused.
        probe: PathBuf,
    },
    /// No protected location exists to test against.
    Unknown,
}

/// Protected locations, most reliable first. Probing opens or lists them
/// without reading any content.
fn candidates(home: &Path) -> [PathBuf; 3] {
    [
        home.join("Library/Application Support/com.apple.TCC/TCC.db"),
        home.join("Library/Safari"),
        home.join("Library/Mail"),
    ]
}

/// Probes whether this process has Full Disk Access.
#[must_use]
pub fn probe(home: &Path) -> FdaStatus {
    for path in candidates(home) {
        let result = if path.extension().is_some() {
            fs::File::open(&path).map(drop)
        } else {
            fs::read_dir(&path).map(drop)
        };
        match result {
            Ok(()) => return FdaStatus::Granted,
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                return FdaStatus::Denied { probe: path };
            }
            Err(_) => {}
        }
    }
    FdaStatus::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_home_is_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(probe(tmp.path()), FdaStatus::Unknown);
    }

    #[test]
    fn readable_location_means_granted() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("Library/Safari")).unwrap();
        assert_eq!(probe(tmp.path()), FdaStatus::Granted);
    }
}
