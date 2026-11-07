//! Volume capacity: `statfs` plus the Foundation capacity keys that include
//! purgeable space (what Finder shows).

use std::ffi::{CStr, CString};
use std::io;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use objc2::rc::autoreleasepool;
use objc2_foundation::{
    NSNumber, NSString, NSURL, NSURLResourceKey, NSURLVolumeAvailableCapacityForImportantUsageKey,
    NSURLVolumeAvailableCapacityForOpportunisticUsageKey,
};
use spacetrail_core::VolumeStats;

/// Raw `statfs` result for a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatFs {
    /// Total bytes (on APFS: the container size).
    pub total: u64,
    /// Bytes used by this volume (`(f_blocks - f_bfree) * f_bsize`).
    pub used: u64,
    /// Bytes available to unprivileged users.
    pub avail: u64,
    /// Device the volume is mounted from (e.g. `/dev/disk3s5`).
    pub mounted_from: String,
    /// Mount point.
    pub mounted_on: PathBuf,
    /// Filesystem type (e.g. `apfs`).
    pub fs_type: String,
}

pub(crate) fn c_chars(buf: &[libc::c_char]) -> String {
    // SAFETY: statfs fills these fixed-size arrays with NUL-terminated strings;
    // the slice bounds guarantee we never read past the array.
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast(), buf.len()) };
    CStr::from_bytes_until_nul(bytes)
        .map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Calls `statfs(2)` on `path`.
///
/// # Errors
/// Returns the OS error (e.g. `ENOENT`).
pub fn statfs(path: &Path) -> io::Result<StatFs> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let mut buf = MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: c_path is a valid NUL-terminated string and buf is a valid,
    // writable statfs struct; statfs fully initializes it on success.
    let rc = unsafe { libc::statfs(c_path.as_ptr(), buf.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: rc == 0 means the struct was initialized.
    let s = unsafe { buf.assume_init() };
    let bsize = u64::from(s.f_bsize);
    Ok(StatFs {
        total: s.f_blocks * bsize,
        used: s.f_blocks.saturating_sub(s.f_bfree) * bsize,
        avail: s.f_bavail * bsize,
        mounted_from: c_chars(&s.f_mntfromname),
        mounted_on: PathBuf::from(c_chars(&s.f_mntonname)),
        fs_type: c_chars(&s.f_fstypename),
    })
}

fn url_number(url: &NSURL, key: &NSURLResourceKey) -> Option<u64> {
    let mut value = None;
    // SAFETY: for the volume capacity keys, Foundation stores an NSNumber (or
    // nothing) in `value`; we check the type with `downcast` before use.
    unsafe { url.getResourceValue_forKey_error(&mut value, key) }.ok()?;
    let number = value?.downcast::<NSNumber>().ok()?;
    u64::try_from(number.as_i64()).ok()
}

/// Capacity figures for the volume containing `path`.
///
/// # Errors
/// Fails only if `statfs` fails; the Foundation figures are optional.
pub fn volume_stats(path: &Path) -> io::Result<VolumeStats> {
    let fs = statfs(path)?;
    let (important, opportunistic) = autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        // SAFETY: reading immutable Foundation string constants.
        let (k_imp, k_opp) = unsafe {
            (
                NSURLVolumeAvailableCapacityForImportantUsageKey,
                NSURLVolumeAvailableCapacityForOpportunisticUsageKey,
            )
        };
        (url_number(&url, k_imp), url_number(&url, k_opp))
    });
    Ok(VolumeStats {
        total: fs.total,
        used: fs.used,
        free: fs.avail,
        available_important: important,
        available_opportunistic: opportunistic,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statfs_root() {
        let s = statfs(Path::new("/")).unwrap();
        assert!(s.total > 0);
        assert!(s.used <= s.total);
        assert_eq!(s.mounted_on, Path::new("/"));
        assert!(s.mounted_from.starts_with("/dev/"));
    }

    #[test]
    fn volume_stats_has_foundation_figures() {
        let v = volume_stats(Path::new("/")).unwrap();
        assert!(v.available_important.is_some());
        assert!(v.free <= v.total);
    }

    #[test]
    fn statfs_missing_path_errors() {
        assert!(statfs(Path::new("/definitely/not/here")).is_err());
    }
}
