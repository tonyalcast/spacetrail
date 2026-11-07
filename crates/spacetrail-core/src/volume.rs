//! Volume and container capacity model, and the reconciliation of "used"
//! space between scanned data and everything else.

use std::path::PathBuf;

use serde::Serialize;

/// Capacity figures for one mounted volume.
///
/// On APFS these numbers legitimately disagree with each other, which
/// confuses users; the UI shows them side by side with explanations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct VolumeStats {
    /// Size of the volume (on APFS: of the whole container).
    pub total: u64,
    /// Bytes used according to `statfs`. On APFS this is the **container's**
    /// used space (all volumes), not this volume's; see `Container` for the split.
    pub used: u64,
    /// Bytes free for an unprivileged user right now (`statfs` `f_bavail`).
    pub free: u64,
    /// Bytes available for "important" usage: free plus purgeable space the
    /// OS would reclaim on demand. This is what Finder shows as "Available".
    pub available_important: Option<u64>,
    /// Bytes available for opportunistic usage (excludes most purgeable space).
    pub available_opportunistic: Option<u64>,
}

impl VolumeStats {
    /// Estimate of purgeable space: what the OS can free on demand (caches,
    /// local snapshots, optimized-storage files) beyond plain free space.
    #[must_use]
    pub fn purgeable_estimate(&self) -> Option<u64> {
        self.available_important
            .map(|a| a.saturating_sub(self.free))
    }
}

/// Role of a volume inside an APFS container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeRole {
    /// Sealed, read-only macOS system volume.
    System,
    /// User and application data (what spacetrail scans).
    Data,
    /// Swap files.
    Vm,
    /// Boot support files.
    Preboot,
    /// Recovery OS.
    Recovery,
    /// Staged OS updates.
    Update,
    /// Anything else (role name as reported).
    Other(String),
}

impl VolumeRole {
    /// Short label, e.g. "VM (swap)".
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::System => "System",
            Self::Data => "Data",
            Self::Vm => "VM (swap)",
            Self::Preboot => "Preboot",
            Self::Recovery => "Recovery",
            Self::Update => "Update",
            Self::Other(name) => name,
        }
    }

    /// Short human description of what lives on a volume with this role.
    #[must_use]
    pub fn explanation(&self) -> &str {
        match self {
            Self::System => "macOS itself (sealed, read-only)",
            Self::Data => "your files, apps and caches",
            Self::Vm => "swap: memory paged out to disk; grows under memory pressure",
            Self::Preboot => "boot support files for installed macOS versions",
            Self::Recovery => "the Recovery OS",
            Self::Update => "staged macOS updates",
            Self::Other(_) => "other system volume",
        }
    }
}

/// One volume of a container and the space it consumes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VolumeUsage {
    /// Volume name (e.g. "Macintosh HD - Data").
    pub name: String,
    /// Role in the container.
    pub role: VolumeRole,
    /// Bytes consumed in the container.
    pub consumed: u64,
    /// BSD device identifier (e.g. `disk3s5`).
    pub device: String,
    /// Mount point, if mounted.
    pub mount: Option<PathBuf>,
}

/// An APFS container: one pool of space shared by several volumes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Container {
    /// Container device (e.g. `disk3`).
    pub device: String,
    /// Total capacity in bytes.
    pub total: u64,
    /// Free bytes in the container.
    pub free: u64,
    /// Volumes, largest first.
    pub volumes: Vec<VolumeUsage>,
}

impl Container {
    /// Bytes in use across the container (total − free).
    #[must_use]
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    /// Bytes used that no volume accounts for (container metadata, snapshots
    /// counted at container level, rounding). Usually small.
    #[must_use]
    pub fn overhead(&self) -> u64 {
        let by_volumes: u64 = self.volumes.iter().map(|v| v.consumed).sum();
        self.used().saturating_sub(by_volumes)
    }

    /// The data volume, if present.
    #[must_use]
    pub fn data_volume(&self) -> Option<&VolumeUsage> {
        self.volumes.iter().find(|v| v.role == VolumeRole::Data)
    }
}

/// How the data volume's consumed space compares to what a scan measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Reconciliation {
    /// Bytes the data volume consumes according to APFS.
    pub data_volume: u64,
    /// Bytes the scan measured.
    pub measured: u64,
    /// Used but not measured: unreadable folders, other users' files, APFS
    /// metadata and snapshot-held blocks.
    pub unattributed: u64,
    /// Measured but not used: blocks shared by APFS clones are counted once
    /// per clone by the scan.
    pub overcounted: u64,
}

impl Reconciliation {
    /// Compares the data volume's consumption with a scan's measurement.
    #[must_use]
    pub fn new(data_volume: u64, measured: u64) -> Self {
        Self {
            data_volume,
            measured,
            unattributed: data_volume.saturating_sub(measured),
            overcounted: measured.saturating_sub(data_volume),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconciliation() {
        let r = Reconciliation::new(100, 70);
        assert_eq!((r.unattributed, r.overcounted), (30, 0));
        let r = Reconciliation::new(100, 120);
        assert_eq!((r.unattributed, r.overcounted), (0, 20));
    }

    #[test]
    fn container_math() {
        let c = Container {
            device: "disk3".into(),
            total: 1000,
            free: 100,
            volumes: vec![
                VolumeUsage {
                    name: "Data".into(),
                    role: VolumeRole::Data,
                    consumed: 700,
                    device: "disk3s5".into(),
                    mount: None,
                },
                VolumeUsage {
                    name: "VM".into(),
                    role: VolumeRole::Vm,
                    consumed: 150,
                    device: "disk3s6".into(),
                    mount: None,
                },
            ],
        };
        assert_eq!(c.used(), 900);
        assert_eq!(c.overhead(), 50);
        assert_eq!(c.data_volume().unwrap().consumed, 700);
    }

    #[test]
    fn purgeable() {
        let v = VolumeStats {
            free: 10,
            available_important: Some(25),
            ..VolumeStats::default()
        };
        assert_eq!(v.purgeable_estimate(), Some(15));
        assert_eq!(VolumeStats::default().purgeable_estimate(), None);
    }
}
