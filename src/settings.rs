//! Shared settings.json under %LOCALAPPDATA%/argus-local (or platform data dir).
//! Holds GitHub shipping opt-in and local prompts opt-in. Never stores secrets.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SettingsFile {
    #[serde(default)]
    pub github: GitHubSettings,
    #[serde(default)]
    pub prompts: PromptsSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitHubSettings {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptsSettings {
    /// When true, Activity session detail may show on-device prompt/response text (local only).
    #[serde(default)]
    pub enabled: bool,
}

pub fn settings_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("argus-local")
        .join("settings.json")
}

pub fn settings_path_display() -> String {
    settings_path().display().to_string()
}

pub fn load() -> SettingsFile {
    let path = settings_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return SettingsFile::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save(settings: &SettingsFile) -> Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create settings dir")?;
    }
    let raw = serde_json::to_string_pretty(settings).context("serialize settings")?;
    std::fs::write(&path, raw).context("write settings.json")?;
    Ok(())
}

fn env_truthy(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let t = v.trim().to_lowercase();
            !(t.is_empty() || t == "0" || t == "false" || t == "no" || t == "off")
        }
        Err(_) => false,
    }
}

/// GitHub shipping opt-in (env or settings). Unchanged from ARG-32 semantics.
pub fn github_opted_in() -> bool {
    if std::env::var("ARGUS_GITHUB_TOKEN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    if env_truthy("ARGUS_GITHUB_ENABLED") {
        return true;
    }
    load().github.enabled
}

/// Local prompts opt-in. Default Off. Env ARGUS_PROMPTS_ENABLED overrides file when set.
pub fn prompts_enabled() -> bool {
    if let Ok(v) = std::env::var("ARGUS_PROMPTS_ENABLED") {
        let t = v.trim().to_lowercase();
        if !(t.is_empty()) {
            return !(t == "0" || t == "false" || t == "no" || t == "off");
        }
    }
    load().prompts.enabled
}

pub fn set_prompts_enabled(enabled: bool) -> Result<SettingsFile> {
    let mut s = load();
    s.prompts.enabled = enabled;
    save(&s)?;
    Ok(s)
}
