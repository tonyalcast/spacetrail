//! One-call snapshot of the disk: capacity, APFS split and permissions.

use std::io;
use std::path::Path;

use serde::Serialize;
use spacetrail_core::{Container, VolumeStats};

use crate::{FdaStatus, apfs, fda, volume, whole_disk_root};

/// Capacity, per-volume usage and Full Disk Access state.
#[derive(Debug, Clone, Serialize)]
pub struct DiskStatus {
    /// Capacity figures of the data volume.
    pub volume: VolumeStats,
    /// The APFS container split, if available.
    pub container: Option<Container>,
    /// Whether this process has Full Disk Access.
    pub full_disk_access: FdaStatus,
}

impl DiskStatus {
    /// `(total, free)` of the whole disk (container when known).
    #[must_use]
    pub fn total_free(&self) -> (u64, u64) {
        self.container
            .as_ref()
            .map_or((self.volume.total, self.volume.free), |c| (c.total, c.free))
    }
}

/// Collects the snapshot. `home` is used for the Full Disk Access probe.
///
/// # Errors
/// Fails only if the data volume's capacity cannot be read.
pub fn disk_status(home: Option<&Path>) -> io::Result<DiskStatus> {
    let root = whole_disk_root();
    Ok(DiskStatus {
        volume: volume::volume_stats(&root)?,
        container: apfs::container_of(&root).ok(),
        full_disk_access: home.map_or(FdaStatus::Unknown, fda::probe),
    })
}
