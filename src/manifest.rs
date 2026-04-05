use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Per-project manifest (.claude/rune.toml).
/// Declares which skills this project uses.
///
/// Skills can be declared as:
///   tidy = {}                          # resolved by registry priority
///   voice = { registry = "arcana" }    # pinned to specific registry
///   tidy = "runes"                     # shorthand pin (v0.1 compat)
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct Manifest {
    #[serde(default)]
    pub skills: BTreeMap<String, SkillEntry>,
}

/// A skill declaration -- either a pinned registry name or a config table.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    /// If set, pinned to this registry. If None, resolved by priority.
    pub registry: Option<String>,
}

// Custom serde: accept both "registry-name" (string) and { registry = "name" } (table)
impl<'de> Deserialize<'de> for SkillEntry {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Pinned(String),
            Table(Table),
        }

        #[derive(Deserialize)]
        struct Table {
            #[serde(default)]
            registry: Option<String>,
        }

        match Raw::deserialize(deserializer)? {
            Raw::Pinned(name) => Ok(SkillEntry {
                registry: Some(name),
            }),
            Raw::Table(t) => Ok(SkillEntry {
                registry: t.registry,
            }),
        }
    }
}

impl Serialize for SkillEntry {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match &self.registry {
            Some(name) => serializer.serialize_str(name),
            None => {
                use serde::ser::SerializeMap;
                let map = serializer.serialize_map(Some(0))?;
                map.end()
            }
        }
    }
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
