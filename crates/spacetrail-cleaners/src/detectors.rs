//! Built-in detectors for things a static path pattern cannot express:
//! stale project artifacts, git worktrees, orphaned editor workspaces,
//! superseded agent versions, Docker's own accounting and large app data.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use spacetrail_core::{DirTree, NodeId, ScanOptions, ScanProgress, human, parse_duration, scan};
use spacetrail_macos::MacPlatform;
use spacetrail_macos::docker;

use crate::schema::{Cleaner, Risk};
use crate::suggest::{Claims, Context, How, Item, Source, Suggestion, age_text, measure};

/// Default code roots searched for projects (decision Q6).
pub const DEFAULT_CODE_ROOTS: &[&str] = &[
    "~/Documents/dev",
    "~/Code",
    "~/Developer",
    "~/Projects",
    "~/src",
    "~/dev",
];

const DAY: i64 = 86_400;

fn detector(id: &str, name: &str, risk: Risk, description: &str, reason: &str) -> Suggestion {
    Suggestion {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        category: "detector".into(),
        risk,
        risk_reason: reason.into(),
        regenerates: None,
        docs: None,
        bytes: 0,
        exact: true,
        items: Vec::new(),
        how: How::default(),
        blocked_by: Vec::new(),
        notes: Vec::new(),
        source: Source::Detector,
    }
}

fn scan_tree(root: &Path) -> Option<DirTree> {
    let opts = ScanOptions {
        top_files: 0,
        ..ScanOptions::default()
    };
    scan(root, &MacPlatform, &opts, &ScanProgress::default())
        .ok()
        .map(|r| r.tree)
}

// ───────────────────────────── projects & worktrees ─────────────────────────────

struct ProjectRule<'a> {
    cleaner: &'a Cleaner,
    markers: Vec<GlobMatcher>,
    literal_markers: Vec<String>,
    stale_after: i64,
}

impl ProjectRule<'_> {
    /// `true` if `dir` directly contains one of the markers.
    fn is_project(&self, dir: &Path) -> bool {
        if self.literal_markers.iter().any(|m| dir.join(m).exists()) {
            return true;
        }
        if self.markers.is_empty() {
            return false;
        }
        std::fs::read_dir(dir).is_ok_and(|rd| {
            rd.flatten()
                .any(|e| self.markers.iter().any(|m| m.is_match(e.file_name())))
        })
    }
}

/// Reads the branch of a git worktree from its `.git` file.
fn worktree_branch(worktree: &Path) -> Option<String> {
    let link = std::fs::read_to_string(worktree.join(".git")).ok()?;
    let gitdir = link.strip_prefix("gitdir:")?.trim();
    let gitdir = if Path::new(gitdir).is_absolute() {
        PathBuf::from(gitdir)
    } else {
        worktree.join(gitdir)
    };
    let head = std::fs::read_to_string(gitdir.join("HEAD")).ok()?;
    Some(head.trim().strip_prefix("ref: refs/heads/").map_or_else(
        || format!("detached at {}", &head.trim()[..head.trim().len().min(9)]),
        str::to_owned,
    ))
}

