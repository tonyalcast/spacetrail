//! Built-in and user cleaners.

use std::path::Path;

use crate::schema::{Cleaner, ValidationError};
use crate::suggest::Source;

include!(concat!(env!("OUT_DIR"), "/builtin.rs"));

/// A loaded catalog plus problems found while loading it.
#[derive(Debug, Default)]
pub struct Catalog {
    /// Valid cleaners with their origin.
    pub cleaners: Vec<(Cleaner, Source)>,
    /// Files that failed to parse or validate (never fatal).
    pub problems: Vec<String>,
}

/// Number of built-in cleaners.
#[must_use]
pub fn builtin_count() -> usize {
    BUILTIN.len()
}

fn load_one(name: &str, text: &str, source: Source, cat: &mut Catalog) {
    match Cleaner::from_toml(text) {
        Err(e) => cat.problems.push(format!("{name}: {e}")),
        Ok(c) => match c.validate() {
            Err(errs) => cat.problems.extend(
                errs.iter()
                    .map(|ValidationError { id, message }| format!("{name} ({id}): {message}")),
            ),
            Ok(()) => cat.cleaners.push((c, source)),
        },
    }
}

/// Loads built-ins, then user cleaners from `user_dir` (if it exists). A user
/// cleaner replaces a built-in with the same id only if it sets `override = true`.
#[must_use]
pub fn load(user_dir: Option<&Path>) -> Catalog {
    let mut cat = Catalog::default();
    for (name, text) in BUILTIN {
        load_one(name, text, Source::Builtin, &mut cat);
    }
    let Some(dir) = user_dir else { return cat };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return cat;
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    for f in files {
        let name = f
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Ok(text) = std::fs::read_to_string(&f) else {
            cat.problems.push(format!("{name}: unreadable"));
            continue;
        };
        let mut user = Catalog::default();
        load_one(&name, &text, Source::User, &mut user);
        cat.problems.extend(user.problems);
        for (c, s) in user.cleaners {
            match cat.cleaners.iter().position(|(b, _)| b.id == c.id) {
                Some(i) if c.override_builtin => cat.cleaners[i] = (c, s),
                Some(_) => cat.problems.push(format!(
                    "{name}: id `{}` already exists (set `override = true` to replace it)",
                    c.id
                )),
                None => cat.cleaners.push((c, s)),
            }
        }
    }
    cat
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_parses_and_validates() {
        let cat = load(None);
        assert!(cat.problems.is_empty(), "{:#?}", cat.problems);
        assert_eq!(cat.cleaners.len(), BUILTIN.len());
        assert!(cat.cleaners.len() >= 40);
    }

    #[test]
    fn builtin_ids_are_unique_and_match_file_names() {
        let cat = load(None);
        let mut ids: Vec<_> = cat.cleaners.iter().map(|(c, _)| c.id.clone()).collect();
        for (name, _) in BUILTIN {
            assert!(
                ids.contains(&name.trim_end_matches(".toml").to_owned()),
                "{name}"
            );
        }
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), BUILTIN.len());
    }

    #[test]
    fn user_cleaners_need_override_to_replace() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, yarn) = BUILTIN
            .iter()
            .find(|(n, _)| *n == "yarn-cache.toml")
            .unwrap();
        std::fs::write(tmp.path().join("dup.toml"), yarn).unwrap();
        std::fs::write(tmp.path().join("broken.toml"), "schema = 1\nid = 3").unwrap();
        let cat = load(Some(tmp.path()));
        assert_eq!(cat.problems.len(), 2, "{:#?}", cat.problems);
        let with_override = yarn.replace(
            "version = \"1.0.0\"",
            "version = \"1.0.0\"\noverride = true",
        );
        let tmp2 = tempfile::tempdir().unwrap();
        std::fs::write(tmp2.path().join("dup.toml"), with_override).unwrap();
        let cat = load(Some(tmp2.path()));
        assert!(cat.problems.is_empty(), "{:#?}", cat.problems);
        assert!(
            cat.cleaners
                .iter()
                .any(|(c, s)| c.id == "yarn-cache" && *s == Source::User)
        );
    }
}
