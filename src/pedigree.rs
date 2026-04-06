use anyhow::{Context, Result};
use std::path::Path;

/// Pedigree metadata from SKILL.md frontmatter.
/// Tracks where a skill came from and whether it's been modified.
#[derive(Debug, Default)]
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
    /// Parse pedigree fields from a SKILL.md file's YAML frontmatter.
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
    fn parse_frontmatter(content: &str) -> Result<Self> {
        let content = content.trim();
        if !content.starts_with("---") {
            return Ok(Self::default());
        }

        let rest = &content[3..];
        let end = rest.find("---").unwrap_or(rest.len());
        let frontmatter = &rest[..end];

        let mut pedigree = Self::default();
        for line in frontmatter.lines() {
            let line = line.trim();
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
                        pedigree.upstream_commit = Some(value.to_string())
                    }
                    "modified" => pedigree.modified = Some(value == "true"),
                    _ => {}
                }
            }
        }

        Ok(pedigree)
    }

    /// Write pedigree fields into a SKILL.md file's frontmatter.
    /// Preserves existing fields and adds/updates pedigree fields.
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
            // No frontmatter -- add one
            let fm = self.to_frontmatter_string();
            return format!("---\n{fm}---\n\n{content}");
        }

        let rest = &content[3..];
        let end = rest.find("---").unwrap_or(rest.len());
        let existing_fm = &rest[..end];
        let body = &rest[end + 3..];

        // Parse existing fields, preserving order for non-pedigree fields
        let mut lines: Vec<String> = Vec::new();
        let pedigree_keys = [
            "origin",
            "origin_path",
            "imported",
            "upstream_commit",
            "modified",
        ];

        for line in existing_fm.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some((key, _)) = trimmed.split_once(':') {
                if !pedigree_keys.contains(&key.trim()) {
                    lines.push(line.to_string());
                }
            }
        }

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

    /// Format pedigree as frontmatter string (for new files).
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

    /// Has pedigree (was imported from upstream).
    pub fn has_origin(&self) -> bool {
        self.origin.is_some()
    }
}

/// Get the current HEAD commit short hash for a registry repo.
pub fn repo_head_short(repo_dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Get today's date as YYYY-MM-DD.
pub fn today() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = now / 86400;
    let years = 1970 + (days * 400 / 146097);
    // Approximate -- good enough for date stamps
    let remaining = days - ((years - 1970) * 365 + (years - 1969) / 4 - (years - 1901) / 100 + (years - 1601) / 400);
    let months = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 0;
    let mut day = remaining;
    for (i, &m) in months.iter().enumerate() {
        if day < m {
            month = i + 1;
            break;
        }
        day -= m;
    }
    if month == 0 {
        month = 12;
    }
    format!("{years}-{month:02}-{:02}", day + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pedigree_from_frontmatter() {
        let content = r#"---
name: scanpy
description: Single-cell RNA sequencing
origin: k-dense/claude-scientific-skills
origin_path: skills/scanpy
imported: 2026-04-06
upstream_commit: a1b2c3d
modified: false
---

# Scanpy
"#;
        let p = Pedigree::parse_frontmatter(content).unwrap();
        assert_eq!(p.name.as_deref(), Some("scanpy"));
        assert_eq!(p.origin.as_deref(), Some("k-dense/claude-scientific-skills"));
        assert_eq!(p.upstream_commit.as_deref(), Some("a1b2c3d"));
        assert_eq!(p.modified, Some(false));
    }

    #[test]
    fn parse_without_pedigree() {
        let content = r#"---
name: tidy
description: Ship-ready checklist
---

# Tidy
"#;
        let p = Pedigree::parse_frontmatter(content).unwrap();
        assert_eq!(p.name.as_deref(), Some("tidy"));
        assert!(!p.has_origin());
    }

    #[test]
    fn update_frontmatter_adds_pedigree() {
        let content = r#"---
name: scanpy
description: Single-cell RNA sequencing
---

# Scanpy
"#;
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
        assert!(updated.contains("name: scanpy"));
    }
}
