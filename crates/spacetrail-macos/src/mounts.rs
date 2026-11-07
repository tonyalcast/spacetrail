//! Mounted filesystems (`getfsstat(2)`).

use std::io;
use std::path::PathBuf;

/// One mounted filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// Device, e.g. `/dev/disk3s5`.
    pub device: String,
    /// Mount point.
    pub mount: PathBuf,
    /// Filesystem type, e.g. `apfs`.
    pub fs_type: String,
}

/// Lists mounted filesystems without blocking on unresponsive network mounts.
///
/// # Errors
/// Returns the OS error from `getfsstat`.
pub fn list() -> io::Result<Vec<Mount>> {
    // SAFETY: a null buffer asks getfsstat for the number of mounts only.
    let n = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    let Ok(n) = usize::try_from(n) else {
        return Err(io::Error::last_os_error());
    };
    // Leave room for mounts appearing between the two calls.
    let cap = n + 8;
    let mut buf: Vec<libc::statfs> = Vec::with_capacity(cap);
    let bytes = libc::c_int::try_from(cap * std::mem::size_of::<libc::statfs>())
        .map_err(|_| io::Error::other("too many mounts"))?;
    // SAFETY: buf has capacity for `cap` statfs structs and `bytes` is exactly
    // that size; getfsstat writes at most that many and returns the count.
    let got = unsafe { libc::getfsstat(buf.as_mut_ptr(), bytes, libc::MNT_NOWAIT) };
    let Ok(got) = usize::try_from(got) else {
        return Err(io::Error::last_os_error());
    };
    // SAFETY: getfsstat initialized the first `got` (≤ cap) elements.
    unsafe { buf.set_len(got.min(cap)) };
    Ok(buf
        .iter()
        .map(|s| Mount {
            device: crate::volume::c_chars(&s.f_mntfromname),
            mount: PathBuf::from(crate::volume::c_chars(&s.f_mntonname)),
            fs_type: crate::volume::c_chars(&s.f_fstypename),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn root_is_mounted() {
        let mounts = super::list().unwrap();
        assert!(mounts.iter().any(|m| m.mount == std::path::Path::new("/")));
    }
}
