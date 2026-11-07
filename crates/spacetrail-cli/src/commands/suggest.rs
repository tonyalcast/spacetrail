//! `spacetrail suggest`: what can be freed, how, and at what risk.

use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::PathBuf;

use serde::Serialize;
use spacetrail_cleaners::detectors::{self, DEFAULT_CODE_ROOTS};
use spacetrail_cleaners::suggest::{self, Claims, Context, Suggestion};
use spacetrail_cleaners::{Risk, catalog};
use spacetrail_core::{Dirs, VolumeRole};
use spacetrail_macos::FdaStatus;

use super::Ctx;
use super::status;
use crate::render::pct;

/// Options of `spacetrail suggest`.
#[derive(Debug, Clone)]
pub(crate) struct Options {
    /// Only show these risk classes (all when empty).
    pub(crate) risks: Vec<Risk>,
    /// Hide suggestions smaller than this.
    pub(crate) min_bytes: u64,
    /// Items listed per suggestion.
    pub(crate) items: usize,
    /// Code roots for project detection (defaults when empty).
    pub(crate) roots: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
struct Report {
    disk: status::Status,
    totals: Totals,
    suggestions: Vec<Suggestion>,
    advice: Vec<String>,
    catalog_problems: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
struct Totals {
    safe: u64,
    review: u64,
    manual: u64,
}

fn progress(ctx: &Ctx, msg: &str) {
    if !ctx.json && std::io::stderr().is_terminal() {
        eprint!("\r\x1b[2K{msg}…");
    }
}

fn collect(ctx: &Ctx, opts: &Options) -> anyhow::Result<Report> {
    let home = ctx
        .home
        .clone()
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let disk = status::collect(ctx)?;
    let sys = Context::from_system(home.clone());
    let cat = catalog::load(Dirs::from_env().map(|d| d.user_cleaners()).as_deref());
    let mut claims = Claims::default();
    let mut out: Vec<Suggestion> = Vec::new();
    let min_detector = opts.min_bytes;

    progress(ctx, "Asking Docker what it can free");
    out.extend(detectors::docker(&sys, &mut claims));

    progress(ctx, "Looking for worktrees and stale project artifacts");
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

    progress(ctx, "Checking editors");
    out.extend(detectors::orphaned_workspaces(&sys, &mut claims));
    out.extend(detectors::agent_versions(&sys, &mut claims));

    progress(ctx, "Measuring caches, logs and tool data");
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

    progress(ctx, "Looking at large app data");
    out.extend(detectors::large_app_data(
        &sys,
        &claims,
        2_000_000_000.max(min_detector),
    ));
    progress(ctx, "");
    if !ctx.json && std::io::stderr().is_terminal() {
        eprint!("\r\x1b[2K");
    }

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
    let advice = advice(ctx, &disk, &out);
    Ok(Report {
        disk,
        totals,
        suggestions: out,
        advice,
        catalog_problems: cat.problems,
    })
}

/// Things worth knowing that are not deletable items.
fn advice(ctx: &Ctx, disk: &status::Status, out: &[Suggestion]) -> Vec<String> {
    let b = |n| ctx.style.bytes(n);
    let mut a = Vec::new();
    if let Some(c) = &disk.container {
        if let Some(vm) = c.volumes.iter().find(|v| v.role == VolumeRole::Vm) {
            if vm.consumed >= 8_000_000_000 {
                a.push(format!(
                    "Swap uses {} because memory is under pressure. It is not deletable; it shrinks when you quit memory-hungry apps (Docker, local LLMs, many browser tabs) or restart. Lowering Docker Desktop's memory limit helps most.",
                    b(vm.consumed)
                ));
            }
        }
    }
    match &disk.full_disk_access {
        FdaStatus::Granted => {}
        _ => a.push(
            "The Trash and some app folders can't be measured without Full Disk Access. Items you moved to the Trash still use space until you empty it (Finder → Empty Trash). See `spacetrail doctor` to grant access."
                .into(),
        ),
    }
    if out
        .iter()
        .any(|s| s.id.ends_with("-logs") && s.bytes >= 2_000_000_000)
    {
        a.push("Editor logs reached several GB: an extension is logging heavily. Look at `spacetrail top ~/Library/Application\\ Support/Code/logs` to find it and lower its log level (e.g. C# Dev Kit), or the logs will grow back.".into());
    }
    if out.iter().any(|s| s.id == "git-worktrees") {
        a.push("Coding agents create a git worktree per task. Removing worktrees after their branch is merged keeps them from piling up.".into());
    }
    a
}

fn render(ctx: &Ctx, r: &Report, opts: &Options) -> String {
    let b = |n| ctx.style.bytes(n);
    let mut o = String::new();
    let v = &r.disk.volume;
    let (total, free) = r
        .disk
        .container
        .as_ref()
        .map_or((v.total, v.free), |c| (c.total, c.free));
    let used = total.saturating_sub(free);
    let warn = if free * 10 < total {
        "  ⚠ almost full"
    } else {
        ""
    };
    let _ = writeln!(
        o,
        "Disk: {} of {} used ({}) · {} free{warn}",
        b(used),
        b(total),
        pct(used, total),
        b(free)
    );
    let _ = writeln!(
        o,
        "Could free: ✅ safe {} · ⚠️ review {} · 🔒 manual {}\n",
        b(r.totals.safe),
        b(r.totals.review),
        b(r.totals.manual)
    );

    let home = ctx.home.as_deref();
    let disp = |p: &std::path::Path| spacetrail_core::paths::tilde(p, home);
    for risk in [Risk::Safe, Risk::Review, Risk::Manual] {
        let group: Vec<&Suggestion> = r.suggestions.iter().filter(|s| s.risk == risk).collect();
        if group.is_empty() {
            continue;
        }
        let total: u64 = group.iter().map(|s| s.bytes).sum();
        let _ = writeln!(
            o,
            "{} {} ({}) · {}",
            risk.icon(),
            risk.to_string().to_uppercase(),
            risk.meaning(),
            b(total)
        );
        for s in group {
            let approx = if s.exact { "" } else { "≤" };
            let _ = writeln!(o, "  {approx}{:>9}  {}", b(s.bytes), s.name);
            let _ = writeln!(o, "             {}", s.description);
            for i in s.items.iter().take(opts.items) {
                let note = i
                    .note
                    .as_deref()
                    .map(|n| format!("  ({n})"))
                    .unwrap_or_default();
                let _ = writeln!(
                    o,
                    "             · {:>9}  {}{note}",
                    b(i.bytes),
                    disp(&i.path)
                );
            }
            if s.items.len() > opts.items {
                let rest: u64 = s.items[opts.items..].iter().map(|i| i.bytes).sum();
                let _ = writeln!(
                    o,
                    "             · {:>9}  … {} more",
                    b(rest),
                    s.items.len() - opts.items
                );
            }
            let _ = writeln!(o, "             → {}", s.how.summary);
            for c in &s.how.commands {
                let _ = writeln!(o, "               $ {}", c.join(" "));
            }
            if let Some(regen) = &s.regenerates {
                let _ = writeln!(o, "             ↺ {regen}");
            }
            for n in &s.notes {
                let _ = writeln!(o, "             ! {n}");
            }
        }
        let _ = writeln!(o);
    }
    if !r.advice.is_empty() {
        let _ = writeln!(o, "Also worth knowing");
        for a in &r.advice {
            let _ = writeln!(o, "  • {a}");
        }
    }
    if !r.catalog_problems.is_empty() {
        let _ = writeln!(o, "\nIgnored cleaner files");
        for p in &r.catalog_problems {
            let _ = writeln!(o, "  ✗ {p}");
        }
    }
    let _ = writeln!(
        o,
        "\nNothing was deleted. Commands are shown for you to run; `spacetrail clean` will automate this with dry-run, Trash and undo."
    );
    o
}

pub(crate) fn run(ctx: &Ctx, opts: &Options) -> anyhow::Result<()> {
    let r = collect(ctx, opts)?;
    ctx.emit(&r, || render(ctx, &r, opts))
}

#[cfg(test)]
mod tests {
    #[test]
    fn source_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&spacetrail_cleaners::suggest::Source::Detector).unwrap(),
            "\"detector\""
        );
    }
}
