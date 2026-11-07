//! The metadata sent to the LLM about one file or folder. Never file contents;
//! the home folder is replaced by `~` so the user name is not sent.

use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use spacetrail_macos::{dataless, metadata};

/// Context the UI already knows about the item.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct Hint {
    /// Allocated bytes measured by the scan.
    pub(crate) bytes: Option<u64>,
    /// What spacetrail itself thinks it is (e.g. "Yarn cache (safe)").
    pub(crate) spacetrail_note: Option<String>,
}

/// Exactly what is sent; also shown to the user.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Facts {
    path: String,
    kind: &'static str,
    size: Option<String>,
    modified: Option<String>,
    created: Option<String>,
    owned_by_root: bool,
    inside_app_bundle: Option<String>,
    downloaded_from: Vec<String>,
    downloaded_by_app: Option<String>,
    cloud_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    items_inside: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sample_of_items_inside: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spacetrail_classification: Option<String>,
}

fn date(t: std::io::Result<SystemTime>) -> Option<String> {
    let secs = t
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let days = i64::try_from(secs / 86_400).ok()?;
    // Civil-from-days (Howard Hinnant), enough for a YYYY-MM-DD date.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

pub(crate) fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

pub(crate) fn gather(path: &Path, home: &Path, hint: &Hint, share_listing: bool) -> Facts {
    let meta = std::fs::symlink_metadata(path);
    let kind = match &meta {
        Ok(m) if m.file_type().is_symlink() => "symlink",
        Ok(m) if m.is_dir() => "folder",
        Ok(_) => "file",
        Err(_) => "missing",
    };
    let cloud_only = meta.as_ref().is_ok_and(dataless::is_dataless);
    let bytes = hint.bytes.or_else(|| {
        meta.as_ref()
            .ok()
            .filter(|m| m.is_file())
            .map(|m| m.blocks() * 512)
    });
    let inside_app_bundle = path
        .ancestors()
        .find(|a| a.extension().is_some_and(|e| e == "app"))
        .filter(|a| *a != path)
        .and_then(|a| a.file_name())
        .map(|n| n.to_string_lossy().into_owned());
    let (mut items_inside, mut sample) = (None, Vec::new());
    if kind == "folder"
        && share_listing
        && !cloud_only
        && let Ok(rd) = std::fs::read_dir(path)
    {
        let mut names: Vec<String> = rd
            .flatten()
            .map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    format!("{n}/")
                } else {
                    n
                }
            })
            .collect();
        names.sort();
        items_inside = Some(names.len());
        names.truncate(30);
        sample = names;
    }
    let downloaded = if kind == "file" && !cloud_only {
        (
            metadata::where_froms(path),
            metadata::quarantine_agent(path),
        )
    } else {
        (Vec::new(), None)
    };
    Facts {
        path: tilde(path, home),
        kind,
        size: bytes.map(|b| spacetrail_core::human(b).to_string()),
        modified: meta.as_ref().ok().and_then(|m| date(m.modified())),
        created: meta.as_ref().ok().and_then(|m| date(m.created())),
        owned_by_root: meta.as_ref().is_ok_and(|m| m.uid() == 0),
        inside_app_bundle,
        downloaded_from: downloaded.0,
        downloaded_by_app: downloaded.1,
        cloud_only,
        items_inside,
        sample_of_items_inside: sample,
        spacetrail_classification: hint.spacetrail_note.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_tilde() {
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_793_923_200);
        assert_eq!(date(Ok(t)).as_deref(), Some("2026-11-06"));
        assert_eq!(
            tilde(Path::new("/Users/a/Library"), Path::new("/Users/a")),
            "~/Library"
        );
        assert_eq!(tilde(Path::new("/opt/x"), Path::new("/Users/a")), "/opt/x");
    }

    #[test]
    fn folder_facts_list_names_not_contents() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("secret.txt"), b"TOP SECRET").unwrap();
        let f = gather(
            tmp.path(),
            Path::new("/nonexistent-home"),
            &Hint::default(),
            true,
        );
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("secret.txt") && json.contains("sub/"));
        assert!(!json.contains("TOP SECRET"));
        let f = gather(
            tmp.path(),
            Path::new("/nonexistent-home"),
            &Hint::default(),
            false,
        );
        assert!(!serde_json::to_string(&f).unwrap().contains("secret.txt"));
    }
}
