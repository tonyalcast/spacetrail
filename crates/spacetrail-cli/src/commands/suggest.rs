//! `spacetrail suggest`: what can be freed, how, and at what risk.

use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::PathBuf;

use spacetrail_cleaners::Risk;
use spacetrail_cleaners::report::{self, SuggestOptions, SuggestReport as Report};
use spacetrail_cleaners::suggest::Suggestion;
use spacetrail_core::Dirs;

use super::Ctx;
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

fn collect(ctx: &Ctx, opts: &Options) -> anyhow::Result<Report> {
    let home = ctx
        .home
        .clone()
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let show = !ctx.json && std::io::stderr().is_terminal();
    let progress = |msg: &str| {
        if show {
            eprint!("\r\x1b[2K{msg}…");
        }
    };
    let user = Dirs::from_env().map(|d| d.user_cleaners());
    let r = report::run(
        &SuggestOptions {
            risks: opts.risks.clone(),
            min_bytes: opts.min_bytes,
            roots: opts.roots.clone(),
        },
        &home,
        user.as_deref(),
        &progress,
    )?;
    if show {
        eprint!("\r\x1b[2K");
    }
    Ok(r)
}

fn render(ctx: &Ctx, r: &Report, opts: &Options) -> String {
    let b = |n| ctx.style.bytes(n);
    let mut o = String::new();
    let (total, free) = r.disk.total_free();
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
