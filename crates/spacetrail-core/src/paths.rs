//! Where spacetrail keeps its configuration and data.
//!
//! `SPACETRAIL_HOME` overrides everything (tests and sandboxes); otherwise
//! config follows XDG (`~/.config/spacetrail`) and data lives in
//! `~/Library/Application Support/spacetrail` on macOS.

use std::path::{Path, PathBuf};

/// Resolved spacetrail directories.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Dirs {
    /// `config.toml` and user cleaners live here.
    pub config: PathBuf,
    /// Database, socket and logs live here.
    pub data: PathBuf,
    /// `true` when `SPACETRAIL_HOME` is in effect.
    pub sandboxed: bool,
}

impl Dirs {
    /// Resolves directories from an environment lookup function.
    #[must_use]
    pub fn resolve(env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let non_empty = |k: &str| env(k).filter(|v| !v.is_empty());
        if let Some(root) = non_empty("SPACETRAIL_HOME") {
            let root = PathBuf::from(root);
            return Some(Self {
                config: root.join("config"),
                data: root.join("data"),
                sandboxed: true,
            });
        }
        let home = PathBuf::from(non_empty("HOME")?);
        let config_base =
            non_empty("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from);
        Some(Self {
            config: config_base.join("spacetrail"),
            data: home.join("Library/Application Support/spacetrail"),
            sandboxed: false,
        })
    }

    /// Resolves directories from the process environment.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        Self::resolve(|k| std::env::var(k).ok())
    }

    /// Path of the SQLite database.
    #[must_use]
    pub fn database(&self) -> PathBuf {
        self.data.join("spacetrail.db")
    }

    /// Path of `config.toml`.
    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    /// Directory of user cleaners.
    #[must_use]
    pub fn user_cleaners(&self) -> PathBuf {
        self.config.join("cleaners")
    }
}

/// Replaces a leading home directory with `~` for display.
#[must_use]
pub fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn spacetrail_home_wins() {
        let d = Dirs::resolve(env(&[("SPACETRAIL_HOME", "/sb"), ("HOME", "/Users/x")])).unwrap();
        assert_eq!(d.config, Path::new("/sb/config"));
        assert_eq!(d.database(), Path::new("/sb/data/spacetrail.db"));
        assert!(d.sandboxed);
    }

    #[test]
    fn xdg_and_defaults() {
        let d = Dirs::resolve(env(&[("HOME", "/Users/x")])).unwrap();
        assert_eq!(
            d.config_file(),
            Path::new("/Users/x/.config/spacetrail/config.toml")
        );
        assert_eq!(
            d.data,
            Path::new("/Users/x/Library/Application Support/spacetrail")
        );
        let d = Dirs::resolve(env(&[("HOME", "/Users/x"), ("XDG_CONFIG_HOME", "/cfg")])).unwrap();
        assert_eq!(d.user_cleaners(), Path::new("/cfg/spacetrail/cleaners"));
        assert!(Dirs::resolve(env(&[])).is_none());
    }

    #[test]
    fn tilde_display() {
        let h = Path::new("/Users/x");
        assert_eq!(tilde(Path::new("/Users/x/Library"), Some(h)), "~/Library");
        assert_eq!(tilde(Path::new("/Users/x"), Some(h)), "~");
        assert_eq!(tilde(Path::new("/opt"), Some(h)), "/opt");
    }
}
