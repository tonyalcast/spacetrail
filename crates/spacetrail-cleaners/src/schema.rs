//! Cleaner TOML schema v1 (see `docs/CLEANER_SPEC.md`) and its validation.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use spacetrail_core::{parse_duration, parse_size};

use crate::pattern;

/// How risky it is to remove what a cleaner targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    /// Regenerates automatically; no user data.
    Safe,
    /// Regenerable but costly, or may hold something wanted.
    Review,
    /// User data or expensive assets: measured and explained only.
    Manual,
}

impl Risk {
    /// Icon shown next to the label (never color alone).
    #[must_use]
    pub fn icon(self) -> &'static str {
        match self {
            Self::Safe => "✅",
            Self::Review => "⚠️",
            Self::Manual => "🔒",
        }
    }

    /// One-line meaning of the class.
    #[must_use]
    pub fn meaning(self) -> &'static str {
        match self {
            Self::Safe => "regenerates automatically, no personal data",
            Self::Review => "regenerable but costly to rebuild, or may hold something you want",
            Self::Manual => "your data or expensive assets: only you can decide",
        }
    }
}

impl fmt::Display for Risk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Safe => "safe",
            Self::Review => "review",
            Self::Manual => "manual",
        })
    }
}

/// A cleaner definition.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cleaner {
    /// Must be 1.
    pub schema: u32,
    /// Unique kebab-case id.
    pub id: String,
    /// SemVer of the rule.
    pub version: String,
    /// Human name.
    pub name: String,
    /// What the data is.
    pub description: String,
    /// Risk class.
    pub risk: Risk,
    /// Why this risk class.
    pub risk_reason: String,
    /// Slash-separated category.
    pub category: String,
    /// What happens after cleaning.
    #[serde(default)]
    pub regenerates: Option<String>,
    /// Authoritative docs URL.
    #[serde(default)]
    pub docs: Option<String>,
    /// Supported platforms (default macOS).
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Lets a user file replace a built-in with the same id.
    #[serde(default, rename = "override")]
    pub override_builtin: bool,
    /// GitHub handles.
    #[serde(default)]
    pub authors: Vec<String>,
    /// When the cleaner applies.
    #[serde(default)]
    pub detect: Detect,
    /// What to size.
    #[serde(default)]
    pub measure: Measure,
    /// Conditions checked before cleaning.
    #[serde(default)]
    pub preconditions: Preconditions,
    /// How to clean, in order.
    #[serde(default, rename = "action")]
    pub actions: Vec<Action>,
    /// Project-artifact mode (markers + artifact dirs under code roots).
    #[serde(default)]
    pub project: Option<ProjectSpec>,
    /// Test fixtures (used by `cleaners test`).
    #[serde(default)]
    pub test: Option<toml::Value>,
}

/// `[detect]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detect {
    /// Applies if any of these exists.
    #[serde(default)]
    pub any_path_exists: Vec<String>,
    /// Applies if any of these commands is installed.
    #[serde(default)]
    pub command_exists: Vec<String>,
    /// Applies if any of these apps is installed.
    #[serde(default)]
    pub app_bundle_id: Vec<String>,
}

/// How measured items are presented.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupBy {
    /// One line per matched item.
    #[default]
    Item,
    /// One aggregated line.
    Cleaner,
}

/// `[measure]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measure {
    /// Path patterns; each match is one item.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Globs excluded from matches.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Hide items smaller than this.
    #[serde(default)]
    pub min_item_bytes: Option<String>,
    /// Presentation.
    #[serde(default)]
    pub group_by: GroupBy,
}

/// `[preconditions]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preconditions {
    /// Apps/processes that must be closed.
    #[serde(default)]
    pub process_not_running: Vec<String>,
    /// Commands that must be installed.
    #[serde(default)]
    pub command_exists: Vec<String>,
}

/// Which timestamp decides an item's age.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgeBasis {
    /// The item's own mtime.
    Mtime,
    /// The newest mtime of anything inside (default for directories).
    NewestMtime,
}

/// Deletion method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    /// Move to Trash (default).
    Trash,
    /// Delete permanently (opt-in).
    Permanent,
}

/// Where a command's size estimate comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Estimate {
    /// Use the `[measure]` total (an upper bound).
    Measure,
    /// No estimate.
    None,
}

