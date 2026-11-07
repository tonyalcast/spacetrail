//! `spacetrail` — monitor, explain and clean disk space on macOS.

#![forbid(unsafe_code)]

use clap::Parser;

/// Monitor, explain and clean disk space on macOS.
#[derive(Debug, Parser)]
#[command(name = "spacetrail", version, about)]
struct Cli {}

fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    eprintln!(
        "spacetrail {}: nothing here yet (Phase 0 scaffold)",
        env!("CARGO_PKG_VERSION")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        super::Cli::command().debug_assert();
    }
}
