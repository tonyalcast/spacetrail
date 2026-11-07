//! Path patterns: `~`, `${VAR}` / `${VAR:-default}` and globs, plus the
//! validation that keeps patterns away from dangerous locations.

use std::path::{Component, Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};

/// Environment variables a pattern may reference.
pub const ALLOWED_VARS: &[&str] = &[
    "HOME",
    "TMPDIR",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "GRADLE_USER_HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "GOPATH",
    "GOCACHE",
    "GOMODCACHE",
    "NUGET_PACKAGES",
    "OLLAMA_MODELS",
    "HF_HOME",
    "npm_config_cache",
    "PNPM_HOME",
    "BUN_INSTALL",
    "CONDA_PREFIX",
    "PUB_CACHE",
    "UV_CACHE_DIR",
];

/// Locations whose contents a cleaner may never delete.
const DENY_PREFIXES: &[&str] = &[
    "/System",
    "/bin",
    "/sbin",
    "/usr/bin",
    "/usr/sbin",
    "/usr/lib",
    "/usr/libexec",
    "/usr/share",
    "/private/var/db",
    "/Volumes",
    "~/Library/CloudStorage",
    "~/Library/Mobile Documents",
    "~/Library/Keychains",
    "~/Library/Mail",
    "~/Library/Messages",
    "~/Library/Photos",
    "~/Pictures",
    "~/Music",
    "~/Movies",
    "~/Desktop",
    "~/Documents",
];

fn has_glob(s: &str) -> bool {
    s.contains(['*', '?', '[', '{'])
}