/// File-removal parameters shared by `delete_contents` and `trash`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveSpec {
    /// Patterns (default: `[measure].paths`).
    #[serde(default)]
    pub paths: Vec<String>,
    /// Only items older than this duration.
    #[serde(default)]
    pub older_than: Option<String>,
    /// Timestamp used for the age filter.
    #[serde(default)]
    pub age_basis: Option<AgeBasis>,
    /// Excluded globs.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Trash (default) or permanent.
    #[serde(default)]
    pub method: Option<Method>,
}

/// `[[action]]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// Remove the matched items.
    DeleteContents(RemoveSpec),
    /// Move the matched items to Trash.
    Trash(RemoveSpec),
    /// Run the tool's own cleanup.
    Command {
        /// argv, never a shell string.
        argv: Vec<String>,
        /// Max run time.
        #[serde(default)]
        timeout: Option<String>,
        /// Exit codes treated as success.
        #[serde(default)]
        ok_exit_codes: Vec<i32>,
        /// Size estimate source.
        #[serde(default)]
        estimate: Option<Estimate>,
        /// Extra env vars.
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    /// Instructions only.
    Manual {
        /// Markdown steps.
        instructions: String,
        /// Copyable commands, never executed.
        #[serde(default)]
        suggested_commands: Vec<Vec<String>>,
    },
}

/// Which timestamp decides a project's staleness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaleBasis {
    /// Last commit date (falls back to source mtime without git).
    GitLastCommit,
    /// Newest mtime outside the artifact dirs.
    NewestSourceMtime,
}

/// `[project]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSpec {
    /// Files that identify a project root (globs allowed, e.g. `*.csproj`).
    pub markers: Vec<String>,
    /// Removable artifact directory names.
    pub artifacts: Vec<String>,
    /// Staleness threshold (default 60d).
    #[serde(default)]
    pub stale_after: Option<String>,
    /// Staleness basis.
    #[serde(default)]
    pub stale_basis: Option<StaleBasis>,
    /// Search depth below each code root.
    #[serde(default)]
    pub max_depth: Option<usize>,
    /// Extra directories never descended into.
    #[serde(default)]
    pub skip_dirs: Vec<String>,
}

/// A validation problem in a cleaner file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cleaner `{id}`: {message}")]
pub struct ValidationError {
    /// Cleaner id (or file name when the id is missing).
    pub id: String,
    /// What is wrong.
    pub message: String,
}

/// Programs that would turn an argv into an arbitrary script.
const SHELLS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "csh",
    "tcsh",
    "env",
    "osascript",
    "perl",
    "ruby",
    "node",
    "deno",
    "eval",
    "xargs",
    "sudo",
];

