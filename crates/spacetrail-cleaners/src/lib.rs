//! Declarative cleaners for spacetrail: TOML schema, loader, validator,
//! read-only suggestions, and the single executor that is allowed to remove
//! files (to the Trash, with a journal and undo) (see `CLAUDE.md`, safety rule 2).

#![forbid(unsafe_code)]

pub mod catalog;
pub mod detectors;
pub mod executor;
pub mod manual;
pub mod pattern;
pub mod report;
pub mod schema;
pub mod suggest;

pub use schema::{Cleaner, Risk};
pub use suggest::{Context, Suggestion};

/// Supported cleaner schema version (`schema = 1` in cleaner TOML files).
pub const CLEANER_SCHEMA_VERSION: u32 = 1;
