use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The three types of items rune can manage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtifactType {
    Skill,
    Agent,
    Rule,
}

/// All supported types, in display order.
pub const ALL_TYPES: [ArtifactType; 3] =
    [ArtifactType::Skill, ArtifactType::Agent, ArtifactType::Rule];

impl ArtifactType {
    /// TOML section name and manifest key.
    pub fn section(self) -> &'static str {
        match self {
            Self::Skill => "skills",
            Self::Agent => "agents",
            Self::Rule => "rules",
        }
    }

    /// Default installation directory relative to project root.
    pub fn default_dir(self) -> &'static str {
        match self {
            Self::Skill => ".claude/skills",
            Self::Agent => ".claude/agents",
            Self::Rule => ".claude/rules",
        }
    }

    /// Whether items of this type are stored as directories (with SKILL.md).
    /// Only skills use directory format; agents and rules are single files.
    pub fn is_directory_type(self) -> bool {
        matches!(self, Self::Skill)
    }

    /// Singular display name.
    pub fn singular(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Agent => "agent",
            Self::Rule => "rule",
        }
    }

    /// Parse from a string (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "skill" | "skills" => Some(Self::Skill),
            "agent" | "agents" => Some(Self::Agent),
            "rule" | "rules" => Some(Self::Rule),
            _ => None,
        }
    }
}

impl std::fmt::Display for ArtifactType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.singular())
    }
}

/// Per-project manifest (.claude/rune.toml).
/// Declares which skills, agents, and rules this project uses.
///
/// Items can be declared as:
///   tidy = {}                                    # resolved by registry priority
///   voice = { registry = "andunn/arcana" }       # pinned to specific registry
///   tidy = "andrewdunndev/arcana"                # shorthand pin, track main
///   voice = "andunn/arcana@v1.2.0"               # shorthand pin at tag
///   voice = { registry = "andunn/arcana", version = "v1.2.0" }   # explicit form
///
/// `version` can be any git ref — a tag, a branch, or a commit hash. When
/// omitted the skill tracks the registry's configured branch (main by
/// default) exactly as before.
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct Manifest {
    #[serde(default)]
    pub skills: BTreeMap<String, SkillEntry>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, SkillEntry>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, SkillEntry>,

    /// Optional path overrides per type. Keys are type names (skills, agents, rules).
    /// Values are paths relative to the project root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<BTreeMap<String, String>>,
}

/// A skill/agent/rule declaration -- either a pinned registry name or a config table.
/// Name kept as SkillEntry for serde backward compatibility.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    /// If set, pinned to this registry. If None, resolved by priority.
    pub registry: Option<String>,
    /// Optional git ref to pin this item at (tag, branch, or commit hash).
    /// If None, tracks the registry's configured branch (default `main`).
    /// Not supported for archive-type registries — will error at sync time.
    pub version: Option<String>,
}

// Custom serde: accept both "registry-name" (string, optionally with `@version`
// suffix) and { registry = "name", version = "..." } (table).
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
            #[serde(default)]
            version: Option<String>,
        }

        match Raw::deserialize(deserializer)? {
            Raw::Pinned(name) => {
                // Parse `<registry>@<version>` shorthand.
                let (reg, ver) = match name.rsplit_once('@') {
                    Some((r, v)) if !r.is_empty() && !v.is_empty() => {
                        (r.to_string(), Some(v.to_string()))
                    }
                    _ => (name, None),
                };
                Ok(SkillEntry {
                    registry: Some(reg),
                    version: ver,
                })
            }
            Raw::Table(t) => Ok(SkillEntry {
                registry: t.registry,
                version: t.version,
            }),
        }
    }
}

impl Serialize for SkillEntry {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match (&self.registry, &self.version) {
            // Shorthand "name" or "name@version" when no other fields set.
            (Some(name), None) => serializer.serialize_str(name),
            (Some(name), Some(ver)) => serializer.serialize_str(&format!("{name}@{ver}")),
            (None, _) => {
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

    /// Where skills live in a project (convenience alias for lib consumers).
    #[allow(dead_code)]
    pub fn skills_dir(project_dir: &Path) -> PathBuf {
        project_dir.join(".claude").join("skills")
    }

    pub fn save(&self, project_dir: &Path) -> Result<()> {
        let path = Self::path(project_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).context("Failed to serialize manifest")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }

    /// Get the section map for a given type (immutable).
    pub fn section(&self, artifact_type: ArtifactType) -> &BTreeMap<String, SkillEntry> {
        match artifact_type {
            ArtifactType::Skill => &self.skills,
            ArtifactType::Agent => &self.agents,
            ArtifactType::Rule => &self.rules,
        }
    }

    /// Get the section map for a given type (mutable).
    pub fn section_mut(
        &mut self,
        artifact_type: ArtifactType,
    ) -> &mut BTreeMap<String, SkillEntry> {
        match artifact_type {
            ArtifactType::Skill => &mut self.skills,
            ArtifactType::Agent => &mut self.agents,
            ArtifactType::Rule => &mut self.rules,
        }
    }

    /// Iterate all items across all types.
    #[allow(dead_code)]
    pub fn all_items(&self) -> Vec<(ArtifactType, &str, &SkillEntry)> {
        let mut items = Vec::new();
        for at in ALL_TYPES {
            for (name, entry) in self.section(at) {
                items.push((at, name.as_str(), entry));
            }
        }
        items
    }

    /// Total count of items across all types.
    pub fn total_count(&self) -> usize {
        self.skills.len() + self.agents.len() + self.rules.len()
    }

    /// Find which type a name belongs to. Searches all sections.
    /// If the name appears in multiple sections, returns the first match
    /// and prints a warning.
    pub fn find_type(&self, name: &str) -> Option<ArtifactType> {
        let mut found: Vec<ArtifactType> = Vec::new();
        for at in ALL_TYPES {
            if self.section(at).contains_key(name) {
                found.push(at);
            }
        }
        if found.len() > 1 {
            eprintln!(
                "  warning: {name} appears in multiple sections: {}",
                found
                    .iter()
                    .map(|t| t.section())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        found.into_iter().next()
    }

    /// Get the installation directory for a type, respecting [paths] overrides.
    pub fn artifact_dir(&self, project_dir: &Path, artifact_type: ArtifactType) -> PathBuf {
        if let Some(ref paths) = self.paths
            && let Some(custom) = paths.get(artifact_type.section())
        {
            return project_dir.join(custom);
        }
        project_dir.join(artifact_type.default_dir())
    }
}
