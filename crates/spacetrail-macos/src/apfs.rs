//! APFS containers and per-volume usage, from `diskutil apfs list -plist`.

use std::io;
use std::path::PathBuf;

use serde::Deserialize;
use spacetrail_core::{Container, VolumeRole, VolumeUsage};

use crate::probe::{self, Probe};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ApfsList {
    containers: Vec<RawContainer>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawContainer {
    container_reference: String,
    capacity_ceiling: u64,
    capacity_free: u64,
    #[serde(default)]
    volumes: Vec<RawVolume>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawVolume {
    #[serde(default)]
    name: String,
    device_identifier: String,
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    capacity_in_use: u64,
}

fn role(roles: &[String]) -> VolumeRole {
    match roles.first().map(String::as_str) {
        Some("System") => VolumeRole::System,
        Some("Data") => VolumeRole::Data,
        Some("VM") => VolumeRole::Vm,
        Some("Preboot") => VolumeRole::Preboot,
        Some("Recovery") => VolumeRole::Recovery,
        Some("Update") => VolumeRole::Update,
        Some(other) => VolumeRole::Other(other.to_owned()),
        None => VolumeRole::Other("none".to_owned()),
    }
}

/// Parses `diskutil apfs list -plist` output and returns the container
/// holding `device` (e.g. `disk3s5`). `mount_of` resolves mount points.
///
/// # Errors
/// Fails on malformed plist data or if no container holds `device`.
pub fn parse_container(
    plist_bytes: &[u8],
    device: &str,
    mount_of: impl Fn(&str) -> Option<PathBuf>,
) -> io::Result<Container> {
    let list: ApfsList = plist::from_bytes(plist_bytes).map_err(io::Error::other)?;
    let raw = list
        .containers
        .into_iter()
        .find(|c| c.volumes.iter().any(|v| v.device_identifier == device))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no APFS container holds {device}"),
            )
        })?;
    let mut volumes: Vec<VolumeUsage> = raw
        .volumes
        .into_iter()
        .map(|v| VolumeUsage {
            mount: mount_of(&v.device_identifier),
            role: role(&v.roles),
            name: v.name,
            consumed: v.capacity_in_use,
            device: v.device_identifier,
        })
        .collect();
    volumes.sort_by_key(|v| std::cmp::Reverse(v.consumed));
    Ok(Container {
        device: raw.container_reference,
        total: raw.capacity_ceiling,
        free: raw.capacity_free,
        volumes,
    })
}

/// The container that holds the data volume mounted at `data_mount`
/// (normally `/System/Volumes/Data`).
///
/// # Errors
/// Fails if `statfs`, `diskutil` or parsing fails.
pub fn container_of(data_mount: &std::path::Path) -> io::Result<Container> {
    let fs = crate::volume::statfs(data_mount)?;
    let device = fs.mounted_from.trim_start_matches("/dev/").to_owned();
    let mounts = crate::mounts::list().unwrap_or_default();
    let bytes = probe::run(Probe::DiskutilApfsList)?;
    parse_container(&bytes, &device, |dev| {
        mounts
            .iter()
            .find(|m| mounted_from_volume(m.device.trim_start_matches("/dev/"), dev))
            .map(|m| m.mount.clone())
    })
}

/// `true` if a mount's device is the volume itself or a snapshot of it: the
/// sealed system volume `disk3s1` is mounted at `/` as snapshot `disk3s1s1`.
fn mounted_from_volume(mount_dev: &str, volume_dev: &str) -> bool {
    mount_dev == volume_dev
        || mount_dev.strip_prefix(volume_dev).is_some_and(|rest| {
            rest.strip_prefix('s')
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>Containers</key><array>
 <dict><key>ContainerReference</key><string>disk1</string>
  <key>CapacityCeiling</key><integer>524288000</integer><key>CapacityFree</key><integer>504516608</integer>
  <key>Volumes</key><array><dict><key>Name</key><string>xART</string><key>DeviceIdentifier</key><string>disk1s2</string>
  <key>Roles</key><array><string>xART</string></array><key>CapacityInUse</key><integer>6311936</integer></dict></array></dict>
 <dict><key>ContainerReference</key><string>disk3</string>
  <key>CapacityCeiling</key><integer>494384795648</integer><key>CapacityFree</key><integer>31000000000</integer>
  <key>Volumes</key><array>
   <dict><key>Name</key><string>Macintosh HD</string><key>DeviceIdentifier</key><string>disk3s1</string>
    <key>Roles</key><array><string>System</string></array><key>CapacityInUse</key><integer>12577169408</integer></dict>
   <dict><key>Name</key><string>Data</string><key>DeviceIdentifier</key><string>disk3s5</string>
    <key>Roles</key><array><string>Data</string></array><key>CapacityInUse</key><integer>405278142464</integer></dict>
   <dict><key>Name</key><string>VM</string><key>DeviceIdentifier</key><string>disk3s6</string>
    <key>Roles</key><array><string>VM</string></array><key>CapacityInUse</key><integer>33292095488</integer></dict>
   <dict><key>Name</key><string>Odd</string><key>DeviceIdentifier</key><string>disk3s9</string>
    <key>Roles</key><array/><key>CapacityInUse</key><integer>1</integer></dict>
  </array></dict>
</array></dict></plist>"#;

    #[test]
    fn picks_the_container_holding_the_device() {
        let c = parse_container(FIXTURE.as_bytes(), "disk3s5", |d| {
            (d == "disk3s5").then(|| PathBuf::from("/System/Volumes/Data"))
        })
        .unwrap();
        assert_eq!(c.device, "disk3");
        assert_eq!(c.volumes.len(), 4);
        assert_eq!(c.volumes[0].role, VolumeRole::Data);
        assert_eq!(
            c.volumes[0].mount.as_deref(),
            Some(std::path::Path::new("/System/Volumes/Data"))
        );
        assert_eq!(c.volumes[1].role, VolumeRole::Vm);
        assert_eq!(c.volumes[3].role, VolumeRole::Other("none".into()));
        assert_eq!(c.data_volume().unwrap().consumed, 405_278_142_464);
    }

    #[test]
    fn snapshot_mounts_belong_to_their_volume() {
        assert!(mounted_from_volume("disk3s1s1", "disk3s1"));
        assert!(mounted_from_volume("disk3s5", "disk3s5"));
        assert!(!mounted_from_volume("disk3s10", "disk3s1"));
        assert!(!mounted_from_volume("disk3s1", "disk3s10"));
    }

    #[test]
    fn unknown_device_is_an_error() {
        assert!(parse_container(FIXTURE.as_bytes(), "disk9s1", |_| None).is_err());
        assert!(parse_container(b"not a plist", "disk3s5", |_| None).is_err());
    }
}
