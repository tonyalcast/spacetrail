//! `spacetrail doctor`: environment checks with remediation hints.

use std::fmt::Write as _;

use serde::Serialize;
use spacetrail_core::Dirs;
use spacetrail_macos::{FdaStatus, apfs, fda, volume, whole_disk_root};

use super::Ctx;

#[derive(Debug, Serialize)]
struct Doctor {
    version: &'static str,
    full_disk_access: FdaStatus,
    dataless_guard: Result<(), String>,
    data_volume: Option<DataVolume>,
    container: Result<String, String>,
    dirs: Option<Dirs>,
}

#[derive(Debug, Serialize)]
struct DataVolume {
    mount: String,
    device: String,
    fs_type: String,
}

pub(crate) fn run(ctx: &Ctx, dataless_guard: &std::io::Result<()>) -> anyhow::Result<()> {
    let root = whole_disk_root();
    let d = Doctor {
        version: env!("CARGO_PKG_VERSION"),
        full_disk_access: ctx.home.as_deref().map_or(FdaStatus::Unknown, fda::probe),
        dataless_guard: match dataless_guard {
            Ok(()) => Ok(()),
            Err(e) => Err(e.to_string()),
        },
        data_volume: volume::statfs(&root).ok().map(|s| DataVolume {
            mount: s.mounted_on.display().to_string(),
            device: s.mounted_from,
            fs_type: s.fs_type,
        }),
        container: apfs::container_of(&root)
            .map(|c| format!("{} ({} volumes)", c.device, c.volumes.len()))
            .map_err(|e| e.to_string()),
        dirs: Dirs::from_env(),
    };
    ctx.emit(&d, || render(ctx, &d))
}

fn render(ctx: &Ctx, d: &Doctor) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "spacetrail {}", d.version);
    match &d.full_disk_access {
        FdaStatus::Granted => {
            let _ = writeln!(o, "✓ Full Disk Access     granted");
        }
        FdaStatus::Denied { probe } => {
            let _ = writeln!(
                o,
                "✗ Full Disk Access     not granted (cannot read {})",
                ctx.display(probe)
            );
            let _ = writeln!(
                o,
                "    Without it, folders such as Mail, Messages, Safari, iCloud Drive and many\n    app Group Containers are reported as \"unreadable (size unknown)\".\n    To grant it: System Settings → Privacy & Security → Full Disk Access →\n    enable your terminal app (Terminal, iTerm, Ghostty…), then restart it."
            );
        }
        FdaStatus::Unknown => {
            let _ = writeln!(o, "? Full Disk Access     could not be determined");
        }
    }
    match &d.dataless_guard {
        Ok(()) => {
            let _ = writeln!(
                o,
                "✓ Cloud files          never downloaded (materialization disabled)"
            );
        }
        Err(e) => {
            let _ = writeln!(
                o,
                "✗ Cloud files          guard failed: {e} (scans still use lstat only)"
            );
        }
    }
    if let Some(v) = &d.data_volume {
        let _ = writeln!(
            o,
            "✓ Data volume          {} ({}, {})",
            v.mount,
            v.device.trim_start_matches("/dev/"),
            v.fs_type
        );
    }
    match &d.container {
        Ok(c) => {
            let _ = writeln!(o, "✓ APFS container       {c}");
        }
        Err(e) => {
            let _ = writeln!(o, "✗ APFS container       {e}");
        }
    }
    if let Some(dirs) = &d.dirs {
        let _ = writeln!(o, "  Config               {}", ctx.display(&dirs.config));
        let _ = writeln!(o, "  Data                 {}", ctx.display(&dirs.data));
        if dirs.sandboxed {
            let _ = writeln!(o, "  (SPACETRAIL_HOME is set: sandboxed)");
        }
    }
    o
}