/// Stale build artifacts and git worktrees under the code roots.
#[must_use]
pub fn projects(
    ctx: &Context,
    cleaners: &[(Cleaner, Source)],
    roots: &[PathBuf],
    claims: &mut Claims,
) -> Vec<Suggestion> {
    let rules: Vec<ProjectRule<'_>> = cleaners
        .iter()
        .filter_map(|(c, _)| {
            let p = c.project.as_ref()?;
            let (globs, literal): (Vec<_>, Vec<_>) =
                p.markers.iter().partition(|m| m.contains(['*', '?', '[']));
            Some(ProjectRule {
                cleaner: c,
                markers: globs
                    .iter()
                    .filter_map(|g| GlobBuilder::new(g).case_insensitive(true).build().ok())
                    .map(|g| g.compile_matcher())
                    .collect(),
                literal_markers: literal.into_iter().cloned().collect(),
                stale_after: p
                    .stale_after
                    .as_deref()
                    .and_then(|d| parse_duration(d).ok())
                    .map_or(60 * DAY, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX)),
            })
        })
        .collect();
    let artifact_names: HashSet<&str> = rules
        .iter()
        .flat_map(|r| {
            r.cleaner
                .project
                .as_ref()
                .into_iter()
                .flat_map(|p| p.artifacts.iter().map(String::as_str))
        })
        .collect();

    let mut worktrees = detector(
        "git-worktrees",
        "Git worktrees",
        Risk::Review,
        "Extra working copies of your repositories (e.g. created by Claude Code, Cursor or `git worktree add`). Each one carries its own dependencies and build output.",
        "Removing a worktree keeps its branch and commits, but uncommitted changes in it would be lost; git refuses to remove dirty worktrees unless forced.",
    );
    worktrees.how = How {
        summary: "Remove worktrees whose work is merged or abandoned (the branch is kept)".into(),
        commands: vec![
            vec!["git".into(), "worktree".into(), "list".into()],
            vec!["git".into(), "worktree".into(), "remove".into(), "<path>".into()],
            vec!["git".into(), "worktree".into(), "prune".into()],
        ],
        instructions: Some("Run the commands from the main repository. `git worktree remove` refuses if the worktree has uncommitted changes.".into()),
    };
    let mut per_rule: Vec<Suggestion> = rules
        .iter()
        .map(|r| {
            let c = r.cleaner;
            let mut s = detector(
                &c.id,
                &c.name,
                c.risk,
                c.description.trim(),
                c.risk_reason.trim(),
            );
            s.category.clone_from(&c.category);
            s.regenerates.clone_from(&c.regenerates);
            s.how.summary = format!(
                "Move these folders to the Trash; projects untouched for {} days",
                r.stale_after / DAY
            );
            s
        })
        .collect();
    let mut active_bytes = vec![0u64; rules.len()];

    let mut seen_roots = HashSet::new();
    for root in roots {
        let Ok(real) = root.canonicalize() else {
            continue;
        };
        if !seen_roots.insert(real.clone()) || claims.covers(&real) {
            continue;
        }
        let Some(tree) = scan_tree(root) else {
            continue;
        };

        // 1. Worktrees: directories whose `.git` is a file.
        let mut worktree_ids = HashSet::new();
        for id in tree.ids() {
            if id == NodeId::ROOT || tree.get(id).name == ".git" {
                continue;
            }
            let path = tree.path(id);
            if std::fs::symlink_metadata(path.join(".git")).is_ok_and(|m| m.is_file()) {
                let n = tree.get(id);
                worktree_ids.insert(id);
                let branch = worktree_branch(&path).unwrap_or_else(|| "unknown branch".into());
                if let Some(bytes) = claims.adjust(&path, n.total.alloc) {
                    claims.claim(path.clone(), bytes);
                    worktrees.items.push(Item {
                        note: Some(format!(
                            "{branch} · last change {}",
                            age_text(ctx.now, n.total.newest_mtime)
                        )),
                        path,
                        bytes,
                        newest_mtime: n.total.newest_mtime,
                    });
                }
            }
        }

        // 2. Artifact directories whose parent is a project of the matching rule.
        let mut artifact: Vec<Option<usize>> = vec![None; tree.len()];
        let mut inside_artifact = vec![false; tree.len()];
        for id in tree.ids() {
            let n = tree.get(id);
            if let Some(p) = n.parent {
                if inside_artifact[p.index()] || artifact[p.index()].is_some() {
                    inside_artifact[id.index()] = true;
                    continue;
                }
            }
            let name = n.name.to_string_lossy();
            if !artifact_names.contains(name.as_ref()) {
                continue;
            }
            let Some(parent) = n.parent else { continue };
            let parent_path = tree.path(parent);
            artifact[id.index()] = rules.iter().position(|r| {
                r.cleaner
                    .project
                    .as_ref()
                    .is_some_and(|p| p.artifacts.iter().any(|a| a == name.as_ref()))
                    && r.is_project(&parent_path)
            });
        }

        // Newest mtime of each subtree, ignoring artifact subtrees.
        let mut newest_src = vec![0i64; tree.len()];
        let ids: Vec<NodeId> = tree.ids().collect();
        for &id in ids.iter().rev() {
            let n = tree.get(id);
            let mut v = n.own.newest_mtime;
            for c in tree.children(id) {
                if artifact[c.index()].is_none() && !inside_artifact[c.index()] {
                    v = v.max(newest_src[c.index()]);
                }
            }
            newest_src[id.index()] = v;
        }

        for id in ids {
            let Some(rule_ix) = artifact[id.index()] else {
                continue;
            };
            let n = tree.get(id);
            let parent = n.parent.expect("artifacts have a parent");
            // The project's activity: the enclosing worktree/repo if any, else the parent.
            let mut owner = parent;
            let mut cur = Some(parent);
            while let Some(c) = cur {
                if worktree_ids.contains(&c) || tree.path(c).join(".git").is_dir() {
                    owner = c;
                    break;
                }
                cur = tree.get(c).parent;
            }
            let last = newest_src[owner.index()];
            let path = tree.path(id);
            let Some(bytes) = claims.adjust(&path, n.total.alloc) else {
                continue;
            };
            if ctx.now - last < rules[rule_ix].stale_after {
                active_bytes[rule_ix] += bytes;
                continue;
            }
            claims.claim(path.clone(), bytes);
            per_rule[rule_ix].items.push(Item {
                note: Some(format!("project last changed {}", age_text(ctx.now, last))),
                path,
                bytes,
                newest_mtime: last,
            });
        }
    }

    let mut out = Vec::new();
    worktrees.finish();
    if !worktrees.items.is_empty() {
        out.push(worktrees);
    }
    for (mut s, active) in per_rule.into_iter().zip(active_bytes) {
        s.finish();
        if active > 0 {
            s.notes.push(format!(
                "{} more in recently active projects is kept",
                human(active)
            ));
        }
        if !s.items.is_empty() {
            out.push(s);
        }
    }
    out
}

