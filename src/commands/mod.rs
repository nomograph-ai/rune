use anyhow::{Context, Result};

use crate::color;
use crate::config::Config;
use crate::manifest::{ArtifactType, SkillEntry};
use crate::registry;

mod check;
mod crud;
mod info;
mod sync;
mod upstream;

// Re-export all public functions so callers keep using `commands::foo`
pub use check::check;
pub use crud::{add_many, prune, push, remove};
pub use info::{audit, clean, doctor, ls, ls_registry, status};
pub use sync::sync;
pub use upstream::{browse, diff, import, update, upstream};

/// Status of an item relative to its registry.
#[derive(Debug)]
pub enum SkillStatus {
    Current,
    Drifted {
        direction: DriftDirection,
    },
    Missing, // in manifest but not on disk
    #[allow(dead_code)]
    Unregistered, // on disk but not in manifest
    RegistryMissing, // in manifest but item not found in any registry
}

#[derive(Debug)]
pub enum DriftDirection {
    LocalNewer,
    RegistryNewer,
    Diverged,
}

impl SkillStatus {
    /// Colored string representation.
    pub fn colored(&self) -> String {
        match self {
            Self::Current => color::green("CURRENT"),
            Self::Drifted { direction } => {
                let dir = match direction {
                    DriftDirection::LocalNewer => "local is newer",
                    DriftDirection::RegistryNewer => "registry is newer",
                    DriftDirection::Diverged => "diverged",
                };
                color::yellow(&format!("DRIFTED  {dir}"))
            }
            Self::Missing => color::red("MISSING"),
            Self::Unregistered => color::yellow("UNREGISTERED"),
            Self::RegistryMissing => color::red("REGISTRY MISSING"),
        }
    }
}

impl std::fmt::Display for SkillStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Current => write!(f, "CURRENT"),
            Self::Drifted { direction } => {
                let dir = match direction {
                    DriftDirection::LocalNewer => "local is newer",
                    DriftDirection::RegistryNewer => "registry is newer",
                    DriftDirection::Diverged => "diverged",
                };
                write!(f, "DRIFTED  {dir}")
            }
            Self::Missing => write!(f, "MISSING"),
            Self::Unregistered => write!(f, "UNREGISTERED"),
            Self::RegistryMissing => write!(f, "REGISTRY MISSING"),
        }
    }
}

/// Resolve which registry to use for an item of a given type.
fn resolve_registry_typed<'a>(
    name: &str,
    entry: &SkillEntry,
    config: &'a Config,
    artifact_type: ArtifactType,
) -> Result<&'a crate::config::Registry> {
    if let Some(ref pinned) = entry.registry {
        config
            .registry(pinned)
            .with_context(|| format!("Unknown registry: {pinned}"))
    } else {
        let cache_dir = Config::cache_dir()?;
        // Ensure all registries are cloned so we can search them
        for reg in &config.registry {
            let _ = registry::ensure_registry(reg);
        }
        config
            .resolve_artifact(name, &cache_dir, artifact_type)
            .with_context(|| format!("{name}: not found in any registry"))
    }
}
