use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Per-project manifest (.claude/rune.toml).
/// Declares which skills this project uses and from which registry.
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct Manifest {
    /// Map of skill name -> registry name.
    #[serde(default)]
    pub skills: BTreeMap<String, String>,
}

impl Manifest {
    /// Load manifest from a project directory.
    pub fn load(project_dir: &Path) -> Result<Self> {
        let path = Self::path(project_dir);
        if !path.exists() {
            anyhow::bail!(
                "No rune.toml found at {}\nRun `rune init` to create one.",
                path.display()
            );
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let manifest: Manifest = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(manifest)
    }

    /// Try to load manifest, returning None if it doesn't exist.
    pub fn try_load(project_dir: &Path) -> Option<Self> {
        let path = Self::path(project_dir);
        let content = std::fs::read_to_string(path).ok()?;
        toml::from_str(&content).ok()
    }

    pub fn path(project_dir: &Path) -> PathBuf {
        project_dir.join(".claude").join("rune.toml")
    }

    /// Where skills live in a project.
    pub fn skills_dir(project_dir: &Path) -> PathBuf {
        project_dir.join(".claude").join("skills")
    }

    pub fn save(&self, project_dir: &Path) -> Result<()> {
        let path = Self::path(project_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .context("Failed to serialize manifest")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }
}
