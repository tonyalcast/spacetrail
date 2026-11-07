//! The full suggestion pass used by the CLI and the desktop app: disk
//! snapshot, detectors, catalog rules, totals and advice.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use spacetrail_core::{VolumeRole, human};
use spacetrail_macos::FdaStatus;
use spacetrail_macos::system::{DiskStatus, disk_status};

use crate::catalog;
use crate::detectors::{self, DEFAULT_CODE_ROOTS};
use crate::schema::Risk;
use crate::suggest::{self, Claims, Context, Suggestion};

/// What to compute.
#[derive(Debug, Clone, Default)]
pub struct SuggestOptions {
    /// Only these risk classes (all when empty).
    pub risks: Vec<Risk>,
    /// Hide suggestions smaller than this.
    pub min_bytes: u64,
    /// Code roots for project detection (defaults when empty).
    pub roots: Vec<PathBuf>,
}

/// Bytes that could be freed per risk class.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Totals {
    /// Safe suggestions.
    pub safe: u64,
    /// Review suggestions.
    pub review: u64,
    /// Manual suggestions.
    pub manual: u64,
}

/// Everything `suggest` reports.
#[derive(Debug, Clone, Serialize)]
pub struct SuggestReport {
    /// Disk snapshot.
    pub disk: DiskStatus,
    /// Totals per risk.
    pub totals: Totals,
    /// Suggestions, safe first, largest first.
    pub suggestions: Vec<Suggestion>,
    /// Non-deletable hints (swap, Trash, log hogs…).
    pub advice: Vec<String>,
    /// Cleaner files that were ignored.
    pub catalog_problems: Vec<String>,
}

/// Runs every detector and rule. `progress` receives short status messages.
///
/// # Errors
/// Fails only if the disk snapshot cannot be taken.
pub fn run(
    opts: &SuggestOptions,
    home: &Path,
    user_cleaners: Option<&Path>,
    progress: &dyn Fn(&str),
) -> io::Result<SuggestReport> {
    progress("Reading disk capacity");
    let disk = disk_status(Some(home))?;
    let sys = Context::from_system(home.to_owned());
    let cat = catalog::load(user_cleaners);
    let mut claims = Claims::default();
    let mut out: Vec<Suggestion> = Vec::new();

    progress("Asking Docker what it can free");
    out.extend(detectors::docker(&sys, &mut claims));

    progress("Looking for worktrees and stale project artifacts");
    let roots: Vec<PathBuf> = if opts.roots.is_empty() {
        DEFAULT_CODE_ROOTS
            .iter()
            .filter_map(|r| sys.expand(r))
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .collect()
    } else {
        opts.roots.clone()
    };
    out.extend(detectors::projects(
        &sys,
        &cat.cleaners,
        &roots,
        &mut claims,
    ));

    progress("Checking editors");
    out.extend(detectors::orphaned_workspaces(&sys, &mut claims));
    out.extend(detectors::agent_versions(&sys, &mut claims));

    progress("Measuring caches, logs and tool data");
    let mut rules: Vec<_> = cat
        .cleaners
        .iter()
        .filter(|(c, _)| c.project.is_none())
        .cloned()
        .collect();
    suggest::order_by_specificity(&mut rules);
    for (c, src) in &rules {
        if let Some(s) = suggest::evaluate(c, *src, &sys, &mut claims, 1) {
            out.push(s);
        }
    }

    progress("Looking at large app data");
    out.extend(detectors::large_app_data(
        &sys,
        &claims,
        2_000_000_000.max(opts.min_bytes),
    ));

    out.retain(|s| {
        s.bytes >= opts.min_bytes && (opts.risks.is_empty() || opts.risks.contains(&s.risk))
    });
    out.sort_by(|a, b| a.risk.cmp(&b.risk).then(b.bytes.cmp(&a.bytes)));
    let mut totals = Totals::default();
    for s in &out {
        match s.risk {
            Risk::Safe => totals.safe += s.bytes,
            Risk::Review => totals.review += s.bytes,
            Risk::Manual => totals.manual += s.bytes,
        }
    }
    let advice = advice(&disk, &out);
    Ok(SuggestReport {
        disk,
        totals,
        suggestions: out,
        advice,
        catalog_problems: cat.problems,
    })
}

/// Things worth knowing that are not deletable items.
#[must_use]
pub fn advice(disk: &DiskStatus, out: &[Suggestion]) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(c) = &disk.container
        && let Some(vm) = c.volumes.iter().find(|v| v.role == VolumeRole::Vm)
        && vm.consumed >= 8_000_000_000
    {
        a.push(format!(
                    "Swap uses {} because memory is under pressure. It is not deletable; it shrinks when you quit memory-hungry apps (Docker, local LLMs, many browser tabs) or restart. Lowering Docker Desktop's memory limit helps most.",
                    human(vm.consumed)
                ));
    }
    if disk.full_disk_access != FdaStatus::Granted {
        a.push(
            "The Trash and some app folders can't be measured without Full Disk Access. Items you moved to the Trash still use space until you empty it (Finder → Empty Trash)."
                .into(),
        );
    }
    if out
        .iter()
        .any(|s| s.id.ends_with("-logs") && s.bytes >= 2_000_000_000)
    {
        a.push("Editor logs reached several GB: an extension is logging heavily (e.g. C# Dev Kit). Lower its log level, or the logs will grow back.".into());
    }
    if out.iter().any(|s| s.id == "git-worktrees") {
        a.push("Coding agents create a git worktree per task. Removing worktrees after their branch is merged keeps them from piling up.".into());
    }
    a
}
