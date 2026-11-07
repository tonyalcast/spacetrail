//! How "your call" (manual) suggestions are removed when the user explicitly
//! picks items: each through the tool's own uninstaller when there is one,
//! otherwise to the Trash. Items the tool still needs are marked protected.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::suggest::{Context, Exec, Item, Suggestion, measure};

/// Gives a manual suggestion its correct removal method (or none).
pub fn enrich(s: &mut Suggestion, ctx: &Context) {
    match s.id.as_str() {
        "ollama-models" => ollama(s, ctx),
        "rustup-toolchains" => rustup(s, ctx),
        "nvm-node-versions" => nvm(s, ctx),
        "simulator-runtimes" => simulator_runtimes(s),
        "simulator-devices" => {
            s.exec = spacetrail_macos::simulators::simctl_path().map_or(Exec::None, |simctl| {
                Exec::Command {
                    argv: vec![
                        simctl.display().to_string(),
                        "delete".into(),
                        "unavailable".into(),
                    ],
                }
            });
            s.how.summary = "Delete simulators whose runtime is no longer installed".into();
        }
        "android-avds" => {
            for i in &mut s.items {
                let ini = i.path.with_extension("ini");
                if ini.exists() {
                    i.companions.push(ini);
                }
            }
            s.exec = Exec::RemoveItems;
        }
        "huggingface-cache" => huggingface(s, ctx),
        "android-sdk-images" | "xcode-archives" | "lmstudio-models" | "claude-vm-bundle" => {
            s.exec = Exec::RemoveItems;
        }
        // System-owned (e.g. /Library/Updates): only Software Update may remove it.
        _ => {}
    }
}

