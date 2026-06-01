//! Global CLI configuration stored at `~/.ldgr/config.json`.
//!
//! Separated from per-vault settings — theme preferences are personal,
//! not tied to any specific vault.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::session;

const CONFIG_FILE: &str = "config.json";

/// Top-level CLI configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    /// Active theme name (built-in or custom).
    #[serde(default = "default_theme")]
    pub theme: String,

    /// User-defined custom themes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub custom_themes: BTreeMap<String, CustomThemeColors>,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            custom_themes: BTreeMap::new(),
        }
    }
}

fn default_theme() -> String {
    "default".to_string()
}

/// Custom theme definition with optional color overrides.
/// Missing fields inherit from the base theme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomThemeColors {
    /// Built-in theme to inherit unset colors from.
    #[serde(default = "default_theme")]
    pub base: String,

    pub accent: Option<String>,
    pub positive: Option<String>,
    pub negative: Option<String>,
    pub warning: Option<String>,
    pub info: Option<String>,
    pub muted: Option<String>,
    pub header: Option<String>,
    pub chart_line: Option<String>,
    pub chart_ma: Option<String>,
}

/// Path to the global config file.
pub fn config_path() -> PathBuf {
    session::default_vault_dir().join(CONFIG_FILE)
}

/// Load config, returning defaults if file is missing or malformed.
pub fn load_config() -> CliConfig {
    load_config_from(&config_path())
}

/// Load config from a specific path.
pub fn load_config_from(path: &Path) -> CliConfig {
    let Ok(json) = fs::read_to_string(path) else {
        return CliConfig::default();
    };
    serde_json::from_str(&json).unwrap_or_else(|e| {
        eprintln!("Warning: malformed config at {}: {e}", path.display());
        CliConfig::default()
    })
}

/// Save config atomically (write to temp, then rename).
pub fn save_config(config: &CliConfig) -> Result<()> {
    save_config_to(config, &config_path())
}

/// Save config to a specific path.
pub fn save_config_to(config: &CliConfig, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let json = serde_json::to_string_pretty(config).context("failed to serialize config")?;

    // Atomic-ish write: temp file in same directory, then rename
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &json).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("failed to rename config into {}", path.display()))?;

    Ok(())
}

/// Get the modification time of the config file (for live-reload polling).
pub fn config_mtime() -> Option<SystemTime> {
    fs::metadata(config_path()).ok()?.modified().ok()
}
