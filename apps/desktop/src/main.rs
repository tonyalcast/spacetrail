//! spacetrail desktop app.
//!
//! The window only gets read-only commands: disk status, a whole-disk
//! overview, suggestions, a Full Disk Access check, and two helpers that open
//! System Settings and reveal a path in Finder. Nothing here deletes files.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use spacetrail_cleaners::report::{self, SuggestOptions, SuggestReport};
use spacetrail_core::{
    DirState, DirTree, NodeId, Reconciliation, ScanOptions, ScanProgress, Stats, scan,
};
use spacetrail_macos::system::{DiskStatus, disk_status};
use spacetrail_macos::{DATA_ROOT, FdaStatus, Firmlinks, MacPlatform, apfs, fda, whole_disk_root};
use tauri::{AppHandle, Emitter};

const FDA_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles";

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_owned())
}

#[tauri::command]
fn get_disk_status() -> Result<DiskStatus, String> {
    disk_status(home().ok().as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
fn check_full_disk_access() -> FdaStatus {
    home().map_or(FdaStatus::Unknown, |h| fda::probe(&h))
}

#[tauri::command]
fn open_full_disk_access_settings() -> Result<(), String> {
    // The probe registers the app in the Full Disk Access list (toggle off),
    // so the user only has to switch it on.
    let _ = check_full_disk_access();
    tauri_plugin_opener::open_url(FDA_SETTINGS, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_in_finder(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.is_absolute() || std::fs::symlink_metadata(p).is_err() {
        return Err(format!("{path} does not exist"));
    }
    tauri_plugin_opener::reveal_item_in_dir(p).map_err(|e| e.to_string())
}

#[tauri::command]
fn restart(app: AppHandle) {
    app.restart();
}

/// A folder in the overview tree.
#[derive(Debug, Serialize)]
struct Folder {
    name: String,
    path: String,
    bytes: u64,
    files: u64,
    state: DirState,
    /// Bytes of files directly inside plus subfolders too small to list.
    other: u64,
    children: Vec<Folder>,
}

#[derive(Debug, Serialize)]
struct LargeFile {
    path: String,
    bytes: u64,
}

#[derive(Debug, Serialize)]
struct Overview {
    elapsed_secs: f64,
    totals: Stats,
    reconciliation: Option<Reconciliation>,
    privacy_protected: usize,
    other_owner: usize,
    tree: Folder,
    largest_files: Vec<LargeFile>,
}

#[derive(Clone, Serialize)]
struct ScanTick {
    files: u64,
    bytes: u64,
}

fn display(firmlinks: &Firmlinks, p: &Path) -> String {
    if p == Path::new(DATA_ROOT) {
        "/".to_owned()
    } else {
        firmlinks.to_display(p).display().to_string()
    }
}

fn folder(tree: &DirTree, id: NodeId, fl: &Firmlinks, depth: usize, min: u64) -> Folder {
    let n = tree.get(id);
    let mut children = Vec::new();
    let mut listed = 0u64;
    if depth < 6 {
        for c in tree.children_by_size(id).into_iter().take(60) {
            let b = tree.get(c).total.alloc;
            if b < min {
                break;
            }
            listed += b;
            children.push(folder(tree, c, fl, depth + 1, min));
        }
    }
    let path = tree.path(id);
    Folder {
        name: if id == NodeId::ROOT {
            "Macintosh HD".to_owned()
        } else {
            n.name.to_string_lossy().into_owned()
        },
        path: display(fl, &path),
        bytes: n.total.alloc,
        files: n.total.files,
        state: n.state,
        other: n.total.alloc.saturating_sub(listed),
        children,
    }
}

#[tauri::command]
async fn get_overview(app: AppHandle) -> Result<Overview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = whole_disk_root();
        let progress = ScanProgress::default();
        let done = AtomicBool::new(false);
        let res = std::thread::scope(|s| {
            s.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    let _ = app.emit(
                        "scan-progress",
                        ScanTick {
                            files: progress.files.load(Ordering::Relaxed),
                            bytes: progress.bytes.load(Ordering::Relaxed),
                        },
                    );
                    std::thread::sleep(Duration::from_millis(300));
                }
            });
            let opts = ScanOptions {
                top_files: 40,
                ..ScanOptions::default()
            };
            let r = scan(&root, &MacPlatform, &opts, &progress);
            done.store(true, Ordering::Relaxed);
            r
        })
        .map_err(|e| e.to_string())?;
        let fl = Firmlinks::load().unwrap_or_default();
        let totals = res.tree.root().total;
        let min = (totals.alloc / 500).max(50_000_000);
        let (privacy, other): (Vec<_>, Vec<_>) = res
            .unreadable
            .iter()
            .partition(|u| u.is_privacy_protected());
        Ok(Overview {
            elapsed_secs: res.elapsed.as_secs_f64(),
            totals,
            reconciliation: apfs::container_of(&root).ok().and_then(|c| {
                c.data_volume()
                    .map(|d| Reconciliation::new(d.consumed, totals.alloc))
            }),
            privacy_protected: privacy.len(),
            other_owner: other.len(),
            tree: folder(&res.tree, NodeId::ROOT, &fl, 0, min),
            largest_files: res
                .top_files
                .iter()
                .map(|f| LargeFile {
                    path: display(&fl, &f.path),
                    bytes: f.alloc,
                })
                .collect(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_suggestions(app: AppHandle) -> Result<SuggestReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let home = home()?;
        let user = spacetrail_core::Dirs::from_env().map(|d| d.user_cleaners());
        let progress = |msg: &str| {
            let _ = app.emit("suggest-progress", msg.to_owned());
        };
        report::run(
            &SuggestOptions {
                min_bytes: 50_000_000,
                ..SuggestOptions::default()
            },
            &home,
            user.as_deref(),
            &progress,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn main() {
    // Never download cloud files, whatever happens below.
    let _ = spacetrail_macos::prevent_materialization();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_disk_status,
            check_full_disk_access,
            open_full_disk_access_settings,
            reveal_in_finder,
            restart,
            get_overview,
            get_suggestions
        ])
        .run(tauri::generate_context!())
        .expect("error while running spacetrail");
}
