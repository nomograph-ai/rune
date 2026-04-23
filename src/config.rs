use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Global rune configuration (~/.config/rune/config.toml).
#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub registry: Vec<Registry>,
}

/// How a registry is fetched. `Git` clones and pulls; `Archive` downloads
/// a tarball with ETag caching — faster for readonly registries.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    #[default]
    Git,
    Archive,
}

impl std::fmt::Display for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Git => write!(f, "git"),
            Self::Archive => write!(f, "archive"),
        }
    }
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
    /// How to fetch. Archive mode is faster for readonly registries --
    /// downloads a tarball with ETag caching instead of cloning. Git is
    /// the default and supports `rune push` and `@version` pins.
    #[serde(default)]
    pub source: SourceKind,
    /// Environment variable containing a PAT for HTTPS authentication.
    /// Token is resolved at runtime and injected transiently -- never
    /// persisted in .git/config of cached clones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
    /// Git user.email for commits to this registry. Required for `rune push`.
    /// If unset, commits use whatever git defaults to (which may be wrong
    /// for cross-namespace registries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_email: Option<String>,
    /// Git user.name for commits to this registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_name: Option<String>,
}

impl Registry {
    /// Filesystem-safe form of the registry name. Used for lock files,
    /// cache directories, etag files, and anywhere else the name hits
    /// the filesystem. Necessary because registry names may include `/`
    /// (e.g. "andunn/arcana") which the filesystem treats as a path
    /// separator.
    ///
    /// Display uses `name` unchanged; only path construction sanitizes.
    pub fn fs_name(&self) -> String {
        self.name.replace('/', "--")
    }
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
    #[allow(dead_code)]
    pub fn resolve_skill(
        &self,
        skill_name: &str,
        cache_dir: &std::path::Path,
    ) -> Option<&Registry> {
        self.resolve_artifact(skill_name, cache_dir, crate::manifest::ArtifactType::Skill)
    }

    /// Resolve which registry has an item of the given type.
    /// Checks registries in declaration order, returns the first match.
    pub fn resolve_artifact(
        &self,
        name: &str,
        cache_dir: &std::path::Path,
        artifact_type: crate::manifest::ArtifactType,
    ) -> Option<&Registry> {
        for reg in &self.registry {
            let repo_dir = cache_dir.join(reg.fs_name());
            if !repo_dir.exists() {
                continue;
            }
            let path = crate::registry::artifact_path(&repo_dir, reg, name, artifact_type);
            if path.exists() {
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
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }
}
