//! SQLite storage for spacetrail: schema and migrations, scan snapshots,
//! volume samples, rollups and retention, and the cleaning journal.

#![forbid(unsafe_code)]

/// Current schema version, stored in `PRAGMA user_version`.
pub const SCHEMA_VERSION: u32 = 0;

#[cfg(test)]
mod tests {
    #[test]
    fn schema_version_starts_at_zero() {
        assert_eq!(super::SCHEMA_VERSION, 0);
    }
}
