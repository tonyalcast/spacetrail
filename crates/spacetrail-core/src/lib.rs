//! Platform-agnostic core of spacetrail: domain model, scanner, size math,
//! diffing and forecasting. OS-specific behavior is reached through a
//! platform trait implemented by `spacetrail-macos`; this crate has no `unsafe`.

#![forbid(unsafe_code)]

/// Version of the spacetrail core library.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}
