//! Docker disk usage via `docker system df` (read-only).

use std::io;

use serde::Deserialize;
use spacetrail_core::parse_size;

use crate::probe::{self, Probe};

/// One row of `docker system df`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerUsage {
    /// "Images", "Containers", "Local Volumes" or "Build Cache".
    pub kind: String,
    /// Total size.
    pub size: u64,
    /// Space Docker itself reports as reclaimable.
    pub reclaimable: u64,
    /// Number of objects.
    pub total: u64,
    /// Objects in use.
    pub active: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Row {
    #[serde(rename = "Type")]
    kind: String,
    #[serde(default)]
    total_count: String,
    #[serde(default)]
    active: String,
    #[serde(default)]
    size: String,
    #[serde(default)]
    reclaimable: String,
}

/// Parses the `{{json .}}` lines of `docker system df`.
#[must_use]
pub fn parse(text: &str) -> Vec<DockerUsage> {
    let size = |s: &str| parse_size(s.split_whitespace().next().unwrap_or("0")).unwrap_or(0);
    text.lines()
        .filter_map(|l| serde_json::from_str::<Row>(l).ok())
        .map(|r| DockerUsage {
            size: size(&r.size),
            reclaimable: size(&r.reclaimable),
            total: r.total_count.parse().unwrap_or(0),
            active: r.active.parse().unwrap_or(0),
            kind: r.kind,
        })
        .collect()
}

/// Asks the Docker CLI for disk usage. Fails if Docker is not installed or
/// its engine is not running.
///
/// # Errors
/// Probe failure (not installed, engine stopped, timeout).
pub fn system_df() -> io::Result<Vec<DockerUsage>> {
    let out = probe::run(Probe::DockerSystemDf)?;
    Ok(parse(&String::from_utf8_lossy(&out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_docker_output() {
        let out = r#"{"Active":"12","Reclaimable":"5.401GB (18%)","Size":"29.18GB","TotalCount":"20","Type":"Images"}
{"Active":"0","Reclaimable":"9.889GB","Size":"15.84GB","TotalCount":"151","Type":"Build Cache"}
{"Active":"4","Reclaimable":"550.4MB (59%)","Size":"928.2MB","TotalCount":"13","Type":"Local Volumes"}
garbage"#;
        let rows = parse(out);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].reclaimable, 5_401_000_000);
        assert_eq!(rows[1].kind, "Build Cache");
        assert_eq!(rows[1].size, 15_840_000_000);
        assert_eq!(rows[2].active, 4);
    }
}
