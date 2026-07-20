use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoConfirmLevel {
    #[default]
    High,
    Medium,
    Low,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannersConfig {
    #[serde(default = "default_true")]
    pub xdg_db: bool,
    #[serde(default = "default_true")]
    pub name_heuristic: bool,
    #[serde(default)]
    pub pacman_orphan: bool,
}

impl Default for ScannersConfig {
    fn default() -> Self {
        Self {
            xdg_db: true,
            name_heuristic: true,
            pacman_orphan: false,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IgnoreConfig {
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // config structs legitimately have many bool fields
pub struct Config {
    #[serde(default)]
    pub auto_confirm: AutoConfirmLevel,
    #[serde(default)]
    pub auto_remove_orphan_deps: bool,
    #[serde(default = "default_true")]
    pub use_trash: bool,
    #[serde(default = "default_true")]
    pub hook_enabled: bool,
    /// Let the hook prompt on the controlling terminal instead of silently
    /// auto-confirming. Set false for unattended machines, where a prompt
    /// would block the pacman transaction with nobody there to answer.
    #[serde(default = "default_true")]
    pub hook_prompt: bool,
    #[serde(default)]
    pub scan_paths_extra: Vec<PathBuf>,
    #[serde(default)]
    pub scanners: ScannersConfig,
    #[serde(default)]
    pub ignore: IgnoreConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto_confirm: AutoConfirmLevel::default(),
            auto_remove_orphan_deps: false,
            use_trash: true,
            hook_enabled: true,
            hook_prompt: true,
            scan_paths_extra: Vec::new(),
            scanners: ScannersConfig::default(),
            ignore: IgnoreConfig::default(),
        }
    }
}

fn load_from_path(path: &Path) -> anyhow::Result<Config> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("reading config {}", path.display()))?;
    toml::from_str(&contents).with_context(|| format!("parsing config {}", path.display()))
}

fn merge(_base: Config, overlay: Config) -> Config {
    // User config wins on every field — overlay replaces base entirely.
    overlay
}

/// Load config: system `/etc/pacrid/config.toml` as base, user config as overlay.
pub fn load() -> Config {
    let system = Path::new("/etc/pacrid/config.toml");
    let system_cfg = if system.exists() {
        load_from_path(system).unwrap_or_else(|e| {
            tracing::warn!("failed to load system config: {e}");
            Config::default()
        })
    } else {
        Config::default()
    };

    let user_cfg_path = directories::ProjectDirs::from("", "", "pacrid")
        .map(|d| d.config_dir().join("config.toml"));

    let user_cfg = user_cfg_path
        .as_deref()
        .filter(|p| p.exists())
        .and_then(|p| {
            load_from_path(p)
                .map_err(|e| {
                    tracing::warn!("failed to load user config: {e}");
                })
                .ok()
        });

    match user_cfg {
        Some(u) => merge(system_cfg, u),
        None => system_cfg,
    }
}
