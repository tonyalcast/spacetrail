//! Running processes (own user and others, names and paths only).

use std::path::PathBuf;

/// A running process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    /// Process id.
    pub pid: i32,
    /// Executable path, if readable.
    pub path: Option<PathBuf>,
}

impl ProcessInfo {
    /// `true` if the process matches `name`: its executable file name, or an
    /// enclosing app bundle named `<name>.app`. Case-insensitive.
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        let exe = path
            .file_name()
            .is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name));
        let bundle = format!("{name}.app").to_ascii_lowercase();
        exe || path
            .components()
            .any(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase() == bundle)
    }
}

/// Lists running processes. Paths of processes we may not inspect are `None`.
#[must_use]
pub fn list() -> Vec<ProcessInfo> {
    // SAFETY: a null buffer asks for the number of pids only.
    let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    let Ok(n) = usize::try_from(n) else {
        return Vec::new();
    };
    let cap = n + 64;
    let mut pids = vec![0i32; cap];
    let bytes = libc::c_int::try_from(cap * std::mem::size_of::<i32>()).unwrap_or(0);
    // SAFETY: `pids` has room for `cap` pids and `bytes` is exactly its size in bytes.
    let got = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    let Ok(got) = usize::try_from(got) else {
        return Vec::new();
    };
    pids.truncate(got.min(cap));
    pids.into_iter()
        .filter(|&pid| pid > 0)
        .map(|pid| ProcessInfo {
            pid,
            path: pid_path(pid),
        })
        .collect()
}

fn pid_path(pid: i32) -> Option<PathBuf> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let len = u32::try_from(buf.len()).ok()?;
    // SAFETY: `buf` is a writable buffer of `len` bytes; the call writes a
    // NUL-terminated path and returns its length (≤ len) or ≤ 0 on error.
    let rc = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), len) };
    let rc = usize::try_from(rc).ok().filter(|&r| r > 0)?;
    buf.truncate(rc);
    Some(PathBuf::from(String::from_utf8_lossy(&buf).into_owned()))
}

/// Names from `wanted` that match at least one running process.
#[must_use]
pub fn running<'a>(wanted: &'a [String], procs: &[ProcessInfo]) -> Vec<&'a str> {
    wanted
        .iter()
        .filter(|w| procs.iter().any(|p| p.matches(w)))
        .map(String::as_str)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_by_exe_and_bundle() {
        let p = ProcessInfo {
            pid: 1,
            path: Some("/Applications/Cursor.app/Contents/MacOS/Cursor".into()),
        };
        assert!(p.matches("Cursor"));
        assert!(p.matches("cursor"));
        let code = ProcessInfo {
            pid: 2,
            path: Some("/Applications/Visual Studio Code.app/Contents/MacOS/Electron".into()),
        };
        assert!(code.matches("Visual Studio Code"));
        assert!(!code.matches("Code"));
    }

    #[test]
    fn current_process_is_listed() {
        let me = i32::try_from(std::process::id()).unwrap();
        assert!(list().iter().any(|p| p.pid == me));
    }
}