// ───────────────────────────── editors ─────────────────────────────

/// Decodes `%XX` escapes of a `file://` URI path.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Editors based on VS Code that keep per-workspace state.
const EDITORS: &[&str] = &["Code", "Cursor", "Windsurf", "Antigravity"];

/// Per-workspace editor state whose project folder no longer exists.
#[must_use]
pub fn orphaned_workspaces(ctx: &Context, claims: &mut Claims) -> Option<Suggestion> {
    let mut s = detector(
        "editor-orphaned-workspaces",
        "Editor state of deleted projects",
        Risk::Safe,
        "VS Code, Cursor, Windsurf and Antigravity keep per-project state (history, chat, indexes). These entries belong to folders that no longer exist.",
        "The folders they describe are gone; the editor never opens them again.",
    );
    s.regenerates = Some("Not needed: the projects no longer exist.".into());
    s.how.summary = "Move these folders to the Trash (with the editor closed)".into();
    for app in EDITORS {
        let dir = ctx
            .home
            .join("Library/Application Support")
            .join(app)
            .join("User/workspaceStorage");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut found = false;
        for e in entries.flatten() {
            let Ok(text) = std::fs::read_to_string(e.path().join("workspace.json")) else {
                continue;
            };
            let Ok(v) = serde_json_lite(&text) else {
                continue;
            };
            let Some(uri) = v else { continue };
            let Some(path) = uri.strip_prefix("file://") else {
                continue;
            };
            let target = PathBuf::from(percent_decode(path));
            if target.exists() {
                continue;
            }
            let Some(m) = measure(&e.path()) else {
                continue;
            };
            let Some(bytes) = claims.adjust(&e.path(), m.bytes) else {
                continue;
            };
            claims.claim(e.path(), bytes);
            found = true;
            s.items.push(Item {
                path: e.path(),
                bytes,
                newest_mtime: m.newest_mtime,
                note: Some(format!("{app}: {}", target.display())),
            });
        }
        if found {
            let running = ctx.running(&[(*app).to_owned()]);
            s.blocked_by.extend(running);
        }
    }
    s.finish();
    if !s.blocked_by.is_empty() {
        s.notes
            .push(format!("close {} first", s.blocked_by.join(", ")));
    }
    (!s.items.is_empty()).then_some(s)
}