impl Cleaner {
    /// Parses a TOML document.
    ///
    /// # Errors
    /// Returns the TOML/serde error message (unknown keys are errors).
    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Checks every rule of the spec that does not need the filesystem.
    ///
    /// # Errors
    /// Returns all problems found.
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errs = Vec::new();
        let mut err = |m: String| {
            errs.push(ValidationError {
                id: self.id.clone(),
                message: m,
            });
        };
        if self.schema != 1 {
            err(format!("unsupported schema {}", self.schema));
        }
        let id_ok = (3..=64).contains(&self.id.len())
            && self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !id_ok {
            err("id must be kebab-case [a-z0-9-]{3,64}".into());
        }
        if self.risk != Risk::Manual && self.regenerates.is_none() && self.project.is_none() {
            err("`regenerates` is required for safe and review cleaners".into());
        }
        if self.actions.is_empty() && self.project.is_none() {
            err("at least one [[action]] is required".into());
        }
        if self.project.is_none() && self.measure.paths.is_empty() {
            err("[measure].paths is required (or a [project] section)".into());
        }
        if let Some(m) = &self.measure.min_item_bytes {
            if parse_size(m).is_err() {
                err(format!("invalid min_item_bytes `{m}`"));
            }
        }
        let mut patterns: Vec<&String> = self.measure.paths.iter().collect();
        patterns.extend(&self.detect.any_path_exists);
        for a in &self.actions {
            match a {
                Action::DeleteContents(r) | Action::Trash(r) => {
                    if self.risk == Risk::Manual {
                        err("manual cleaners may only have `manual` actions".into());
                    }
                    patterns.extend(&r.paths);
                    if let Some(d) = &r.older_than {
                        if parse_duration(d).is_err() {
                            err(format!("invalid older_than `{d}`"));
                        }
                    }
                }
                Action::Command { argv, timeout, .. } => {
                    if self.risk == Risk::Manual {
                        err("manual cleaners may only have `manual` actions".into());
                    }
                    match argv.first() {
                        None => err("command argv is empty".into()),
                        Some(prog) => {
                            let base = prog.rsplit('/').next().unwrap_or(prog);
                            if SHELLS.contains(&base) || base.starts_with("python") {
                                err(format!(
                                    "command `{base}` would run arbitrary code; not allowed"
                                ));
                            }
                        }
                    }
                    if let Some(t) = timeout {
                        if parse_duration(t).is_err() {
                            err(format!("invalid timeout `{t}`"));
                        }
                    }
                }
                Action::Manual { .. } => {}
            }
        }
        let deletable = self.risk != Risk::Manual;
        for p in patterns {
            if let Err(m) = pattern::check_pattern(p, deletable) {
                err(format!("path `{p}`: {m}"));
            }
        }
        if let Some(p) = &self.project {
            if p.markers.is_empty() || p.artifacts.is_empty() {
                err("[project] needs markers and artifacts".into());
            }
            if let Some(d) = &p.stale_after {
                if parse_duration(d).is_err() {
                    err(format!("invalid stale_after `{d}`"));
                }
            }
            for a in &p.artifacts {
                if a.contains('/') || a == "." || a == ".." || a.is_empty() {
                    err(format!("artifact `{a}` must be a plain directory name"));
                }
            }
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    /// Commands a user could run to clean this up (from `command` and
    /// `manual` actions).
    #[must_use]
    pub fn commands(&self) -> Vec<Vec<String>> {
        self.actions
            .iter()
            .flat_map(|a| match a {
                Action::Command { argv, .. } => vec![argv.clone()],
                Action::Manual {
                    suggested_commands, ..
                } => suggested_commands.clone(),
                _ => Vec::new(),
            })
            .collect()
    }

    /// Instructions from `manual` actions.
    #[must_use]
    pub fn instructions(&self) -> Option<String> {
        let parts: Vec<&str> = self
            .actions
            .iter()
            .filter_map(|a| match a {
                Action::Manual { instructions, .. } => Some(instructions.trim()),
                _ => None,
            })
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }

    /// The removal spec, if the cleaner deletes files.
    #[must_use]
    pub fn remove_spec(&self) -> Option<(&RemoveSpec, bool)> {
        self.actions.iter().find_map(|a| match a {
            Action::DeleteContents(r) => Some((r, false)),
            Action::Trash(r) => Some((r, true)),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
schema = 1
id = "xcode-derived-data"
version = "1.0.0"
name = "Xcode DerivedData"
description = "Build intermediates."
risk = "safe"
risk_reason = "Only build outputs."
category = "developer/xcode"
regenerates = "On next build."
[measure]
paths = ["~/Library/Developer/Xcode/DerivedData/*"]
[preconditions]
process_not_running = ["Xcode"]
[[action]]
type = "delete_contents"
older_than = "0d"
"#;

    #[test]
    fn parses_and_validates() {
        let c = Cleaner::from_toml(GOOD).unwrap();
        assert_eq!(c.risk, Risk::Safe);
        c.validate().unwrap();
        assert!(c.remove_spec().is_some());
    }

    #[test]
    fn unknown_keys_are_errors() {
        let bad = GOOD.replace("category =", "categroy =");
        assert!(Cleaner::from_toml(&bad).is_err());
    }

    fn errors(toml: &str) -> Vec<String> {
        Cleaner::from_toml(toml)
            .unwrap()
            .validate()
            .unwrap_err()
            .into_iter()
            .map(|e| e.message)
            .collect()
    }

    #[test]
    fn manual_cleaners_cannot_delete() {
        let bad = GOOD.replace("risk = \"safe\"", "risk = \"manual\"");
        assert!(errors(&bad).iter().any(|e| e.contains("manual cleaners")));
    }

    #[test]
    fn shells_are_rejected() {
        let bad = GOOD.replace(
            "type = \"delete_contents\"\nolder_than = \"0d\"",
            "type = \"command\"\nargv = [\"/bin/sh\", \"-c\", \"rm -rf ~\"]",
        );
        assert!(errors(&bad).iter().any(|e| e.contains("arbitrary code")));
    }

    #[test]
    fn dangerous_paths_are_rejected() {
        for p in [
            "~/*",
            "~/Documents/*",
            "/*",
            "~/Library/*",
            "/System/Library/*",
        ] {
            let bad = GOOD.replace("~/Library/Developer/Xcode/DerivedData/*", p);
            assert!(!errors(&bad).is_empty(), "{p} should be rejected");
        }
    }
}
