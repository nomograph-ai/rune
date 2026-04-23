use anyhow::{Context, Result};

use crate::color;
use crate::config::Config;
use crate::manifest::{ArtifactType, SkillEntry};
use crate::registry;

mod audit;
mod check;
mod clean;
mod crud;
mod doctor;
mod ls;
mod status;
mod sync;
mod upstream;

// Re-export all public functions so callers keep using `commands::foo`
pub use audit::audit;
pub use check::check;
pub use clean::clean;
pub use crud::{add_many, prune, push, remove};
pub use doctor::doctor;
pub use ls::{ls, ls_registry};
pub use status::status;
pub use sync::sync;
pub use upstream::{browse, diff, import, update, upstream};

/// Status of an item relative to its registry.
#[derive(Debug)]
pub enum SkillStatus {
    Current,
    Drifted { direction: DriftDirection },
    Missing,         // in manifest but not on disk
    RegistryMissing, // in manifest but item not found in any registry
}

#[derive(Debug)]
pub enum DriftDirection {
    LocalNewer,
    RegistryNewer,
    Diverged,
}

impl SkillStatus {
    /// Plain label for the status (used by Display and colored).
    fn label(&self) -> String {
        match self {
            Self::Current => "CURRENT".to_string(),
            Self::Drifted { direction } => {
                let dir = match direction {
                    DriftDirection::LocalNewer => "local is newer",
                    DriftDirection::RegistryNewer => "registry is newer",
                    DriftDirection::Diverged => "diverged",
                };
                format!("DRIFTED  {dir}")
            }
            Self::Missing => "MISSING".to_string(),
            Self::RegistryMissing => "REGISTRY MISSING".to_string(),
        }
    }

    /// Colored string representation.
    pub fn colored(&self) -> String {
        let label = self.label();
        match self {
            Self::Current => color::green(&label),
            Self::Drifted { .. } => color::yellow(&label),
            Self::Missing | Self::RegistryMissing => color::red(&label),
        }
    }

    /// Prescriptive next action for a non-current status.
    pub fn hint(&self, name: &str) -> Option<String> {
        match self {
            Self::Current => None,
            Self::Drifted { direction } => Some(match direction {
                DriftDirection::LocalNewer => format!("→ run: rune push {name}"),
                DriftDirection::RegistryNewer => {
                    format!("→ run: rune sync  (or `rune sync --force {name}` to discard local)")
                }
                DriftDirection::Diverged => format!(
                    "→ run: rune diff {name}  (inspect), then `rune push {name}` or `rune sync --force`"
                ),
            }),
            Self::Missing => Some(format!("→ run: rune sync  (pulls {name} from registry)")),
            Self::RegistryMissing => Some(format!(
                "→ check config: registry for {name} is configured but the item is not in the cached tree; run `rune doctor`"
            )),
        }
    }
}

impl std::fmt::Display for SkillStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
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
