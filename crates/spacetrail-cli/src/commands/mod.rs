//! Command implementations.

pub(crate) mod doctor;
pub(crate) mod scan;
pub(crate) mod status;

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use spacetrail_core::paths::tilde;
use spacetrail_macos::Firmlinks;

use crate::render::Style;

/// Context shared by all commands.
#[derive(Debug)]
pub(crate) struct Ctx {
    pub(crate) style: Style,
    pub(crate) json: bool,
    pub(crate) home: Option<PathBuf>,
    pub(crate) firmlinks: Firmlinks,
}

impl Ctx {
    /// User-facing form of a path: firmlinks resolved, home shown as `~`.
    pub(crate) fn display(&self, path: &Path) -> String {
        tilde(&self.firmlinks.to_display(path), self.home.as_deref())
    }

    /// Writes either JSON (when `--json`) or the human text.
    pub(crate) fn emit<T: Serialize>(
        &self,
        value: &T,
        human: impl FnOnce() -> String,
    ) -> anyhow::Result<()> {
        let mut out = std::io::stdout().lock();
        if self.json {
            serde_json::to_writer_pretty(&mut out, value)?;
            writeln!(out)?;
        } else {
            write!(out, "{}", human())?;
        }
        Ok(())
    }
}
