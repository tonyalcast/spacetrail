//! `spacetrail status`: capacity, where the used space lives, and FDA.

use std::fmt::Write as _;

use spacetrail_macos::FdaStatus;
use spacetrail_macos::system::{DiskStatus, disk_status};

use super::Ctx;
use crate::render::{Align, pct, table};

pub(crate) fn render(ctx: &Ctx, s: &DiskStatus) -> String {
    let b = |n| ctx.style.bytes(n);
    let v = &s.volume;
    let (total, free) = s
        .container
        .as_ref()
        .map_or((v.total, v.free), |c| (c.total, c.free));
    let used = total.saturating_sub(free);
    let mut out = String::new();
    let mut rows = vec![
        vec!["Capacity".into(), b(total), String::new()],
        vec!["Used".into(), b(used), pct(used, total)],
        vec!["Free".into(), b(v.free), pct(v.free, total)],
    ];
    if let (Some(avail), Some(purge)) = (v.available_important, v.purgeable_estimate()) {
        rows.push(vec![
            "Available".into(),
            b(avail),
            format!("free + {} purgeable (what Finder shows)", b(purge)),
        ]);
    }
    let _ = writeln!(out, "Disk");
    out.push_str(&table(&rows, &[Align::Left, Align::Right, Align::Left], 2));

    if let Some(c) = &s.container {
        let _ = writeln!(
            out,
            "\nWhere the used space is (APFS container {})",
            c.device
        );
        let mut rows: Vec<Vec<String>> = c
            .volumes
            .iter()
            .filter(|vol| vol.consumed > 0)
            .map(|vol| {
                vec![
                    vol.role.label().to_owned(),
                    b(vol.consumed),
                    pct(vol.consumed, used),
                    vol.role.explanation().to_owned(),
                ]
            })
            .collect();
        if c.overhead() > 0 {
            rows.push(vec![
                "Overhead".into(),
                b(c.overhead()),
                pct(c.overhead(), used),
                "APFS metadata not assigned to a volume".into(),
            ]);
        }
        out.push_str(&table(
            &rows,
            &[Align::Left, Align::Right, Align::Right, Align::Left],
            2,
        ));
    }

    let _ = writeln!(out);
    match &s.full_disk_access {
        FdaStatus::Granted => {
            let _ = writeln!(out, "Full Disk Access: ✓ granted");
        }
        FdaStatus::Denied { .. } => {
            let _ = writeln!(
                out,
                "Full Disk Access: ✗ not granted. Some folders can't be measured; see `spacetrail doctor`."
            );
        }
        FdaStatus::Unknown => {
            let _ = writeln!(out, "Full Disk Access: ? unknown");
        }
    }
    out
}

pub(crate) fn run(ctx: &Ctx) -> anyhow::Result<()> {
    let s = disk_status(ctx.home.as_deref())?;
    ctx.emit(&s, || render(ctx, &s))
}
