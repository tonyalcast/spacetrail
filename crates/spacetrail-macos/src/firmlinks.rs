//! APFS firmlinks: the sealed system volume exposes parts of the data volume
//! at familiar paths (`/Users` is really `/System/Volumes/Data/Users`).
//! spacetrail scans the data volume once and shows the familiar paths.

use std::io;
use std::path::{Path, PathBuf};

/// Mount point of the data volume on macOS 10.15+.
pub const DATA_ROOT: &str = "/System/Volumes/Data";

/// Location of the firmlink table.
pub const FIRMLINKS_FILE: &str = "/usr/share/firmlinks";

/// Mapping between visible paths and their location on the data volume.
#[derive(Debug, Clone, Default)]
pub struct Firmlinks {
    /// (visible absolute path, path relative to the data volume), longest
    /// relative path first so the most specific entry wins.
    entries: Vec<(PathBuf, PathBuf)>,
}

impl Firmlinks {
    /// Loads the system table.
    ///
    /// # Errors
    /// Returns the I/O error if the file cannot be read.
    pub fn load() -> io::Result<Self> {
        Ok(Self::parse(&std::fs::read_to_string(FIRMLINKS_FILE)?))
    }

    /// Parses lines of `<visible path>\t<data-relative path>`.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut entries: Vec<(PathBuf, PathBuf)> = text
            .lines()
            .filter_map(|l| {
                let (visible, rel) = l.split_once('\t')?;
                let (visible, rel) = (visible.trim(), rel.trim());
                (visible.starts_with('/') && !rel.is_empty())
                    .then(|| (PathBuf::from(visible), PathBuf::from(rel)))
            })
            .collect();
        entries.sort_by_key(|(_, rel)| std::cmp::Reverse(rel.components().count()));
        Self { entries }
    }

    /// Converts a path on the data volume to the path users know:
    /// `/System/Volumes/Data/Users/x` → `/Users/x`. Other paths are returned unchanged.
    #[must_use]
    pub fn to_display(&self, path: &Path) -> PathBuf {
        let Ok(rel) = path.strip_prefix(DATA_ROOT) else {
            return path.to_owned();
        };
        for (visible, data_rel) in &self.entries {
            if let Ok(rest) = rel.strip_prefix(data_rel) {
                return if rest.as_os_str().is_empty() {
                    visible.clone()
                } else {
                    visible.join(rest)
                };
            }
        }
        path.to_owned()
    }

    /// Converts a visible path to its location on the data volume, if it is
    /// behind a firmlink: `/Users/x` → `/System/Volumes/Data/Users/x`.
    #[must_use]
    pub fn to_data(&self, path: &Path) -> Option<PathBuf> {
        let mut best: Option<(usize, PathBuf)> = None;
        for (visible, data_rel) in &self.entries {
            if let Ok(rest) = path.strip_prefix(visible) {
                let depth = visible.components().count();
                if best.as_ref().is_none_or(|(d, _)| depth > *d) {
                    best = Some((depth, Path::new(DATA_ROOT).join(data_rel).join(rest)));
                }
            }
        }
        best.map(|(_, p)| {
            // `join("")` adds a trailing separator; normalize it away.
            p.components().collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "/Applications\tApplications\n/Users\tUsers\n/usr/local\tusr/local\n/System/Library/Caches\tSystem/Library/Caches\nbogus line\n";

    #[test]
    fn maps_data_paths_to_display_paths() {
        let f = Firmlinks::parse(TABLE);
        let d = |p: &str| f.to_display(Path::new(p));
        assert_eq!(
            d("/System/Volumes/Data/Users/al/x"),
            Path::new("/Users/al/x")
        );
        assert_eq!(d("/System/Volumes/Data/Users"), Path::new("/Users"));
        assert_eq!(
            d("/System/Volumes/Data/usr/local/bin"),
            Path::new("/usr/local/bin")
        );
        assert_eq!(
            d("/System/Volumes/Data/System/Library/Caches/c"),
            Path::new("/System/Library/Caches/c")
        );
        // Not behind a firmlink: unchanged.
        assert_eq!(
            d("/System/Volumes/Data/.Spotlight-V100"),
            Path::new("/System/Volumes/Data/.Spotlight-V100")
        );
        assert_eq!(d("/tmp/x"), Path::new("/tmp/x"));
    }

    #[test]
    fn maps_visible_paths_to_data_paths() {
        let f = Firmlinks::parse(TABLE);
        assert_eq!(
            f.to_data(Path::new("/Users/al")),
            Some(PathBuf::from("/System/Volumes/Data/Users/al"))
        );
        assert_eq!(
            f.to_data(Path::new("/Users")),
            Some(PathBuf::from("/System/Volumes/Data/Users"))
        );
        assert_eq!(f.to_data(Path::new("/usr/bin")), None);
    }

    #[test]
    fn system_table_loads() {
        let f = Firmlinks::load().unwrap();
        assert_eq!(
            f.to_display(Path::new("/System/Volumes/Data/Users")),
            Path::new("/Users")
        );
    }
}
