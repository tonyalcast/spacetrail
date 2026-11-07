//! App settings. Preferences live in `<config>/app.json`; API keys in
//! `<config>/api-keys.json` with owner-only permissions.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use spacetrail_macos::keychain;

/// LLM providers the "Explain" button can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Provider {
    Anthropic,
    Openai,
    /// Any server speaking the OpenAI chat-completions API (OpenRouter, Ollama, LM Studio…).
    Compatible,
}

impl Provider {
    pub(crate) const ALL: [Self; 3] = [Self::Anthropic, Self::Openai, Self::Compatible];

    /// Key name in `api-keys.json` (also the legacy Keychain account).
    pub(crate) fn account(self) -> &'static str {
        match self {
            Self::Anthropic => "llm.anthropic",
            Self::Openai => "llm.openai",
            Self::Compatible => "llm.compatible",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::Openai => "OpenAI",
            Self::Compatible => "OpenAI-compatible",
        }
    }
}

/// Non-secret preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    /// Provider used by "Explain"; `None` until the user picks one.
    pub(crate) active: Option<Provider>,
    pub(crate) anthropic_model: String,
    pub(crate) openai_model: String,
    pub(crate) compatible_base_url: String,
    pub(crate) compatible_model: String,
    /// Language of the explanations (e.g. "Spanish", "English").
    pub(crate) language: String,
    /// Send the names of a folder's first items (never file contents).
    pub(crate) share_folder_listing: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            active: None,
            anthropic_model: "claude-opus-5-5".into(),
            openai_model: String::new(),
            compatible_base_url: "http://localhost:11434/v1".into(),
            compatible_model: String::new(),
            language: "Spanish".into(),
            share_folder_listing: true,
        }
    }
}

impl Settings {
    pub(crate) fn model(&self, p: Provider) -> &str {
        match p {
            Provider::Anthropic => &self.anthropic_model,
            Provider::Openai => &self.openai_model,
            Provider::Compatible => &self.compatible_model,
        }
    }
}

fn path() -> Option<PathBuf> {
    spacetrail_core::Dirs::from_env().map(|d| d.config.join("app.json"))
}

pub(crate) fn load() -> Settings {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub(crate) fn save(s: &Settings) -> Result<(), String> {
    let p = path().ok_or("HOME is not set")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(&p, text).map_err(|e| e.to_string())
}

/// What the UI may know: settings plus whether each key exists (never the key).
#[derive(Debug, Serialize)]
pub(crate) struct SettingsView {
    pub(crate) settings: Settings,
    pub(crate) has_key: Vec<(Provider, bool)>,
    /// Where the keys are stored, shown in Settings.
    pub(crate) keys_file: String,
}

pub(crate) fn view() -> SettingsView {
    let secrets = load_secrets();
    SettingsView {
        settings: load(),
        has_key: Provider::ALL
            .iter()
            .map(|p| (*p, secrets.keys.contains_key(p.account())))
            .collect(),
        keys_file: secrets_path().map(|p| tilde(&p)).unwrap_or_default(),
    }
}

// ───────────────────────── API keys ─────────────────────────
//
// Keys live in `<config>/api-keys.json` (mode 600, folder 700), like `gh` or
// the AWS CLI. The Keychain was dropped: its per-binary ACL made a
// self-signed app ask for the password after every rebuild.

/// Contents of `api-keys.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Secrets {
    #[serde(default)]
    keys: BTreeMap<String, String>,
    /// Set once keys from older versions were moved out of the Keychain.
    #[serde(default)]
    migrated_from_keychain: bool,
}

fn secrets_path() -> Option<PathBuf> {
    spacetrail_core::Dirs::from_env().map(|d| d.config.join("api-keys.json"))
}

fn tilde(p: &std::path::Path) -> String {
    match std::env::var_os("HOME") {
        Some(h) => crate::facts::tilde(p, std::path::Path::new(&h)),
        None => p.display().to_string(),
    }
}

fn load_secrets() -> Secrets {
    secrets_path()
        .map(|p| load_secrets_at(&p))
        .unwrap_or_default()
}

fn load_secrets_at(p: &std::path::Path) -> Secrets {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_secrets(s: &Secrets) -> Result<(), String> {
    save_secrets_at(&secrets_path().ok_or("HOME is not set")?, s)
}

/// Writes the file atomically with owner-only permissions.
fn save_secrets_at(p: &std::path::Path, s: &Secrets) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let dir = p.parent().ok_or("invalid config path")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())
}

pub(crate) fn key(p: Provider) -> Result<Option<String>, String> {
    Ok(load_secrets().keys.get(p.account()).cloned())
}

pub(crate) fn set_key(p: Provider, key: &str) -> Result<(), String> {
    let mut s = load_secrets();
    let key = key.trim();
    if key.is_empty() {
        s.keys.remove(p.account());
    } else {
        s.keys.insert(p.account().to_owned(), key.to_owned());
    }
    save_secrets(&s)
}

/// One-time move of keys saved by older versions in the Keychain. May show
/// one last Keychain prompt per stored key; never touches the Keychain again.
pub(crate) fn migrate_from_keychain() {
    let mut s = load_secrets();
    if s.migrated_from_keychain {
        return;
    }
    for p in Provider::ALL {
        if s.keys.contains_key(p.account()) {
            continue;
        }
        if let Ok(Some(k)) = keychain::get(p.account()) {
            s.keys.insert(p.account().to_owned(), k);
        }
    }
    s.migrated_from_keychain = true;
    if save_secrets(&s).is_ok() {
        for p in Provider::ALL {
            let _ = keychain::delete(p.account());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn keys_file_is_owner_only_and_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("cfg/api-keys.json");
        let mut s = Secrets::default();
        s.keys.insert("llm.anthropic".into(), "sk-ant-test".into());
        save_secrets_at(&p, &s).unwrap();
        let mode = |q: &std::path::Path| std::fs::metadata(q).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&p), 0o600);
        assert_eq!(mode(p.parent().unwrap()), 0o700);
        assert_eq!(
            load_secrets_at(&p)
                .keys
                .get("llm.anthropic")
                .map(String::as_str),
            Some("sk-ant-test")
        );
        assert!(
            load_secrets_at(&tmp.path().join("missing.json"))
                .keys
                .is_empty()
        );
    }
}
