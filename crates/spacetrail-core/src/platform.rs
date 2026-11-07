//! The seam between platform-agnostic logic and OS-specific behavior.
//!
//! `spacetrail-macos` provides the real implementation; tests use fakes.

use std::fmt::Debug;
use std::fs::Metadata;

/// OS-specific knowledge the scanner needs about a directory entry.
///
/// Implementations must be cheap: they run once per entry, using only the
/// metadata the walker already obtained with `lstat` (no extra syscalls that
/// could touch file contents).
pub trait Platform: Send + Sync + Debug {
    /// `true` if the entry is a cloud placeholder whose content is not stored
    /// locally (macOS: `st_flags & SF_DATALESS`). The scanner never opens such
    /// files and never lists such directories, because that could trigger a download.
    fn is_dataless(&self, meta: &Metadata) -> bool;
}

/// A platform with no special knowledge: nothing is dataless.
#[derive(Debug, Clone, Copy, Default)]
pub struct GenericPlatform;

impl Platform for GenericPlatform {
    fn is_dataless(&self, _meta: &Metadata) -> bool {
        false
    }
}
