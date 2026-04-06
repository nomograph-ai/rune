use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Global rune configuration (~/.config/rune/config.toml).
#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub registry: Vec<Registry>,
}

/// A named skill registry. Can be git-based or HTTP archive.
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
    /// Read-only registries cannot be pushed to. Defaults to false.
    #[serde(default)]
    pub readonly: bool,
    /// How to fetch: "git" (default) or "archive" (download tarball).
    /// Archive mode is faster for readonly registries -- downloads a
    /// tarball instead of cloning. Supports GitHub and GitLab URLs.
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "git".to_string()
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

    /// Resolve which registry has a skill, checking in declaration order.
    /// Returns the first registry that contains the skill.
    pub fn resolve_skill(&self, skill_name: &str, cache_dir: &std::path::Path) -> Option<&Registry> {
        for reg in &self.registry {
            let repo_dir = cache_dir.join(&reg.name);
            if !repo_dir.exists() {
                continue;
            }
            let skill_path = crate::registry::skill_path(&repo_dir, reg, skill_name);
            if skill_path.exists() {
                return Some(reg);
            }
        }
        None
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