/// Extracts `folder` or `workspace` from a VS Code `workspace.json`.
fn serde_json_lite(text: &str) -> Result<Option<String>, ()> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|_| ())?;
    Ok(v.get("folder")
        .or_else(|| v.get("workspace"))
        .and_then(|x| x.as_str())
        .map(str::to_owned))
}

/// Old versions of the Cursor agent CLI kept next to the active one.
#[must_use]
pub fn agent_versions(ctx: &Context, claims: &mut Claims) -> Option<Suggestion> {
    let mut s = detector(
        "cursor-agent-old-versions",
        "Old Cursor agent versions",
        Risk::Safe,
        "Cursor's agent keeps every downloaded version; only the one its launcher points to is used.",
        "The active version (target of the `cursor-agent` launcher) is kept.",
    );
    s.regenerates = Some("Not needed: newer versions replace them.".into());
    s.how.summary = "Move the old versions to the Trash".into();
    let installs = [
        ctx.home.join("Library/Application Support/Cursor/User/globalStorage/anysphere.cursor-agent-worker/agent-cli/.local"),
        ctx.home.join(".local"),
    ];
    for base in installs {
        let versions = base.join("share/cursor-agent/versions");
        let Ok(active) = std::fs::read_link(base.join("bin/cursor-agent")) else {
            continue;
        };
        let active_dir = active.parent().map(Path::to_path_buf).unwrap_or_default();
        let Ok(entries) = std::fs::read_dir(&versions) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p == active_dir
                || std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink())
            {
                continue;
            }
            let Some(m) = measure(&p) else { continue };
            let Some(bytes) = claims.adjust(&p, m.bytes) else {
                continue;
            };
            claims.claim(p.clone(), bytes);
            s.items.push(Item {
                path: p,
                bytes,
                newest_mtime: m.newest_mtime,
                note: None,
            });
        }
        // Leftover downloads next to the install.
        if let Some(agent_cli) = base.parent() {
            if let Ok(entries) = std::fs::read_dir(agent_cli) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "gz")
                        && p.to_string_lossy().contains("cursor-agent")
                    {
                        if let Some(m) = measure(&p) {
                            claims.claim(p.clone(), m.bytes);
                            s.items.push(Item {
                                path: p,
                                bytes: m.bytes,
                                newest_mtime: m.newest_mtime,
                                note: Some("leftover download".into()),
                            });
                        }
                    }
                }
            }
        }
    }
    s.finish();
    (!s.items.is_empty()).then_some(s)
}

// ───────────────────────────── Docker ─────────────────────────────