/// Replaces variables with their default (or a placeholder) for validation.
fn shape(pattern: &str) -> String {
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &rest[start + 2..start + end];
        match inner.split_once(":-") {
            Some((_, default)) => out.push_str(default),
            None => {
                out.push_str("/__var__/");
                out.push_str(inner);
            }
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

/// Validates a pattern (syntax, variables, depth). Patterns whose matches
/// may be deleted (`deletable`) must also stay out of protected locations;
/// measure-only patterns of `manual` cleaners may point anywhere.
///
/// # Errors
/// A human-readable reason.
pub fn check_pattern(pattern: &str, deletable: bool) -> Result<(), String> {
    for var in pattern.split("${").skip(1) {
        let name = var.split(['}', ':']).next().unwrap_or_default();
        if !ALLOWED_VARS.contains(&name) {
            return Err(format!("variable `{name}` is not in the allow-list"));
        }
    }
    let shaped = shape(pattern);
    let (anchor, rel) = if let Some(r) = shaped.strip_prefix("~/") {
        ("~", r)
    } else if let Some(r) = shaped.strip_prefix('/') {
        ("/", r)
    } else {
        return Err("must start with `~/`, `/` or `${VAR`".into());
    };
    if rel.split('/').any(|c| c == "..") {
        return Err("`..` is not allowed".into());
    }
    let literal: Vec<&str> = rel
        .split('/')
        .filter(|c| !c.is_empty())
        .take_while(|c| !has_glob(c))
        .collect();
    let has_wild = rel.split('/').any(has_glob);
    // `~/Downloads/*.dmg` is fine (the glob has a literal part); `~/Downloads/*` is not.
    let first_glob_is_bare = rel
        .split('/')
        .find(|c| has_glob(c))
        .is_some_and(|c| c.chars().all(|ch| matches!(ch, '*' | '?')));
    if has_wild && (literal.len() < 2 && (literal.is_empty() || first_glob_is_bare)) {
        return Err(
            "a glob needs at least two literal components before it (e.g. `~/Library/Caches/*`)"
                .into(),
        );
    }
    if !has_wild && literal.len() < 2 {
        return Err("path is too close to the root".into());
    }
    let prefix = if anchor == "~" {
        format!("~/{}", literal.join("/"))
    } else {
        format!("/{}", literal.join("/"))
    };
    if !deletable {
        return Ok(());
    }
    for deny in DENY_PREFIXES {
        if prefix == *deny || prefix.starts_with(&format!("{deny}/")) {
            return Err(format!("`{deny}` is protected"));
        }
    }
    Ok(())
}

/// Expands `~` and allowed variables. Returns `None` when a variable without
/// default is unset (the pattern then matches nothing).
#[must_use]
pub fn expand_vars(
    pattern: &str,
    home: &Path,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let tilde = |s: &str| -> String {
        if s == "~" {
            home.display().to_string()
        } else if let Some(r) = s.strip_prefix("~/") {
            home.join(r).display().to_string()
        } else {
            s.to_owned()
        }
    };
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let end = rest[start..].find('}')? + start;
        let inner = &rest[start + 2..end];
        let (name, default) = inner
            .split_once(":-")
            .map_or((inner, None), |(n, d)| (n, Some(d)));
        if !ALLOWED_VARS.contains(&name) {
            return None;
        }
        let value = env(name).filter(|v| !v.is_empty());
        out.push_str(&match (value, default) {
            (Some(v), _) => v,
            (None, Some(d)) => tilde(d),
            (None, None) => return None,
        });
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(tilde(&out))
}

/// Compiles a single-component, case-insensitive glob.
fn component_matcher(glob: &str) -> Option<GlobMatcher> {
    GlobBuilder::new(glob)
        .case_insensitive(true)
        .literal_separator(true)
        .build()
        .ok()
        .map(|g| g.compile_matcher())
}

/// Expands an absolute pattern against the filesystem. The literal prefix
/// (the declared root) is canonicalized once, so system symlinks such as
/// `/var` → `/private/var` are resolved; below it, nothing is ever resolved
/// through a symlink. Results are sorted.
#[must_use]
pub fn glob(pattern: &str) -> Vec<PathBuf> {
    let path = Path::new(pattern);
    if !path.is_absolute() {
        return Vec::new();
    }
    let names: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if names.is_empty() {
        return Vec::new();
    }
    // Prefix: literal components before the first glob; for a fully literal
    // pattern, everything but the leaf (which must not be resolved).
    let split = names
        .iter()
        .position(|n| has_glob(n))
        .unwrap_or(names.len() - 1);
    let prefix: PathBuf = std::iter::once("/".to_owned())
        .chain(names[..split].iter().cloned())
        .collect();
    let Ok(prefix) = prefix.canonicalize() else {
        return Vec::new();
    };
    let mut current = vec![prefix];
    for name in &names[split..] {
        let mut next = Vec::new();
        for dir in &current {
            // Never descend into a symlinked directory.
            let is_real_dir = std::fs::symlink_metadata(dir).is_ok_and(|md| md.is_dir());
            if !is_real_dir {
                continue;
            }
            if has_glob(name) {
                let Some(m) = component_matcher(name) else {
                    return Vec::new();
                };
                let Ok(entries) = std::fs::read_dir(dir) else {
                    continue;
                };
                for e in entries.flatten() {
                    if m.is_match(e.file_name()) {
                        next.push(e.path());
                    }
                }
            } else {
                let p = dir.join(name);
                if std::fs::symlink_metadata(&p).is_ok() {
                    next.push(p);
                }
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    current.sort();
    current
}

/// Exclusion globs: absolute ones match the full path, others the file name.
#[derive(Debug, Default)]
pub struct Excludes {
    full: Vec<GlobMatcher>,
    name: Vec<GlobMatcher>,
}

impl Excludes {
    /// Builds the matchers (patterns already expanded).
    #[must_use]
    pub fn new(patterns: &[String]) -> Self {
        let mut ex = Self::default();
        for p in patterns {
            let full = p.starts_with('/');
            let Ok(g) = GlobBuilder::new(p)
                .case_insensitive(true)
                .literal_separator(full)
                .build()
            else {
                continue;
            };
            if full {
                ex.full.push(g.compile_matcher());
            } else {
                ex.name.push(g.compile_matcher());
            }
        }
        ex
    }

    /// `true` if `path` is excluded.
    #[must_use]
    pub fn matches(&self, path: &Path) -> bool {
        self.full.iter().any(|m| m.is_match(path))
            || path
                .file_name()
                .is_some_and(|n| self.name.iter().any(|m| m.is_match(n)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation() {
        assert!(check_pattern("~/Library/Caches/*", true).is_ok());
        assert!(check_pattern("~/Library/Developer/Xcode/DerivedData/*", true).is_ok());
        assert!(check_pattern("${GOMODCACHE:-~/go/pkg/mod}", true).is_ok());
        assert!(check_pattern("${TMPDIR}DockerDesktopUpdates", true).is_ok());
        assert!(check_pattern("/opt/miniconda3/pkgs", true).is_ok());
        assert!(check_pattern("~/Downloads/*.dmg", true).is_ok());
        assert!(check_pattern("~/Downloads/*", true).is_err());
        assert!(check_pattern("/System/Library/AssetsV2/x", false).is_ok());
        assert!(check_pattern("~/*", false).is_err());
        for bad in [
            "~/*",
            "~/Library/*",
            "/*",
            "~",
            "~/Documents/dev/x",
            "/System/Library/Caches/*",
            "~/Library/CloudStorage/x/*",
            "~/Library/../Documents/x",
            "${PATH}/x/y",
            "relative/path",
        ] {
            assert!(check_pattern(bad, true).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn variable_expansion() {
        let home = Path::new("/Users/t");
        let env = |k: &str| (k == "GOMODCACHE").then(|| "/custom/mod".to_owned());
        assert_eq!(
            expand_vars("${GOMODCACHE:-~/go/pkg/mod}/x", home, &env).as_deref(),
            Some("/custom/mod/x")
        );
        assert_eq!(
            expand_vars("${HF_HOME:-~/.cache/huggingface}/hub", home, &env).as_deref(),
            Some("/Users/t/.cache/huggingface/hub")
        );
        assert_eq!(expand_vars("${OLLAMA_MODELS}", home, &env), None);
        assert_eq!(
            expand_vars("~/Library", home, &env).as_deref(),
            Some("/Users/t/Library")
        );
        assert_eq!(expand_vars("${PATH}", home, &env), None);
    }

    #[test]
    fn glob_expansion_does_not_follow_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        std::fs::create_dir_all(r.join("Caches/a")).unwrap();
        std::fs::create_dir_all(r.join("Caches/B")).unwrap();
        std::fs::write(r.join("Caches/file.log"), b"x").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join("secret")).unwrap();
        std::os::unix::fs::symlink(outside.path(), r.join("Caches/link")).unwrap();

        let all = glob(&format!("{}/Caches/*", r.display()));
        assert_eq!(all.len(), 4);
        let inside_link = glob(&format!("{}/Caches/*/secret", r.display()));
        assert!(inside_link.is_empty(), "must not descend through symlinks");
        let ci = glob(&format!("{}/caches/b", r.display()));
        assert!(ci.is_empty() || ci.len() == 1); // literal components follow FS case rules
        let logs = glob(&format!("{}/Caches/*.LOG", r.display()));
        assert_eq!(logs.len(), 1, "globs are case-insensitive");
    }

    #[test]
    fn excludes() {
        let ex = Excludes::new(&["*.keep".into(), "/x/y/*".into()]);
        assert!(ex.matches(Path::new("/a/b/c.keep")));
        assert!(ex.matches(Path::new("/x/y/z")));
        assert!(!ex.matches(Path::new("/x/y/z/w")));
        assert!(!ex.matches(Path::new("/a/b/c.txt")));
    }
}