/// `hf cache rm <type>/<repo> --yes` per cached repo (folders are named
/// `models--org--name`, `datasets--…`, `spaces--…`); Trash if `hf` is missing.
fn huggingface(s: &mut Suggestion, ctx: &Context) {
    let hf = ctx.which("hf");
    for i in &mut s.items {
        let name = name_of(&i.path);
        let Some((kind, repo)) = name.split_once("--") else {
            continue;
        };
        let kind = match kind {
            "models" => "model",
            "datasets" => "dataset",
            "spaces" => "space",
            _ => continue,
        };
        let repo = repo.replace("--", "/");
        i.note = Some(format!("{kind} {repo}"));
        i.argv = hf.as_ref().map(|hf| {
            vec![
                hf.display().to_string(),
                "cache".into(),
                "rm".into(),
                format!("{kind}/{repo}"),
                "--yes".into(),
            ]
        });
    }
    s.exec = Exec::RemoveItems;
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One item per model, removed with `ollama rm` (blobs shared between models
/// are only deleted when no model uses them anymore).
fn ollama(s: &mut Suggestion, ctx: &Context) {
    let Some(ollama) = ctx.which("ollama") else {
        s.exec = Exec::None;
        s.notes.push(
            "Install or start Ollama to remove models; deleting its files by hand breaks it."
                .into(),
        );
        return;
    };
    let base = ctx
        .expand("${OLLAMA_MODELS:-~/.ollama/models}")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("manifests");
    let mut items = Vec::new();
    // manifests/<registry>/<namespace>/<model>/<tag>
    for reg in read_dirs(&base) {
        for ns in read_dirs(&reg) {
            for model in read_dirs(&ns) {
                let Ok(tags) = std::fs::read_dir(&model) else {
                    continue;
                };
                for tag in tags.flatten().map(|e| e.path()).filter(|p| p.is_file()) {
                    let (r, n, m, t) =
                        (name_of(&reg), name_of(&ns), name_of(&model), name_of(&tag));
                    let name = match (r.as_str(), n.as_str()) {
                        ("registry.ollama.ai", "library") => format!("{m}:{t}"),
                        ("registry.ollama.ai", _) => format!("{n}/{m}:{t}"),
                        _ => format!("{r}/{n}/{m}:{t}"),
                    };
                    let bytes = std::fs::read(&tag)
                        .ok()
                        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                        .map_or(0, |v| {
                            let size =
                                |x: &Value| x.get("size").and_then(Value::as_u64).unwrap_or(0);
                            v.get("layers")
                                .and_then(Value::as_array)
                                .map_or(0, |l| l.iter().map(size).sum::<u64>())
                                + v.get("config").map_or(0, size)
                        });
                    items.push(Item {
                        path: tag,
                        bytes,
                        note: Some(format!("model {name}")),
                        argv: Some(vec![ollama.display().to_string(), "rm".into(), name]),
                        ..Default::default()
                    });
                }
            }
        }
    }
    if !items.is_empty() {
        s.items = items;
        s.finish();
        s.notes
            .push("Models can share layers: the space actually freed may be smaller.".into());
    }
    s.exec = Exec::RemoveItems;
}

fn read_dirs(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// `rustup toolchain uninstall`, never the default toolchain.
fn rustup(s: &mut Suggestion, ctx: &Context) {
    let rustup = ctx.which("rustup");
    let home = ctx
        .expand("${RUSTUP_HOME:-~/.rustup}")
        .map(PathBuf::from)
        .unwrap_or_default();
    let default = std::fs::read_to_string(home.join("settings.toml"))
        .ok()
        .and_then(|t| {
            t.lines().find_map(|l| {
                l.trim().strip_prefix("default_toolchain").map(|r| {
                    r.trim_start_matches([' ', '='])
                        .trim()
                        .trim_matches('"')
                        .to_owned()
                })
            })
        })
        .unwrap_or_default();
    for i in &mut s.items {
        let name = name_of(&i.path);
        if !default.is_empty() && (name == default || name.starts_with(&format!("{default}-"))) {
            i.protected = Some("your default toolchain".into());
        }
        i.argv = rustup.as_ref().map(|r| {
            vec![
                r.display().to_string(),
                "toolchain".into(),
                "uninstall".into(),
                name,
            ]
        });
    }
    s.exec = Exec::RemoveItems;
}

/// Node versions go to the Trash (that is what `nvm uninstall` does), never
/// the default one.
fn nvm(s: &mut Suggestion, ctx: &Context) {
    let alias = std::fs::read_to_string(ctx.home.join(".nvm/alias/default")).unwrap_or_default();
    let alias = alias.trim().trim_start_matches('v');
    let names: Vec<String> = s.items.iter().map(|i| name_of(&i.path)).collect();
    // "22" → the newest v22.x installed; "22.14.0" → exactly that one.
    let default = names
        .iter()
        .filter(|n| {
            let v = n.trim_start_matches('v');
            !alias.is_empty() && (v == alias || v.starts_with(&format!("{alias}.")))
        })
        .max_by_key(|n| {
            n.trim_start_matches('v')
                .split('.')
                .map(|x| x.parse::<u32>().unwrap_or(0))
                .collect::<Vec<_>>()
        })
        .cloned();
    for i in &mut s.items {
        if Some(name_of(&i.path)) == default {
            i.protected = Some("your default Node version".into());
        }
    }
    s.exec = Exec::RemoveItems;
}

/// One item per runtime, removed with `simctl runtime delete`.
fn simulator_runtimes(s: &mut Suggestion) {
    let (Some(simctl), Ok(runtimes)) = (
        spacetrail_macos::simulators::simctl_path(),
        spacetrail_macos::simulators::runtimes(),
    ) else {
        s.exec = Exec::None;
        s.notes
            .push("Open Xcode → Settings → Components to remove simulator runtimes.".into());
        return;
    };
    let items: Vec<Item> = runtimes
        .into_iter()
        .map(|r| Item {
            bytes: r.size_bytes.max(measure(&r.path).map_or(0, |m| m.bytes)),
            note: Some(r.label()),
            protected: (!r.deletable).then(|| "macOS says it can't be deleted".to_owned()),
            argv: Some(vec![
                simctl.display().to_string(),
                "runtime".into(),
                "delete".into(),
                r.identifier,
            ]),
            path: r.path,
            ..Default::default()
        })
        .collect();
    if !items.is_empty() {
        s.items = items;
        s.finish();
    }
    s.exec = Exec::RemoveItems;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Risk;
    use crate::suggest::{How, Source};

    fn sugg(id: &str, items: Vec<Item>) -> Suggestion {
        Suggestion {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            category: String::new(),
            risk: Risk::Manual,
            risk_reason: String::new(),
            regenerates: None,
            docs: None,
            bytes: 0,
            exact: true,
            items,
            how: How::default(),
            blocked_by: Vec::new(),
            apps_to_close: Vec::new(),
            notes: Vec::new(),
            source: Source::Builtin,
            exec: Exec::None,
        }
    }

    fn ctx(home: &Path) -> Context {
        Context {
            home: home.to_owned(),
            env: Vec::new(),
            now: 0,
            processes: Vec::new(),
            path_dirs: Vec::new(),
        }
    }

    #[test]
    fn nvm_protects_the_default_version() {
        let tmp = tempfile::tempdir().unwrap();
        let h = tmp.path();
        std::fs::create_dir_all(h.join(".nvm/alias")).unwrap();
        std::fs::write(h.join(".nvm/alias/default"), "22\n").unwrap();
        let items = ["v18.20.0", "v22.14.0", "v22.22.2"]
            .iter()
            .map(|v| Item {
                path: h.join(".nvm/versions/node").join(v),
                ..Default::default()
            })
            .collect();
        let mut s = sugg("nvm-node-versions", items);
        enrich(&mut s, &ctx(h));
        let protected: Vec<_> = s
            .items
            .iter()
            .filter(|i| i.protected.is_some())
            .map(|i| name_of(&i.path))
            .collect();
        assert_eq!(protected, ["v22.22.2"]);
        assert_eq!(s.exec, Exec::RemoveItems);
    }

    #[test]
    fn rustup_protects_default_and_uses_uninstall() {
        let tmp = tempfile::tempdir().unwrap();
        let h = tmp.path();
        std::fs::create_dir_all(h.join(".rustup/toolchains")).unwrap();
        std::fs::write(
            h.join(".rustup/settings.toml"),
            "default_toolchain = \"stable-aarch64-apple-darwin\"\n",
        )
        .unwrap();
        let bin = h.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("rustup"), "").unwrap();
        let items = ["stable-aarch64-apple-darwin", "1.85-aarch64-apple-darwin"]
            .iter()
            .map(|v| Item {
                path: h.join(".rustup/toolchains").join(v),
                ..Default::default()
            })
            .collect();
        let mut s = sugg("rustup-toolchains", items);
        let mut c = ctx(h);
        c.path_dirs.push(bin);
        enrich(&mut s, &c);
        assert!(s.items[0].protected.is_some());
        assert!(s.items[1].protected.is_none());
        assert_eq!(
            s.items[1].argv.as_ref().unwrap()[1..],
            ["toolchain", "uninstall", "1.85-aarch64-apple-darwin"]
        );
    }

    #[test]
    fn huggingface_uses_hf_cache_rm() {
        let tmp = tempfile::tempdir().unwrap();
        let h = tmp.path();
        let bin = h.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("hf"), "").unwrap();
        let mut s = sugg(
            "huggingface-cache",
            vec![Item {
                path: h.join(".cache/huggingface/hub/models--Qwen--Qwen2.5-0.5B"),
                ..Default::default()
            }],
        );
        let mut c = ctx(h);
        c.path_dirs.push(bin);
        enrich(&mut s, &c);
        assert_eq!(
            s.items[0].argv.as_ref().unwrap()[1..],
            ["cache", "rm", "model/Qwen/Qwen2.5-0.5B", "--yes"]
        );
    }

    #[test]
    fn avds_take_their_ini_along() {
        let tmp = tempfile::tempdir().unwrap();
        let h = tmp.path();
        std::fs::create_dir_all(h.join(".android/avd/Pixel_8.avd")).unwrap();
        std::fs::write(h.join(".android/avd/Pixel_8.ini"), "").unwrap();
        let mut s = sugg(
            "android-avds",
            vec![Item {
                path: h.join(".android/avd/Pixel_8.avd"),
                ..Default::default()
            }],
        );
        enrich(&mut s, &ctx(h));
        assert_eq!(s.items[0].companions, [h.join(".android/avd/Pixel_8.ini")]);
    }
}