/// Docker's own reclaimable space (images, build cache, containers, volumes).
#[must_use]
pub fn docker(ctx: &Context, claims: &mut Claims) -> Vec<Suggestion> {
    let container = ctx.home.join("Library/Containers/com.docker.docker");
    let raw = container.join("Data/vms/0/data/Docker.raw");
    let Some(disk) = measure(&raw) else {
        return Vec::new();
    };
    if let Some(m) = measure(&container) {
        claims.claim(container.clone(), m.bytes);
    }
    let disk_note = format!(
        "Docker's disk image uses {} on your Mac; after pruning, Docker Desktop gives space back gradually (restarting Docker speeds it up).",
        human(disk.bytes)
    );
    let raw_item = |bytes| Item {
        path: raw.clone(),
        bytes,
        newest_mtime: disk.newest_mtime,
        note: None,
    };
    let rows = match docker::system_df() {
        Ok(rows) => rows,
        Err(_) => {
            let mut s = detector(
                "docker-data",
                "Docker data (engine not running)",
                Risk::Review,
                "Images, containers, volumes and build cache stored in Docker Desktop's disk image.",
                "Start Docker so spacetrail can ask it what is reclaimable.",
            );
            s.items.push(raw_item(disk.bytes));
            s.exact = false;
            s.how.summary = "Start Docker Desktop, then run `spacetrail suggest` again".into();
            s.finish();
            return vec![s];
        }
    };
    let mut out = Vec::new();
    for row in rows {
        let (id, name, risk, reason, cmd): (&str, &str, Risk, &str, Vec<&str>) =
            match row.kind.as_str() {
                "Build Cache" => (
                    "docker-build-cache",
                    "Docker build cache",
                    Risk::Review,
                    "Rebuilding images afterwards is slower and re-downloads layers.",
                    vec!["docker", "builder", "prune", "--force"],
                ),
                "Images" => (
                    "docker-unused-images",
                    "Docker images not used by any container",
                    Risk::Review,
                    "Images are pulled or rebuilt again when needed (network, time).",
                    vec!["docker", "image", "prune", "--all", "--force"],
                ),
                "Containers" => (
                    "docker-stopped-containers",
                    "Stopped Docker containers",
                    Risk::Review,
                    "Stopped containers may hold files written inside them.",
                    vec!["docker", "container", "prune", "--force"],
                ),
                "Local Volumes" => (
                    "docker-unused-volumes",
                    "Docker volumes not used by any container",
                    Risk::Manual,
                    "Volumes usually hold databases and other persistent data.",
                    vec!["docker", "volume", "ls", "--filter", "dangling=true"],
                ),
                _ => continue,
            };
        if row.reclaimable == 0 {
            continue;
        }
        let mut s = detector(
            id,
            name,
            risk,
            &format!(
                "{} of {} {} in use ({} total).",
                row.active,
                row.total,
                row.kind.to_lowercase(),
                human(row.size)
            ),
            reason,
        );
        s.category = "containers/docker".into();
        s.exact = false;
        s.items.push(raw_item(row.reclaimable));
        s.how = How {
            summary: if risk == Risk::Manual {
                "Inspect the volumes; remove only those you know are disposable".into()
            } else {
                "Run the command below".into()
            },
            commands: vec![cmd.into_iter().map(str::to_owned).collect()],
            instructions: None,
        };
        s.notes.push(disk_note.clone());
        s.finish();
        out.push(s);
    }
    out
}

// ───────────────────────────── app data ─────────────────────────────

