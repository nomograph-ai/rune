use anyhow::{Context, Result};
use std::path::Path;

/// Pedigree metadata from SKILL.md frontmatter.
/// Tracks where a skill came from and whether it's been modified.
#[derive(Debug, Default, Clone)]
pub struct Pedigree {
    pub name: Option<String>,
    pub description: Option<String>,
    pub origin: Option<String>,
    pub origin_path: Option<String>,
    pub imported: Option<String>,
    pub upstream_commit: Option<String>,
    pub modified: Option<bool>,
}

impl Pedigree {
    /// Parse pedigree fields from a skill path (file or directory).
    pub fn from_skill(path: &Path) -> Result<Self> {
        let skill_file = if path.is_dir() {
            path.join("SKILL.md")
        } else {
            path.to_path_buf()
        };

        if !skill_file.exists() {
            return Ok(Self::default());
        }

        let content =
            std::fs::read_to_string(&skill_file).context("Failed to read SKILL.md")?;

        Self::parse_frontmatter(&content)
    }

    /// Parse YAML frontmatter from markdown content.
    /// Handles the --- delimited block at the start of a markdown file.
    fn parse_frontmatter(content: &str) -> Result<Self> {
        let content = content.trim();
        if !content.starts_with("---") {
            return Ok(Self::default());
        }

        let rest = &content[3..];
        let end = match rest.find("\n---") {
            Some(pos) => pos,
            None => return Ok(Self::default()),
        };
        let frontmatter = &rest[..end];

        let mut pedigree = Self::default();
        for line in frontmatter.lines() {
            let line = line.trim();
            // Only split on first colon to handle descriptions with colons
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim();
                let value = value.trim().trim_matches('"').trim_matches('\'');
                match key {
                    "name" => pedigree.name = Some(value.to_string()),
                    "description" => pedigree.description = Some(value.to_string()),
                    "origin" => pedigree.origin = Some(value.to_string()),
                    "origin_path" => pedigree.origin_path = Some(value.to_string()),
                    "imported" => pedigree.imported = Some(value.to_string()),
                    "upstream_commit" => {
                        pedigree.upstream_commit = Some(value.to_string());
                    }
                    "modified" => pedigree.modified = Some(value == "true"),
                    _ => {}
                }
            }
        }

        Ok(pedigree)
    }

    /// Write pedigree fields into a SKILL.md file's frontmatter.
    /// Preserves existing non-pedigree fields and adds/updates pedigree fields.
    pub fn write_to_skill(&self, path: &Path) -> Result<()> {
        let skill_file = if path.is_dir() {
            path.join("SKILL.md")
        } else {
            path.to_path_buf()
        };

        let content = std::fs::read_to_string(&skill_file)
            .unwrap_or_else(|_| "---\n---\n".to_string());

        let updated = self.update_frontmatter(&content);
        std::fs::write(&skill_file, updated)
            .with_context(|| format!("Failed to write {}", skill_file.display()))?;

        Ok(())
    }

    /// Update frontmatter in markdown content with pedigree fields.
    fn update_frontmatter(&self, content: &str) -> String {
        let content = content.trim();

        if !content.starts_with("---") {
            let fm = self.to_frontmatter_string();
            return format!("---\n{fm}---\n\n{content}");
        }

        let rest = &content[3..];
        let end = match rest.find("\n---") {
            Some(pos) => pos,
            None => {
                let fm = self.to_frontmatter_string();
                return format!("---\n{fm}---\n\n{content}");
            }
        };
        let existing_fm = &rest[..end];
        let body = &rest[end + 4..]; // skip \n---

        let pedigree_keys = [
            "origin",
            "origin_path",
            "imported",
            "upstream_commit",
            "modified",
        ];

        // Keep existing non-pedigree fields
        let mut lines: Vec<String> = existing_fm
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return false;
                }
                match trimmed.split_once(':') {
                    Some((key, _)) => !pedigree_keys.contains(&key.trim()),
                    None => true,
                }
            })
            .map(|line| line.to_string())
            .collect();

        // Add pedigree fields
        if let Some(ref origin) = self.origin {
            lines.push(format!("origin: {origin}"));
        }
        if let Some(ref origin_path) = self.origin_path {
            lines.push(format!("origin_path: {origin_path}"));
        }
        if let Some(ref imported) = self.imported {
            lines.push(format!("imported: {imported}"));
        }
        if let Some(ref commit) = self.upstream_commit {
            lines.push(format!("upstream_commit: {commit}"));
        }
        if let Some(modified) = self.modified {
            lines.push(format!("modified: {modified}"));
        }

        format!("---\n{}\n---{body}", lines.join("\n"))
    }

    fn to_frontmatter_string(&self) -> String {
        let mut lines = Vec::new();
        if let Some(ref name) = self.name {
            lines.push(format!("name: {name}"));
        }
        if let Some(ref desc) = self.description {
            lines.push(format!("description: {desc}"));
        }
        if let Some(ref origin) = self.origin {
            lines.push(format!("origin: {origin}"));
        }
        if let Some(ref origin_path) = self.origin_path {
            lines.push(format!("origin_path: {origin_path}"));
        }
        if let Some(ref imported) = self.imported {
            lines.push(format!("imported: {imported}"));
        }
        if let Some(ref commit) = self.upstream_commit {
            lines.push(format!("upstream_commit: {commit}"));
        }
        if let Some(modified) = self.modified {
            lines.push(format!("modified: {modified}"));
        }
        if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        }
    }

    /// Whether this skill was imported from an upstream source.
    pub fn has_origin(&self) -> bool {
        self.origin.is_some()
    }
}

