use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Global rune configuration (~/.config/rune/config.toml).
#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub registry: Vec<Registry>,
}

/// A named git-based skill registry.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Registry {
    pub name: String,
    pub url: String,
    /// Optional subdirectory within the repo where skills live.
    #[serde(default)]
    pub path: Option<String>,
    /// Branch to track. Defaults to "main".
    #[serde(default = "default_branch")]
    pub branch: String,
}

fn default_branch() -> String {
    "main".to_string()
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            anyhow::bail!(
                "No config found at {}\nRun `rune setup` to create one.",
                path.display()
            );
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(config)
    }

    pub fn path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".config").join("rune").join("config.toml"))
    }

    pub fn config_dir() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".config").join("rune"))
    }

    /// Directory where registry clones are cached.
    pub fn cache_dir() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".cache").join("rune").join("registries"))
    }

    pub fn registry(&self, name: &str) -> Option<&Registry> {
        self.registry.iter().find(|r| r.name == name)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }
}