/// Large app data folders not covered by any other suggestion.
#[must_use]
pub fn large_app_data(ctx: &Context, claims: &Claims, min_bytes: u64) -> Option<Suggestion> {
    let mut s = detector(
        "large-app-data",
        "Large app data",
        Risk::Manual,
        "Data stored by individual apps (games, offline content, VMs, browser profiles, chat history).",
        "This is app data: remove it from inside the app or uninstall apps you no longer use.",
    );
    s.how.summary = "Clean it up from inside each app, or uninstall apps you no longer use".into();
    let parents = [
        "Library/Application Support",
        "Library/Containers",
        "Library/Group Containers",
    ];
    for parent in parents {
        let Some(tree) = scan_tree(&ctx.home.join(parent)) else {
            continue;
        };
        for c in tree.children(NodeId::ROOT) {
            let path = tree.path(c);
            let n = tree.get(c);
            let Some(bytes) = claims.adjust(&path, n.total.alloc) else {
                continue;
            };
            if bytes >= min_bytes {
                s.items.push(Item {
                    path,
                    bytes,
                    newest_mtime: n.total.newest_mtime,
                    note: Some(format!(
                        "last used {}",
                        age_text(ctx.now, n.total.newest_mtime)
                    )),
                });
            }
        }
    }
    s.finish();
    (!s.items.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;
    use crate::suggest::now;

    fn ctx(home: &Path) -> Context {
        Context {
            home: home.to_owned(),
            env: Vec::new(),
            now: now(),
            processes: Vec::new(),
            path_dirs: Vec::new(),
        }
    }

    fn write(p: &Path, len: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![1u8; len]).unwrap();
    }

    #[test]
    fn decodes_uris() {
        assert_eq!(
            percent_decode("/Users/a/My%20Project"),
            "/Users/a/My Project"
        );
        assert_eq!(percent_decode("/x%2"), "/x%2");
    }

    #[test]
    fn finds_stale_artifacts_and_worktrees() {
        let tmp = tempfile::tempdir().unwrap();
        let h = tmp.path();
        let code = h.join("Code");
        write(&code.join("old/package.json"), 10);
        write(&code.join("old/node_modules/x/index.js"), 200_000);
        write(&code.join("fresh/package.json"), 10);
        write(&code.join("fresh/node_modules/y/index.js"), 200_000);
        write(&code.join("api/App.csproj"), 10);
        write(&code.join("api/bin/Debug/app.dll"), 300_000);
        // A worktree: `.git` is a file.
        write(&code.join("repo/.claude/worktrees/feat/package.json"), 10);
        write(
            &code.join("repo/.claude/worktrees/feat/node_modules/z/i.js"),
            100_000,
        );
        std::fs::write(
            code.join("repo/.claude/worktrees/feat/.git"),
            "gitdir: /nonexistent\n",
        )
        .unwrap();
        std::fs::create_dir_all(code.join("repo/.git")).unwrap();

        let cat = catalog::load(None);
        let mut later = ctx(h);
        later.now += 365 * DAY; // everything is a year old…
        let out = projects(
            &later,
            &cat.cleaners,
            std::slice::from_ref(&code),
            &mut Claims::default(),
        );
        let ids: Vec<&str> = out.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"git-worktrees"), "{ids:?}");
        assert!(ids.contains(&"project-node-modules"), "{ids:?}");
        assert!(ids.contains(&"project-dotnet-build"), "{ids:?}");
        let nm = out.iter().find(|s| s.id == "project-node-modules").unwrap();
        assert_eq!(
            nm.items.len(),
            2,
            "worktree node_modules is part of the worktree"
        );

        // …and with today's clock nothing is stale, but worktrees are still listed.
        let out = projects(&ctx(h), &cat.cleaners, &[code], &mut Claims::default());
        assert!(out.iter().all(|s| s.id == "git-worktrees"));
    }

    #[test]
    fn finds_orphaned_workspaces() {
        let tmp = tempfile::tempdir().unwrap();
        let h = tmp.path();
        let ws = h.join("Library/Application Support/Cursor/User/workspaceStorage");
        write(&ws.join("aaa/state.vscdb"), 50_000);
        std::fs::write(
            ws.join("aaa/workspace.json"),
            r#"{"folder":"file:///nonexistent/My%20Old"}"#,
        )
        .unwrap();
        write(&ws.join("bbb/state.vscdb"), 50_000);
        std::fs::write(
            ws.join("bbb/workspace.json"),
            format!(r#"{{"folder":"file://{}"}}"#, h.display()),
        )
        .unwrap();
        let s = orphaned_workspaces(&ctx(h), &mut Claims::default()).unwrap();
        assert_eq!(s.items.len(), 1);
        assert!(s.items[0].note.as_deref().unwrap().contains("My Old"));
    }

    #[test]
    fn keeps_active_agent_version() {
        let tmp = tempfile::tempdir().unwrap();
        let h = tmp.path();
        let base = h.join(".local");
        write(&base.join("share/cursor-agent/versions/v1/agent"), 10_000);
        write(&base.join("share/cursor-agent/versions/v2/agent"), 10_000);
        std::fs::create_dir_all(base.join("bin")).unwrap();
        std::os::unix::fs::symlink(
            base.join("share/cursor-agent/versions/v2/agent"),
            base.join("bin/cursor-agent"),
        )
        .unwrap();
        let s = agent_versions(&ctx(h), &mut Claims::default()).unwrap();
        assert_eq!(s.items.len(), 1);
        assert!(s.items[0].path.ends_with("v1"));
    }

    #[test]
    fn large_app_data_respects_claims() {
        let tmp = tempfile::tempdir().unwrap();
        let h = tmp.path();
        write(
            &h.join("Library/Application Support/Game/data.bin"),
            400_000,
        );
        write(&h.join("Library/Application Support/Tiny/x"), 10);
        let mut claims = Claims::default();
        claims.claim(h.join("Library/Application Support/Game/data.bin"), 350_000);
        let s = large_app_data(&ctx(h), &claims, 30_000).unwrap();
        assert_eq!(s.items.len(), 1);
        assert!(s.items[0].bytes < 400_000, "claimed bytes are subtracted");
    }
}
