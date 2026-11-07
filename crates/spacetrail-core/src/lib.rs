//! Platform-agnostic core of spacetrail: domain model, scanner, size math,
//! diffing and forecasting. OS-specific behavior is reached through the
//! [`Platform`] trait implemented by `spacetrail-macos`; this crate has no `unsafe`.

#![forbid(unsafe_code)]

pub mod platform;
pub mod scan;
pub mod tree;
pub mod units;

pub use platform::{GenericPlatform, Platform};
pub use scan::{ScanError, ScanOptions, ScanProgress, ScanResult, TopFile, Unreadable, scan};
pub use tree::{DirState, DirTree, Node, NodeId, Stats};
pub use units::{HumanBytes, UnitSystem, human, parse_duration, parse_size};

/// Version of the spacetrail core library.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
