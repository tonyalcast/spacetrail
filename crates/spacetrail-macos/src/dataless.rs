//! Cloud placeholder ("dataless") files: detection and a process-wide
//! guarantee that nothing gets downloaded.

use std::fs::Metadata;
use std::io;
use std::os::macos::fs::MetadataExt;

use spacetrail_core::Platform;

/// `SF_DATALESS` from `<sys/stat.h>`: the file's content is not stored locally.
pub const SF_DATALESS: u32 = 0x4000_0000;

// From <sys/resource.h>.
const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: libc::c_int = 3;
const IOPOL_SCOPE_PROCESS: libc::c_int = 0;
const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: libc::c_int = 1;

unsafe extern "C" {
    fn setiopolicy_np(iotype: libc::c_int, scope: libc::c_int, policy: libc::c_int) -> libc::c_int;
}

/// `true` if the metadata carries the `SF_DATALESS` flag.
#[must_use]
pub fn is_dataless(meta: &Metadata) -> bool {
    meta.st_flags() & SF_DATALESS != 0
}

/// Tells the kernel never to materialize dataless files for this process:
/// any access that would download content fails instead. Call once at startup.
///
/// # Errors
/// Returns the OS error if the policy cannot be set.
pub fn prevent_materialization() -> io::Result<()> {
    // SAFETY: plain syscall wrapper taking three integers; no pointers involved.
    let rc = unsafe {
        setiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_PROCESS,
            IOPOL_MATERIALIZE_DATALESS_FILES_OFF,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The macOS implementation of [`Platform`].
#[derive(Debug, Clone, Copy, Default)]
pub struct MacPlatform;

impl Platform for MacPlatform {
    fn is_dataless(&self, meta: &Metadata) -> bool {
        is_dataless(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_temp_file_is_not_dataless() {
        let f = tempfile::NamedTempFile::new().unwrap();
        assert!(!is_dataless(&f.as_file().metadata().unwrap()));
    }

    #[test]
    fn policy_can_be_set() {
        prevent_materialization().unwrap();
    }
}
