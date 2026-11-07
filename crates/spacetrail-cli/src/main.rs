//! `spacetrail` — monitor, explain and clean disk space on macOS.

#![forbid(unsafe_code)]

mod commands;
mod render;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use spacetrail_core::UnitSystem;

use commands::Ctx;
use render::Style;

/// Monitor, explain and clean disk space on macOS.
#[derive(Debug, Parser)]
#[command(name = "spacetrail", version, about)]
struct Cli {
    /// Machine-readable JSON output.
    #[arg(long, global = true)]
    json: bool,
    /// Show sizes in binary units (GiB) instead of decimal (GB, like Finder).
    #[arg(long, global = true)]
    binary: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Disk capacity and where the used space lives.
    Status,
    /// Scan a folder (default: the whole data volume) and summarize it.
    Scan {
        /// Folder to scan.
        path: Option<PathBuf>,
    },
    /// Largest folders and files under PATH (default: the whole data volume).
    Top {
        /// Folder to scan.
        path: Option<PathBuf>,
        /// Folder tree depth.
        #[arg(long, default_value_t = 3)]
        depth: usize,
        /// Number of largest files to list.
        #[arg(long, default_value_t = 20)]
        files: usize,
        /// Hide folders smaller than this percentage of the total.
        #[arg(long, default_value_t = 1.0)]
        min_pct: f64,
    },
    /// Check permissions (Full Disk Access) and environment.
    Doctor,
}

fn main() -> anyhow::Result<()> {
    // Never download cloud files, whatever happens below.
    let dataless_guard = spacetrail_macos::prevent_materialization();
    let cli = Cli::parse();
    let ctx = Ctx {
        style: Style {
            units: if cli.binary {
                UnitSystem::Binary
            } else {
                UnitSystem::Decimal
            },
        },
        json: cli.json,
        home: std::env::var_os("HOME").map(PathBuf::from),
        firmlinks: spacetrail_macos::Firmlinks::load().unwrap_or_default(),
    };
    match cli.command {
        None | Some(Command::Status) => commands::status::run(&ctx),
        Some(Command::Scan { path }) => commands::scan::run(
            &ctx,
            path,
            commands::scan::View {
                depth: 2,
                min_pct: 2.0,
                files: 10,
            },
        ),
        Some(Command::Top {
            path,
            depth,
            files,
            min_pct,
        }) => commands::scan::run(
            &ctx,
            path,
            commands::scan::View {
                depth,
                min_pct,
                files,
            },
        ),
        Some(Command::Doctor) => commands::doctor::run(&ctx, &dataless_guard),
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        super::Cli::command().debug_assert();
    }
}
