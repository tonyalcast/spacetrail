//! Extra file metadata macOS keeps outside the inode: where a downloaded file
//! came from (`kMDItemWhereFroms`), and quarantine origin.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Reads an extended attribute without following symlinks. `None` if absent.
fn xattr(path: &Path, name: &str) -> Option<Vec<u8>> {
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let c_name = CString::new(name).ok()?;
    // SAFETY: valid NUL-terminated strings; a null buffer asks for the size only.
    let size = unsafe {
        libc::getxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            std::ptr::null_mut(),
            0,
            0,
            libc::XATTR_NOFOLLOW,
        )
    };
    let size = usize::try_from(size)
        .ok()
        .filter(|&s| s > 0 && s < 1_000_000)?;
    let mut buf = vec![0u8; size];
    // SAFETY: `buf` has room for `size` bytes, which we pass as its length.
    let got = unsafe {
        libc::getxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            buf.as_mut_ptr().cast(),
            size,
            0,
            libc::XATTR_NOFOLLOW,
        )
    };
    let got = usize::try_from(got).ok()?;
    buf.truncate(got);
    Some(buf)
}

/// URLs a file was downloaded from (browser, Mail, AirDrop…), newest first.
#[must_use]
pub fn where_froms(path: &Path) -> Vec<String> {
    let Some(raw) = xattr(path, "com.apple.metadata:kMDItemWhereFroms") else {
        return Vec::new();
    };
    plist::from_bytes::<Vec<String>>(&raw)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect()
}

/// The app that downloaded a quarantined file (e.g. "Safari", "Google Chrome").
#[must_use]
pub fn quarantine_agent(path: &Path) -> Option<String> {
    let raw = xattr(path, "com.apple.quarantine")?;
    // Format: flags;timestamp;agent name;uuid
    let text = String::from_utf8_lossy(&raw).into_owned();
    text.split(';')
        .nth(2)
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_files_have_no_origin() {
        let f = tempfile::NamedTempFile::new().unwrap();
        assert!(where_froms(f.path()).is_empty());
        assert!(quarantine_agent(f.path()).is_none());
    }
}
