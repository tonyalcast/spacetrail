//! App settings. Non-secret preferences live in `<config>/app.json`; API keys
//! live only in the login keychain.

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

    /// Keychain account name for the provider's key.
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
}

pub(crate) fn view() -> SettingsView {
    SettingsView {
        settings: load(),
        has_key: Provider::ALL
            .iter()
            .map(|p| (*p, keychain::get(p.account()).ok().flatten().is_some()))
            .collect(),
    }
}

pub(crate) fn key(p: Provider) -> Result<Option<String>, String> {
    keychain::get(p.account()).map_err(|e| e.to_string())
}

pub(crate) fn set_key(p: Provider, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        keychain::delete(p.account())
    } else {
        keychain::set(p.account(), key)
    }
    .map_err(|e| e.to_string())
}
