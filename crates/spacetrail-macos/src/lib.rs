//! macOS integration for spacetrail: volume capacity, APFS volume breakdown,
//! firmlinks, dataless detection, Full Disk Access probing, and read-only
//! system probes. All FFI and `unsafe` code of the project lives here.

#[cfg(not(target_os = "macos"))]
compile_error!("spacetrail-macos only builds on macOS");

pub mod apfs;
pub mod dataless;
pub mod docker;
pub mod fda;
pub mod firmlinks;
pub mod hints;
pub mod keychain;
pub mod metadata;
pub mod mounts;
mod probe;
pub mod process;
pub mod system;
pub mod volume;

pub use dataless::{MacPlatform, prevent_materialization};
pub use fda::FdaStatus;
pub use firmlinks::{DATA_ROOT, Firmlinks};

/// The directory to scan for "the whole disk": the data volume when the
/// system uses a split system/data layout, otherwise `/`.
#[must_use]
pub fn whole_disk_root() -> std::path::PathBuf {
    let data = std::path::Path::new(DATA_ROOT);
    if data.is_dir() {
        data.to_owned()
    } else {
        std::path::PathBuf::from("/")
    }
}