/// Extract owner/repo slug from a git URL.
/// Handles HTTPS, SSH, and git:// URLs.
///   https://github.com/owner/repo.git → owner/repo
///   git@github.com:owner/repo.git     → owner/repo
pub fn url_to_slug(url: &str) -> String {
    let url = url.trim_end_matches(".git");
    // Handle SSH URLs: git@host:owner/repo
    if let Some(path) = url.split_once(':').and_then(|(prefix, path)| {
        if prefix.contains('@') && !path.starts_with('/') {
            Some(path)
        } else {
            None
        }
    }) {
        return path.to_string();
    }
    // Handle HTTPS/git:// URLs: split on / and take last two segments
    let parts: Vec<&str> = url.split('/').collect();
    if parts.len() >= 2 {
        format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else {
        url.to_string()
    }
}

/// Get the current HEAD commit short hash for a git repo using git2.
pub fn repo_head_short(repo_dir: &Path) -> Option<String> {
    let repo = git2::Repository::open(repo_dir).ok()?;
    let head = repo.head().ok()?;
    let commit = head.peel_to_commit().ok()?;
    let id = commit.id();
    Some(format!("{:.7}", id))
}

/// Get today's date as YYYY-MM-DD. Pure Rust, no shell commands.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    epoch_to_date(secs)
}

/// Convert epoch seconds to YYYY-MM-DD. Handles leap years correctly.
fn epoch_to_date(epoch_secs: u64) -> String {
    let mut days = (epoch_secs / 86400) as i64;
    // Civil date from day count (algorithm from Howard Hinnant)
    days += 719468; // shift epoch from 1970-01-01 to 0000-03-01
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pedigree_from_frontmatter() {
        let content = "---\nname: scanpy\ndescription: Single-cell RNA sequencing\norigin: k-dense/claude-scientific-skills\norigin_path: skills/scanpy\nimported: 2026-04-06\nupstream_commit: a1b2c3d\nmodified: false\n---\n\n# Scanpy\n";
        let p = Pedigree::parse_frontmatter(content).unwrap();
        assert_eq!(p.name.as_deref(), Some("scanpy"));
        assert_eq!(
            p.origin.as_deref(),
            Some("k-dense/claude-scientific-skills")
        );
        assert_eq!(p.upstream_commit.as_deref(), Some("a1b2c3d"));
        assert_eq!(p.modified, Some(false));
    }

    #[test]
    fn parse_without_pedigree() {
        let content = "---\nname: tidy\ndescription: Ship-ready checklist\n---\n\n# Tidy\n";
        let p = Pedigree::parse_frontmatter(content).unwrap();
        assert_eq!(p.name.as_deref(), Some("tidy"));
        assert!(!p.has_origin());
    }

    #[test]
    fn parse_no_frontmatter() {
        let content = "# Just a heading\n\nSome content.";
        let p = Pedigree::parse_frontmatter(content).unwrap();
        assert!(p.name.is_none());
        assert!(!p.has_origin());
    }

    #[test]
    fn update_frontmatter_adds_pedigree() {
        let content = "---\nname: scanpy\ndescription: Single-cell RNA sequencing\n---\n\n# Scanpy\n";
        let p = Pedigree {
            origin: Some("k-dense".to_string()),
            imported: Some("2026-04-06".to_string()),
            upstream_commit: Some("abc123".to_string()),
            modified: Some(false),
            ..Default::default()
        };
        let updated = p.update_frontmatter(content);
        assert!(updated.contains("origin: k-dense"));
        assert!(updated.contains("imported: 2026-04-06"));
        assert!(updated.contains("upstream_commit: abc123"));
        assert!(updated.contains("modified: false"));
        assert!(updated.contains("name: scanpy"));
        // Body preserved
        assert!(updated.contains("# Scanpy"));
    }

    #[test]
    fn update_frontmatter_replaces_existing_pedigree() {
        let content = "---\nname: scanpy\norigin: old-source\nimported: 2025-01-01\nupstream_commit: old123\n---\n\n# Scanpy\n";
        let p = Pedigree {
            origin: Some("new-source".to_string()),
            imported: Some("2026-04-06".to_string()),
            upstream_commit: Some("new456".to_string()),
            modified: Some(false),
            ..Default::default()
        };
        let updated = p.update_frontmatter(content);
        assert!(updated.contains("origin: new-source"));
        assert!(updated.contains("upstream_commit: new456"));
        assert!(!updated.contains("old-source"));
        assert!(!updated.contains("old123"));
        // Non-pedigree fields preserved
        assert!(updated.contains("name: scanpy"));
    }

    #[test]
    fn url_to_slug_https() {
        assert_eq!(
            url_to_slug("https://github.com/K-Dense-AI/claude-scientific-skills.git"),
            "K-Dense-AI/claude-scientific-skills"
        );
    }

    #[test]
    fn url_to_slug_gitlab() {
        assert_eq!(
            url_to_slug("https://gitlab.com/dunn.dev/runes.git"),
            "dunn.dev/runes"
        );
    }

    #[test]
    fn url_to_slug_no_git_suffix() {
        assert_eq!(
            url_to_slug("https://github.com/anthropics/skills"),
            "anthropics/skills"
        );
    }

    #[test]
    fn url_to_slug_ssh() {
        assert_eq!(
            url_to_slug("git@github.com:K-Dense-AI/claude-scientific-skills.git"),
            "K-Dense-AI/claude-scientific-skills"
        );
    }

    #[test]
    fn url_to_slug_ssh_gitlab() {
        assert_eq!(
            url_to_slug("git@gitlab.com:dunn.dev/arcana.git"),
            "dunn.dev/arcana"
        );
    }

    #[test]
    fn today_returns_valid_date() {
        let date = today();
        assert_eq!(date.len(), 10); // YYYY-MM-DD
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[7..8], "-");
    }
}
