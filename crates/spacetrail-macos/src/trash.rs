//! The Finder Trash, through `NSFileManager`, which returns where the item
//! landed so it can be put back.
//!
//! **Only `spacetrail_cleaners::executor` may call [`move_to_trash`]**: it runs
//! the safety checks first (CLAUDE.md, rule 2). `cargo xtask check-safety`
//! enforces this.

use std::io;
use std::path::{Path, PathBuf};

use objc2::rc::autoreleasepool;
use objc2_foundation::{NSFileManager, NSString, NSURL};

/// Moves `path` to the Trash (a symlink itself is trashed, never its target)
/// and returns the item's new location inside the Trash.
///
/// # Errors
/// The Foundation error (permissions, missing item, volume without a Trash…).
pub fn move_to_trash(path: &Path) -> io::Result<PathBuf> {
    autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let mut resulting = None;
        NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting))
            .map_err(|e| io::Error::other(e.localizedDescription().to_string()))?;
        resulting
            .and_then(|u| u.path())
            .map(|p| PathBuf::from(p.to_string()))
            .ok_or_else(|| io::Error::other("the Trash did not report where the item went"))
    })
}
