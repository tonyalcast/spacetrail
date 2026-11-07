//! macOS integration for spacetrail: volume capacity, APFS volume breakdown,
//! firmlinks, dataless detection, Full Disk Access probing, FSEvents,
//! per-process I/O, Trash and launchd. All FFI and `unsafe` code lives here.

#[cfg(not(target_os = "macos"))]
compile_error!("spacetrail-macos only builds on macOS");

#[cfg(test)]
mod tests {
    #[test]
    fn builds_on_macos() {
        assert_eq!(std::env::consts::OS, "macos");
    }
}
