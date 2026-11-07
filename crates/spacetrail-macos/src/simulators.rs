//! Installed iOS/watchOS/tvOS/visionOS simulator runtimes (`simctl runtime list -j`).

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use serde::Deserialize;

use crate::probe::{self, Probe};

pub use crate::probe::simctl_path;

/// One installed simulator runtime.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    /// UUID used by `simctl runtime delete`.
    pub identifier: String,
    /// e.g. "18.1".
    #[serde(default)]
    pub version: String,
    /// e.g. "com.apple.platform.iphonesimulator".
    #[serde(default)]
    pub platform_identifier: String,
    /// Bytes on disk.
    #[serde(default)]
    pub size_bytes: u64,
    /// Whether simctl may delete it.
    #[serde(default)]
    pub deletable: bool,
    /// Image location.
    #[serde(default)]
    pub path: PathBuf,
}

impl Runtime {
    /// "iOS 18.1".
    #[must_use]
    pub fn label(&self) -> String {
        let platform = match self.platform_identifier.as_str() {
            p if p.contains("iphone") => "iOS",
            p if p.contains("watch") => "watchOS",
            p if p.contains("appletv") => "tvOS",
            p if p.contains("xr") || p.contains("vision") => "visionOS",
            _ => "Simulator",
        };
        format!("{platform} {}", self.version)
    }
}

/// Parses `simctl runtime list -j`.
#[must_use]
pub fn parse(json: &[u8]) -> Vec<Runtime> {
    serde_json::from_slice::<BTreeMap<String, Runtime>>(json)
        .map(|m| m.into_values().collect())
        .unwrap_or_default()
}

/// Lists installed runtimes.
///
/// # Errors
/// Xcode missing or simctl failure.
pub fn runtimes() -> io::Result<Vec<Runtime>> {
    Ok(parse(&probe::run(Probe::SimctlRuntimeList)?))
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_runtime_list() {
        let json = br#"{"46590DF7":{"build":"22B81","deletable":true,"identifier":"46590DF7","kind":"Cryptex Disk Image","path":"/Library/Developer/CoreSimulator/Cryptex/x.dmg","platformIdentifier":"com.apple.platform.iphonesimulator","sizeBytes":8558900711,"state":"Ready","version":"18.1"}}"#;
        let r = super::parse(json);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].label(), "iOS 18.1");
        assert!(r[0].deletable);
        assert_eq!(r[0].size_bytes, 8_558_900_711);
        assert!(super::parse(b"oops").is_empty());
    }
}
