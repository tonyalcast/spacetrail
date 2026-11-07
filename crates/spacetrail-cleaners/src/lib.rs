//! Declarative cleaners for spacetrail: TOML schema, loader, validator,
//! planner, and the single executor that is allowed to delete files or
//! spawn processes (see `CLAUDE.md`, safety rule 2).

#![forbid(unsafe_code)]

/// Supported cleaner schema version (`schema = 1` in cleaner TOML files).
pub const CLEANER_SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    #[test]
    fn schema_version() {
        assert_eq!(super::CLEANER_SCHEMA_VERSION, 1);
    }
}
