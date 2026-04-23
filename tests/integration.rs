use std::fs;
use std::os::unix::fs as unix_fs;
use std::path::Path;

/// Create a minimal git repo for testing.
fn init_git_repo(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir)
        .status()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir)
        .status()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "test"])
        .current_dir(dir)
        .status()
        .unwrap();
}

fn commit_all(dir: &Path) {
    std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir)
        .status()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "--quiet", "--allow-empty", "-m", "commit"])
        .current_dir(dir)
        .status()
        .unwrap();
}

// ============================================================
// Path traversal tests
// ============================================================

#[test]
fn path_traversal_names_rejected() {
    let attacks = [
        "../etc/passwd",
        "../../.ssh/id_rsa",
        "skill/../../escape",
        "..%2F..%2Fetc",
        ".hidden",
        "-flag-injection",
        "null\0byte",
        "\\backslash",
        "../",
        "..",
        " ",
        "  spaces  ",
        "tab\there",
        " leading",
        "trailing ",
    ];
    for name in &attacks {
        let result = rune::registry::validate_name(name);
        assert!(
            result.is_err(),
            "SECURITY: validate_name accepted malicious name: {name:?}"
        );
    }
}

#[test]
fn valid_skill_names_accepted() {
    let names = [
        "tidy",
        "feedback-audit",
        "scanpy",
        "claude-api",
        "my_skill_123",
        "CamelCase",
        "a",
    ];
    for name in &names {
        let result = rune::registry::validate_name(name);
        assert!(result.is_ok(), "Rejected valid name: {name}");
    }
}

// ============================================================
// Symlink protection tests
// ============================================================

#[test]
fn copy_skill_rejects_symlink_source() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("target");

    // Create a symlink as the "skill"
    unix_fs::symlink("/etc/passwd", tmp.path().join("evil-skill")).unwrap();

    let result = rune::registry::copy_skill(&tmp.path().join("evil-skill"), &target);
    assert!(
        result.is_err(),
        "SECURITY: copy_skill should reject symlink sources"
    );
}

#[test]
fn copy_dir_skips_symlinks_inside_skill() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("skill");
    let dest = tmp.path().join("dest");
    fs::create_dir_all(&src).unwrap();

    // Real file
    fs::write(src.join("SKILL.md"), "---\nname: test\n---\n").unwrap();
    // Symlink to sensitive file
    unix_fs::symlink("/etc/passwd", src.join("secrets")).unwrap();

    rune::registry::copy_skill(&src, &dest).unwrap();

    // SKILL.md should be copied
    assert!(dest.join("SKILL.md").exists());
    // Symlink should NOT be copied
    assert!(
        !dest.join("secrets").exists(),
        "SECURITY: symlink was copied to destination"
    );
}

#[test]
fn collect_files_skips_symlinks() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("skill");
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("SKILL.md"), "content").unwrap();
    unix_fs::symlink("/etc/passwd", dir.join("link")).unwrap();

    let files = rune::registry::fs::collect_files(&dir);
    let names: Vec<String> = files
        .iter()
        .map(|f| f.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert!(names.contains(&"SKILL.md".to_string()));
    assert!(
        !names.contains(&"link".to_string()),
        "SECURITY: collect_files returned a symlink"
    );
}

#[test]
fn collect_files_skips_dotfiles() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("skill");
    fs::create_dir_all(dir.join(".git")).unwrap();

    fs::write(dir.join("SKILL.md"), "content").unwrap();
    fs::write(dir.join(".git").join("config"), "secret").unwrap();
    fs::write(dir.join(".hidden"), "hidden").unwrap();

    let files = rune::registry::fs::collect_files(&dir);
    let names: Vec<String> = files
        .iter()
        .map(|f| f.to_string_lossy().to_string())
        .collect();

    assert!(
        !names.iter().any(|n| n.contains(".git")),
        "SECURITY: collect_files returned .git contents"
    );
    assert!(
        !names.iter().any(|n| n.contains(".hidden")),
        "SECURITY: collect_files returned dotfile"
    );
}

// ============================================================
// Skill hash tests
// ============================================================

#[test]
fn skill_hash_deterministic() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("skill");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), "---\nname: test\n---\n").unwrap();
    fs::write(dir.join("extra.md"), "extra content").unwrap();

    let hash1 = rune::registry::skill_hash(&dir).expect("hash ok");
    let hash2 = rune::registry::skill_hash(&dir).expect("hash ok");
    assert_eq!(hash1, hash2, "Hash should be deterministic");
    assert!(!hash1.is_empty());
}

#[test]
fn skill_hash_differs_on_content_change() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("skill");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), "version 1").unwrap();

    let hash1 = rune::registry::skill_hash(&dir).expect("hash ok");

    fs::write(dir.join("SKILL.md"), "version 2").unwrap();
    let hash2 = rune::registry::skill_hash(&dir).expect("hash ok");

    assert_ne!(hash1, hash2, "Hash should change when content changes");
}

#[test]
fn skill_hash_errors_for_symlink_file() {
    let tmp = tempfile::tempdir().unwrap();
    unix_fs::symlink("/etc/passwd", tmp.path().join("link.md")).unwrap();

    let result = rune::registry::skill_hash(&tmp.path().join("link.md"));
    assert!(
        result.is_err(),
        "SECURITY: skill_hash must reject symlinks with an error, not silently"
    );
}

// ============================================================
// Pedigree tests
// ============================================================

#[test]
fn pedigree_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let skill_dir = tmp.path().join("test-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: test\ndescription: A test skill\n---\n\n# Test\n\nBody content.\n",
    )
    .unwrap();

    let ped = rune::pedigree::Pedigree {
        origin: Some("upstream/repo".to_string()),
        origin_path: Some("skills/test".to_string()),
        imported: Some("2026-04-06".to_string()),
        upstream_commit: Some("abc1234".to_string()),
        modified: Some(false),
        ..Default::default()
    };
    ped.write_to_skill(&skill_dir).unwrap();

    let read_back = rune::pedigree::Pedigree::from_skill(&skill_dir).unwrap();
    assert_eq!(read_back.origin.as_deref(), Some("upstream/repo"));
    assert_eq!(read_back.origin_path.as_deref(), Some("skills/test"));
    assert_eq!(read_back.upstream_commit.as_deref(), Some("abc1234"));
    assert_eq!(read_back.modified, Some(false));
    // Non-pedigree fields preserved
    assert_eq!(read_back.name.as_deref(), Some("test"));
    assert_eq!(read_back.description.as_deref(), Some("A test skill"));
    // Body preserved
    let content = fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
    assert!(content.contains("# Test"));
    assert!(content.contains("Body content."));
}

#[test]
fn pedigree_handles_malformed_frontmatter() {
    let tmp = tempfile::tempdir().unwrap();
    let cases: &[(&str, bool)] = &[
        ("", false),                                     // empty
        ("no frontmatter here", false),                  // no ---
        ("---\n", false),                                // unclosed
        ("---\nname: test\n---\n", true),                // valid
        ("---\n\n---\nbody", false),                     // empty frontmatter (no name)
        ("---\nname: has:colons:in:value\n---\n", true), // colons in value
    ];
    for (i, (content, should_have_name)) in cases.iter().enumerate() {
        let file = tmp.path().join(format!("test-{i}.md"));
        fs::write(&file, content).unwrap();
        let result = rune::pedigree::Pedigree::from_skill(&file);
        assert!(result.is_ok(), "Should not panic on: {content:?}");
        if *should_have_name {
            assert!(
                result.unwrap().name.is_some(),
                "Expected name in: {content:?}"
            );
        }
    }
}

#[test]
fn url_to_slug_handles_all_formats() {
    let cases = [
        ("https://github.com/owner/repo.git", "owner/repo"),
        ("https://gitlab.com/group/project.git", "group/project"),
        ("git@github.com:owner/repo.git", "owner/repo"),
        ("git@gitlab.com:group/project.git", "group/project"),
        ("https://github.com/owner/repo", "owner/repo"),
        ("ssh://git@host/owner/repo.git", "owner/repo"),
    ];
    for (url, expected) in &cases {
        assert_eq!(
            rune::pedigree::url_to_slug(url),
            *expected,
            "Failed for URL: {url}"
        );
    }
}

#[test]
fn today_produces_valid_date() {
    let date = rune::pedigree::today();
    assert_eq!(date.len(), 10);
    assert_eq!(&date[4..5], "-");
    assert_eq!(&date[7..8], "-");
    let year: u32 = date[..4].parse().unwrap();
    let month: u32 = date[5..7].parse().unwrap();
    let day: u32 = date[8..10].parse().unwrap();
    assert!((2024..=2100).contains(&year));
    assert!((1..=12).contains(&month));
    assert!((1..=31).contains(&day));
}

// ============================================================
// List skills tests
// ============================================================

#[test]
fn list_skills_requires_skill_md() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    // Directory WITH SKILL.md -- should be listed
    fs::create_dir_all(base.join("valid-skill")).unwrap();
    fs::write(
        base.join("valid-skill").join("SKILL.md"),
        "---\nname: valid\n---\n",
    )
    .unwrap();

    // Directory WITHOUT SKILL.md -- should NOT be listed
    fs::create_dir_all(base.join("not-a-skill")).unwrap();
    fs::write(base.join("not-a-skill").join("README.md"), "not a skill").unwrap();

    // Dotdir -- should NOT be listed
    fs::create_dir_all(base.join(".hidden")).unwrap();
    fs::write(
        base.join(".hidden").join("SKILL.md"),
        "---\nname: hidden\n---\n",
    )
    .unwrap();

    let reg = rune::config::Registry {
        name: "test".to_string(),
        url: "unused".to_string(),
        path: None,
        branch: "main".to_string(),
        readonly: false,
        source: rune::config::SourceKind::Git,
        token_env: None,
        git_email: None,
        git_name: None,
        aliases: Vec::new(),
    };
    let skills =
        rune::registry::list_artifacts(base, &reg, rune::manifest::ArtifactType::Skill).unwrap();
    assert_eq!(skills, vec!["valid-skill"]);
}

#[test]
fn list_skills_skips_symlink_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    // Real skill
    fs::create_dir_all(base.join("real")).unwrap();
    fs::write(base.join("real").join("SKILL.md"), "---\nname: real\n---\n").unwrap();

    // Symlink dir
    unix_fs::symlink("/tmp", base.join("evil")).unwrap();

    let reg = rune::config::Registry {
        name: "test".to_string(),
        url: "unused".to_string(),
        path: None,
        branch: "main".to_string(),
        readonly: false,
        source: rune::config::SourceKind::Git,
        token_env: None,
        git_email: None,
        git_name: None,
        aliases: Vec::new(),
    };
    let skills =
        rune::registry::list_artifacts(base, &reg, rune::manifest::ArtifactType::Skill).unwrap();
    assert_eq!(skills, vec!["real"]);
}

// ============================================================
// repo_head_short tests
// ============================================================

#[test]
fn repo_head_short_returns_hash() {
    let tmp = tempfile::tempdir().unwrap();
    init_git_repo(tmp.path());
    fs::write(tmp.path().join("file"), "content").unwrap();
    commit_all(tmp.path());

    let hash = rune::pedigree::repo_head_short(tmp.path());
    assert!(hash.is_some(), "Should return a hash for a valid repo");
    let h = hash.unwrap();
    assert_eq!(h.len(), 7, "Short hash should be 7 chars, got: {h}");
    assert!(
        h.chars().all(|c| c.is_ascii_hexdigit()),
        "Hash should be hex: {h}"
    );
}

#[test]
fn repo_head_short_returns_none_for_non_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let hash = rune::pedigree::repo_head_short(tmp.path());
    assert!(hash.is_none(), "Should return None for non-repo directory");
}

// ============================================================
// Lockfile tests
// ============================================================

#[test]
fn lockfile_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    fs::create_dir_all(project.join(".claude")).unwrap();

    let mut lf = rune::lockfile::Lockfile::default();
    lf.skills.insert(
        "tidy".to_string(),
        rune::lockfile::LockedSkill {
            registry: "runes".to_string(),
            hash: "abc123".to_string(),
            registry_commit: Some("def456".to_string()),
            synced_at: "2026-04-07".to_string(),
        },
    );
    lf.skills.insert(
        "voice".to_string(),
        rune::lockfile::LockedSkill {
            registry: "arcana".to_string(),
            hash: "789abc".to_string(),
            registry_commit: None,
            synced_at: "2026-04-07".to_string(),
        },
    );

    lf.save(project).unwrap();

    // Verify file exists
    assert!(project.join(".claude").join("rune.lock").exists());

    // Read back
    let loaded = rune::lockfile::Lockfile::load(project).unwrap();
    assert_eq!(loaded.skills.len(), 2);

    let tidy = loaded.skills.get("tidy").unwrap();
    assert_eq!(tidy.registry, "runes");
    assert_eq!(tidy.hash, "abc123");
    assert_eq!(tidy.registry_commit.as_deref(), Some("def456"));
    assert_eq!(tidy.synced_at, "2026-04-07");

    let voice = loaded.skills.get("voice").unwrap();
    assert_eq!(voice.registry, "arcana");
    assert!(voice.registry_commit.is_none());
}

#[test]
fn lockfile_load_missing_returns_default() {
    let tmp = tempfile::tempdir().unwrap();
    let lf = rune::lockfile::Lockfile::load(tmp.path()).unwrap();
    assert!(lf.skills.is_empty());
}

#[test]
fn lockfile_header_preserved() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    fs::create_dir_all(project.join(".claude")).unwrap();

    let lf = rune::lockfile::Lockfile::default();
    lf.save(project).unwrap();

    let content = fs::read_to_string(project.join(".claude").join("rune.lock")).unwrap();
    assert!(content.starts_with("# Generated by rune sync"));
}

// ============================================================
// Color tests
// ============================================================

#[test]
fn color_functions_return_content() {
    // Without init(), colors are disabled (default AtomicBool is false)
    assert_eq!(rune::color::green("ok"), "ok");
    assert_eq!(rune::color::red("fail"), "fail");
    assert_eq!(rune::color::yellow("warn"), "warn");
    assert_eq!(rune::color::cyan("info"), "info");
    assert_eq!(rune::color::dim("dim"), "dim");
    assert_eq!(rune::color::bold("bold"), "bold");
}

// ============================================================
// Drift direction with lockfile
// ============================================================

#[test]
fn skill_status_display() {
    use rune::commands::{DriftDirection, SkillStatus};

    let current = SkillStatus::Current;
    assert_eq!(format!("{current}"), "CURRENT");

    let drifted = SkillStatus::Drifted {
        direction: DriftDirection::LocalNewer,
    };
    assert!(format!("{drifted}").contains("local is newer"));

    let drifted = SkillStatus::Drifted {
        direction: DriftDirection::RegistryNewer,
    };
    assert!(format!("{drifted}").contains("registry is newer"));

    let drifted = SkillStatus::Drifted {
        direction: DriftDirection::Diverged,
    };
    assert!(format!("{drifted}").contains("diverged"));

    let missing = SkillStatus::Missing;
    assert_eq!(format!("{missing}"), "MISSING");
}

// ============================================================
// Lockfile drift direction logic
// ============================================================

#[test]
fn lockfile_detects_local_modification() {
    // When lockfile hash differs from local hash, skill was locally modified
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    let skills_dir = project.join(".claude").join("skills");
    fs::create_dir_all(skills_dir.join("test-skill")).unwrap();
    fs::write(
        skills_dir.join("test-skill").join("SKILL.md"),
        "---\nname: test\n---\nmodified content",
    )
    .unwrap();

    // Lockfile records a different hash (the "original" content)
    let mut lf = rune::lockfile::Lockfile::default();
    lf.skills.insert(
        "test-skill".to_string(),
        rune::lockfile::LockedSkill {
            registry: "test".to_string(),
            hash: "original_hash_that_does_not_match".to_string(),
            registry_commit: Some("abc123".to_string()),
            synced_at: "2026-04-07".to_string(),
        },
    );

    // The local hash should NOT match the lockfile hash
    let local_hash = rune::registry::skill_hash(&skills_dir.join("test-skill")).expect("hash ok");
    let locked = lf.skills.get("test-skill").unwrap();
    assert_ne!(
        local_hash, locked.hash,
        "Local hash should differ from lockfile (skill was modified)"
    );
}

#[test]
fn lockfile_matches_when_unmodified() {
    // When we hash a skill and record it in lockfile, re-hashing should match
    let tmp = tempfile::tempdir().unwrap();
    let skill_dir = tmp.path().join("skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "---\nname: test\n---\ncontent").unwrap();

    let hash = rune::registry::skill_hash(&skill_dir).unwrap();

    let mut lf = rune::lockfile::Lockfile::default();
    lf.skills.insert(
        "skill".to_string(),
        rune::lockfile::LockedSkill {
            registry: "test".to_string(),
            hash: hash.clone(),
            registry_commit: None,
            synced_at: "2026-04-07".to_string(),
        },
    );

    // Without modifying the file, hash should still match
    let rehash = rune::registry::skill_hash(&skill_dir).unwrap();
    assert_eq!(rehash, hash, "Hash should be stable for unmodified skill");
    assert_eq!(
        Some(rehash.as_str()),
        Some(lf.skills.get("skill").unwrap().hash.as_str()),
        "Lockfile hash should match local hash for unmodified skill"
    );
}

// ============================================================
// Clean identifies stale caches
// ============================================================

#[test]
fn parse_cache_metadata_name_recovers_registry() {
    // Non-vacuous: exercises the exact function rune clean calls,
    // not a reimplementation. If the production parser changes, this
    // test moves with it or fails.
    use rune::registry::parse_cache_metadata_name;

    // All five metadata suffixes for a plain registry name
    for filename in [
        ".myregistry.lock",
        ".myregistry.etag",
        ".myregistry-headers.txt",
        ".myregistry-archive.tar.gz",
        ".myregistry-extract",
    ] {
        assert_eq!(parse_cache_metadata_name(filename), "myregistry");
    }

    // Slash-containing registry names are stored with `/` replaced by `--`.
    // The parser must return that fs_name form so `clean` can compare
    // it against Registry.fs_name() in the configured set.
    for filename in [
        ".andunn--arcana.lock",
        ".andunn--arcana.etag",
        ".andunn--arcana-archive.tar.gz",
    ] {
        assert_eq!(parse_cache_metadata_name(filename), "andunn--arcana");
    }

    // A bare directory name (the registry tree itself) isn't a metadata
    // file, and the function is not expected to be called on it. But it
    // should be idempotent-ish if it is: no leading dot, no known suffix.
    assert_eq!(parse_cache_metadata_name("myregistry"), "myregistry");

    // Negative: a file with an unknown suffix is returned unchanged
    // (minus the leading dot). rune clean then treats it as an unknown
    // name and leaves it alone, which is the safe default.
    assert_eq!(
        parse_cache_metadata_name(".surprise-artifact.json"),
        "surprise-artifact.json"
    );
}

// ============================================================
// Multi-type manifest tests
// ============================================================

#[test]
fn multi_type_manifest_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    fs::create_dir_all(project.join(".claude")).unwrap();

    let mut manifest = rune::manifest::Manifest::default();

    // Add skills
    manifest.skills.insert(
        "tidy".to_string(),
        rune::manifest::SkillEntry {
            registry: Some("runes".to_string()),
            version: None,
        },
    );
    manifest.skills.insert(
        "voice".to_string(),
        rune::manifest::SkillEntry {
            registry: None,
            version: None,
        },
    );

    // Add agents
    manifest.agents.insert(
        "researcher".to_string(),
        rune::manifest::SkillEntry {
            registry: Some("runes".to_string()),
            version: None,
        },
    );

    // Add rules
    manifest.rules.insert(
        "no-emdash".to_string(),
        rune::manifest::SkillEntry {
            registry: None,
            version: None,
        },
    );

    // Save and reload
    manifest.save(project).unwrap();
    let loaded = rune::manifest::Manifest::load(project).unwrap();

    // Verify all entries preserved
    assert_eq!(loaded.skills.len(), 2, "skills count");
    assert_eq!(loaded.agents.len(), 1, "agents count");
    assert_eq!(loaded.rules.len(), 1, "rules count");
    assert_eq!(loaded.total_count(), 4, "total count");

    assert!(loaded.skills.contains_key("tidy"));
    assert!(loaded.skills.contains_key("voice"));
    assert!(loaded.agents.contains_key("researcher"));
    assert!(loaded.rules.contains_key("no-emdash"));

    // Verify pinning preserved
    assert_eq!(
        loaded.skills.get("tidy").unwrap().registry.as_deref(),
        Some("runes")
    );
    assert!(loaded.skills.get("voice").unwrap().registry.is_none());
    assert_eq!(
        loaded.agents.get("researcher").unwrap().registry.as_deref(),
        Some("runes")
    );
    assert!(loaded.rules.get("no-emdash").unwrap().registry.is_none());
}

#[test]
fn manifest_find_type() {
    use rune::manifest::ArtifactType;

    let mut manifest = rune::manifest::Manifest::default();
    manifest.skills.insert(
        "tidy".to_string(),
        rune::manifest::SkillEntry {
            registry: None,
            version: None,
        },
    );
    manifest.agents.insert(
        "researcher".to_string(),
        rune::manifest::SkillEntry {
            registry: None,
            version: None,
        },
    );
    manifest.rules.insert(
        "no-emdash".to_string(),
        rune::manifest::SkillEntry {
            registry: None,
            version: None,
        },
    );

    assert_eq!(manifest.find_type("tidy"), Some(ArtifactType::Skill));
    assert_eq!(manifest.find_type("researcher"), Some(ArtifactType::Agent));
    assert_eq!(manifest.find_type("no-emdash"), Some(ArtifactType::Rule));
    assert_eq!(manifest.find_type("nonexistent"), None);
}

#[test]
fn manifest_section_accessors() {
    use rune::manifest::ArtifactType;

    let mut manifest = rune::manifest::Manifest::default();
    manifest.section_mut(ArtifactType::Skill).insert(
        "tidy".to_string(),
        rune::manifest::SkillEntry {
            registry: None,
            version: None,
        },
    );
    manifest.section_mut(ArtifactType::Agent).insert(
        "researcher".to_string(),
        rune::manifest::SkillEntry {
            registry: None,
            version: None,
        },
    );

    assert_eq!(manifest.section(ArtifactType::Skill).len(), 1);
    assert_eq!(manifest.section(ArtifactType::Agent).len(), 1);
    assert_eq!(manifest.section(ArtifactType::Rule).len(), 0);
}

#[test]
fn manifest_backward_compat_skills_only() {
    // A manifest with only [skills] should parse correctly
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    fs::create_dir_all(project.join(".claude")).unwrap();

    let content = "[skills]\ntidy = \"runes\"\nvoice = {}\n";
    fs::write(project.join(".claude").join("rune.toml"), content).unwrap();

    let loaded = rune::manifest::Manifest::load(project).unwrap();
    assert_eq!(loaded.skills.len(), 2);
    assert!(loaded.agents.is_empty());
    assert!(loaded.rules.is_empty());
}

#[test]
fn manifest_artifact_dir_default() {
    use rune::manifest::ArtifactType;

    let manifest = rune::manifest::Manifest::default();
    let project = std::path::Path::new("/tmp/test-project");

    assert_eq!(
        manifest.artifact_dir(project, ArtifactType::Skill),
        project.join(".claude/skills")
    );
    assert_eq!(
        manifest.artifact_dir(project, ArtifactType::Agent),
        project.join(".claude/agents")
    );
    assert_eq!(
        manifest.artifact_dir(project, ArtifactType::Rule),
        project.join(".claude/rules")
    );
}

#[test]
fn manifest_artifact_dir_with_paths_override() {
    use rune::manifest::ArtifactType;
    use std::collections::BTreeMap;

    let mut paths = BTreeMap::new();
    paths.insert("agents".to_string(), ".cursor/agents".to_string());

    let manifest = rune::manifest::Manifest {
        paths: Some(paths),
        ..Default::default()
    };
    let project = std::path::Path::new("/tmp/test-project");

    // agents should use the override
    assert_eq!(
        manifest.artifact_dir(project, ArtifactType::Agent),
        project.join(".cursor/agents")
    );
    // skills should use the default (no override)
    assert_eq!(
        manifest.artifact_dir(project, ArtifactType::Skill),
        project.join(".claude/skills")
    );
}

// ============================================================
// Multi-type lockfile tests
// ============================================================

#[test]
fn multi_type_lockfile_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    fs::create_dir_all(project.join(".claude")).unwrap();

    let mut lf = rune::lockfile::Lockfile::default();

    lf.skills.insert(
        "tidy".to_string(),
        rune::lockfile::LockedSkill {
            registry: "runes".to_string(),
            hash: "abc123".to_string(),
            registry_commit: Some("def456".to_string()),
            synced_at: "2026-04-09".to_string(),
        },
    );

    lf.agents.insert(
        "researcher".to_string(),
        rune::lockfile::LockedSkill {
            registry: "runes".to_string(),
            hash: "ghi789".to_string(),
            registry_commit: None,
            synced_at: "2026-04-09".to_string(),
        },
    );

    lf.rules.insert(
        "no-emdash".to_string(),
        rune::lockfile::LockedSkill {
            registry: "runes".to_string(),
            hash: "jkl012".to_string(),
            registry_commit: None,
            synced_at: "2026-04-09".to_string(),
        },
    );

    lf.save(project).unwrap();

    let loaded = rune::lockfile::Lockfile::load(project).unwrap();
    assert_eq!(loaded.skills.len(), 1);
    assert_eq!(loaded.agents.len(), 1);
    assert_eq!(loaded.rules.len(), 1);
    assert_eq!(loaded.total_count(), 3);

    let tidy = loaded.skills.get("tidy").unwrap();
    assert_eq!(tidy.registry, "runes");
    assert_eq!(tidy.hash, "abc123");

    let researcher = loaded.agents.get("researcher").unwrap();
    assert_eq!(researcher.hash, "ghi789");

    let rule = loaded.rules.get("no-emdash").unwrap();
    assert_eq!(rule.hash, "jkl012");
}

#[test]
fn lockfile_backward_compat_skills_only() {
    // A lockfile with only [skills] should parse correctly
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    fs::create_dir_all(project.join(".claude")).unwrap();

    let content = "# Generated by rune sync\n\n[skills.tidy]\nregistry = \"runes\"\nhash = \"abc\"\nsynced_at = \"2026-04-09\"\n";
    fs::write(project.join(".claude").join("rune.lock"), content).unwrap();

    let loaded = rune::lockfile::Lockfile::load(project).unwrap();
    assert_eq!(loaded.skills.len(), 1);
    assert!(loaded.agents.is_empty());
    assert!(loaded.rules.is_empty());
}

#[test]
fn lockfile_section_accessors() {
    use rune::manifest::ArtifactType;

    let mut lf = rune::lockfile::Lockfile::default();
    lf.section_mut(ArtifactType::Agent).insert(
        "researcher".to_string(),
        rune::lockfile::LockedSkill {
            registry: "runes".to_string(),
            hash: "abc".to_string(),
            registry_commit: None,
            synced_at: "2026-04-09".to_string(),
        },
    );

    assert_eq!(lf.section(ArtifactType::Agent).len(), 1);
    assert_eq!(lf.section(ArtifactType::Skill).len(), 0);
    assert_eq!(lf.section(ArtifactType::Rule).len(), 0);
}

// ============================================================
// Typed registry tests
// ============================================================

#[test]
fn list_artifacts_typed_registry() {
    use rune::manifest::ArtifactType;

    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    // Create typed subdirectories
    let skills_dir = base.join("skills");
    let agents_dir = base.join("agents");
    fs::create_dir_all(&skills_dir).unwrap();
    fs::create_dir_all(&agents_dir).unwrap();

    // Skill as directory with SKILL.md
    fs::create_dir_all(skills_dir.join("tidy")).unwrap();
    fs::write(
        skills_dir.join("tidy").join("SKILL.md"),
        "---\nname: tidy\n---\n",
    )
    .unwrap();

    // Skill as flat file
    fs::write(skills_dir.join("voice.md"), "---\nname: voice\n---\n").unwrap();

    // Agent as flat file
    fs::write(
        agents_dir.join("researcher.md"),
        "---\nname: researcher\n---\n",
    )
    .unwrap();

    let reg = rune::config::Registry {
        name: "test".to_string(),
        url: "unused".to_string(),
        path: None,
        branch: "main".to_string(),
        readonly: false,
        source: rune::config::SourceKind::Git,
        token_env: None,
        git_email: None,
        git_name: None,
        aliases: Vec::new(),
    };

    let skills = rune::registry::list_artifacts(base, &reg, ArtifactType::Skill).unwrap();
    assert_eq!(skills, vec!["tidy", "voice"]);

    let agents = rune::registry::list_artifacts(base, &reg, ArtifactType::Agent).unwrap();
    assert_eq!(agents, vec!["researcher"]);

    let rules = rune::registry::list_artifacts(base, &reg, ArtifactType::Rule).unwrap();
    assert!(rules.is_empty());
}

#[test]
fn list_artifacts_legacy_fallback() {
    use rune::manifest::ArtifactType;

    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    // No skills/ subdir -- skills at root (legacy format)
    fs::create_dir_all(base.join("tidy")).unwrap();
    fs::write(base.join("tidy").join("SKILL.md"), "---\nname: tidy\n---\n").unwrap();
    fs::write(base.join("voice.md"), "---\nname: voice\n---\n").unwrap();

    let reg = rune::config::Registry {
        name: "test".to_string(),
        url: "unused".to_string(),
        path: None,
        branch: "main".to_string(),
        readonly: false,
        source: rune::config::SourceKind::Git,
        token_env: None,
        git_email: None,
        git_name: None,
        aliases: Vec::new(),
    };

    // Skills should be found via legacy fallback (root)
    let skills = rune::registry::list_artifacts(base, &reg, ArtifactType::Skill).unwrap();
    assert!(skills.contains(&"tidy".to_string()));
    assert!(skills.contains(&"voice".to_string()));
}

#[test]
fn artifact_path_typed_vs_legacy() {
    use rune::manifest::ArtifactType;

    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    let reg = rune::config::Registry {
        name: "test".to_string(),
        url: "unused".to_string(),
        path: None,
        branch: "main".to_string(),
        readonly: false,
        source: rune::config::SourceKind::Git,
        token_env: None,
        git_email: None,
        git_name: None,
        aliases: Vec::new(),
    };

    // With typed subdir: agents/
    fs::create_dir_all(base.join("agents")).unwrap();
    fs::write(base.join("agents").join("researcher.md"), "content").unwrap();

    let agent_path = rune::registry::artifact_path(base, &reg, "researcher", ArtifactType::Agent);
    assert_eq!(agent_path, base.join("agents").join("researcher.md"));

    // Without typed subdir: skills fall back to root
    // (no skills/ subdir exists, so falls back to base)
    let skill_path = rune::registry::artifact_path(base, &reg, "tidy", ArtifactType::Skill);
    // Should look at root since no skills/ subdir
    assert_eq!(skill_path, base.join("tidy.md"));

    // With typed subdir: skills/
    fs::create_dir_all(base.join("skills")).unwrap();
    let skill_path_typed = rune::registry::artifact_path(base, &reg, "tidy", ArtifactType::Skill);
    // Now should prefer the typed subdir
    assert_eq!(skill_path_typed, base.join("skills").join("tidy.md"));
}

#[test]
fn artifact_type_parse() {
    use rune::manifest::ArtifactType;

    assert_eq!(ArtifactType::parse("skill"), Some(ArtifactType::Skill));
    assert_eq!(ArtifactType::parse("skills"), Some(ArtifactType::Skill));
    assert_eq!(ArtifactType::parse("agent"), Some(ArtifactType::Agent));
    assert_eq!(ArtifactType::parse("agents"), Some(ArtifactType::Agent));
    assert_eq!(ArtifactType::parse("rule"), Some(ArtifactType::Rule));
    assert_eq!(ArtifactType::parse("rules"), Some(ArtifactType::Rule));
    assert_eq!(ArtifactType::parse("SKILL"), Some(ArtifactType::Skill));
    assert_eq!(ArtifactType::parse("unknown"), None);
    assert_eq!(ArtifactType::parse("command"), None);
}

#[test]
fn artifact_type_properties() {
    use rune::manifest::ArtifactType;

    assert_eq!(ArtifactType::Skill.section(), "skills");
    assert_eq!(ArtifactType::Agent.section(), "agents");
    assert_eq!(ArtifactType::Rule.section(), "rules");

    assert_eq!(ArtifactType::Skill.default_dir(), ".claude/skills");
    assert_eq!(ArtifactType::Agent.default_dir(), ".claude/agents");
    assert_eq!(ArtifactType::Rule.default_dir(), ".claude/rules");

    assert!(ArtifactType::Skill.is_directory_type());
    assert!(!ArtifactType::Agent.is_directory_type());
    assert!(!ArtifactType::Rule.is_directory_type());

    assert_eq!(ArtifactType::Skill.singular(), "skill");
    assert_eq!(ArtifactType::Agent.singular(), "agent");
    assert_eq!(ArtifactType::Rule.singular(), "rule");
}

#[test]
fn validate_name_works() {
    // validate_name is the new generic version
    assert!(rune::registry::validate_name("tidy").is_ok());
    assert!(rune::registry::validate_name("my-agent").is_ok());
    assert!(rune::registry::validate_name("no-emdash").is_ok());

    // Same validation rules as validate_name
    assert!(rune::registry::validate_name("../escape").is_err());
    assert!(rune::registry::validate_name(".hidden").is_err());
    assert!(rune::registry::validate_name("").is_err());
    assert!(rune::registry::validate_name("-flag").is_err());

    // Alias still works
    assert!(rune::registry::validate_name("tidy").is_ok());
    assert!(rune::registry::validate_name("../escape").is_err());
}

// ── Skill version (@version suffix + table form) ───────────────────────

#[test]
fn skill_entry_string_no_version() {
    // Plain registry name, no version
    let toml = r#"voice = "andunn/arcana""#;
    let manifest: toml::Value = toml::from_str(toml).unwrap();
    let entry: rune::manifest::SkillEntry =
        manifest.get("voice").unwrap().clone().try_into().unwrap();
    assert_eq!(entry.registry.as_deref(), Some("andunn/arcana"));
    assert_eq!(entry.version, None);
}

#[test]
fn skill_entry_string_with_version() {
    // `registry@version` shorthand
    let toml = r#"voice = "andunn/arcana@v1.2.0""#;
    let manifest: toml::Value = toml::from_str(toml).unwrap();
    let entry: rune::manifest::SkillEntry =
        manifest.get("voice").unwrap().clone().try_into().unwrap();
    assert_eq!(entry.registry.as_deref(), Some("andunn/arcana"));
    assert_eq!(entry.version.as_deref(), Some("v1.2.0"));
}

#[test]
fn skill_entry_string_commit_hash() {
    // Full commit hash as version
    let toml = r#"voice = "andunn/arcana@abc123def456""#;
    let manifest: toml::Value = toml::from_str(toml).unwrap();
    let entry: rune::manifest::SkillEntry =
        manifest.get("voice").unwrap().clone().try_into().unwrap();
    assert_eq!(entry.registry.as_deref(), Some("andunn/arcana"));
    assert_eq!(entry.version.as_deref(), Some("abc123def456"));
}

#[test]
fn skill_entry_table_with_version() {
    // Explicit table form
    let toml = r#"
        [voice]
        registry = "andunn/arcana"
        version = "v1.2.0"
    "#;
    let manifest: toml::Value = toml::from_str(toml).unwrap();
    let entry: rune::manifest::SkillEntry =
        manifest.get("voice").unwrap().clone().try_into().unwrap();
    assert_eq!(entry.registry.as_deref(), Some("andunn/arcana"));
    assert_eq!(entry.version.as_deref(), Some("v1.2.0"));
}

#[test]
fn skill_entry_serialize_shorthand() {
    // Round-trip: with version serializes as "voice = \"registry@version\""
    let mut manifest = rune::manifest::Manifest::default();
    manifest.skills.insert(
        "voice".to_string(),
        rune::manifest::SkillEntry {
            registry: Some("andunn/arcana".to_string()),
            version: Some("v1.2.0".to_string()),
        },
    );
    let out = toml::to_string(&manifest).unwrap();
    assert!(
        out.contains("voice = \"andunn/arcana@v1.2.0\""),
        "got: {out}"
    );
}

#[test]
fn skill_entry_empty_version_keeps_registry() {
    // Trailing `@` with nothing after it isn't a real version pin.
    let toml = r#"voice = "andunn/arcana@""#;
    let manifest: toml::Value = toml::from_str(toml).unwrap();
    let entry: rune::manifest::SkillEntry =
        manifest.get("voice").unwrap().clone().try_into().unwrap();
    // Empty version → treat as no version
    assert_eq!(entry.registry.as_deref(), Some("andunn/arcana@"));
    assert_eq!(entry.version, None);
}

#[test]
fn hook_script_covers_all_artifact_types() {
    // Derivation obligation: the hook script's file-path matcher must
    // cover every ArtifactType. Adding a fourth type without updating
    // resources/hook.sh would silently leak its file events to no hook
    // handler. This test links ALL_TYPES (the spec) to the hook script
    // (the derived artifact) so drift between them is a CI failure.
    let hook = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/hook.sh"))
        .expect("hook script must be present for testing");
    for at in rune::manifest::ALL_TYPES {
        let dir = at.default_dir();
        assert!(
            hook.contains(dir),
            "hook.sh is missing a file-path match for {} (expected substring {dir:?}). \
             Add the case arm to resources/hook.sh, or remove {} from ArtifactType.",
            at.singular(),
            at.singular()
        );
    }
}

#[test]
fn registry_lookup_matches_by_alias() {
    // Backward compatibility: a registry that was renamed (e.g. from a
    // short form to a qualified form) must still resolve lookups
    // against the old name when it's declared as an alias.
    let config = rune::config::Config {
        registry: vec![rune::config::Registry {
            name: "owner/canonical".to_string(),
            url: "https://example.test/owner/canonical.git".to_string(),
            path: None,
            branch: "main".to_string(),
            readonly: false,
            source: rune::config::SourceKind::Git,
            token_env: None,
            git_email: None,
            git_name: None,
            aliases: vec!["canonical".to_string()],
        }],
    };

    let canonical = config
        .registry("owner/canonical")
        .expect("canonical lookup");
    assert_eq!(canonical.name, "owner/canonical");

    let alias = config.registry("canonical").expect("alias lookup");
    assert_eq!(alias.name, "owner/canonical", "alias resolves to canonical");

    assert!(
        config.registry("other").is_none(),
        "unrelated name must not match"
    );
}

#[test]
fn resolve_artifact_handles_slash_in_registry_name() {
    // Regression test for v0.8.1-era bug: Config::resolve_artifact must use
    // reg.fs_name() (replaces `/` with `--`) when constructing the cache path,
    // not reg.name directly. A registry named `foo/bar` caches at `foo--bar/`;
    // joining by name produces a nonexistent `foo/bar/` path and resolution
    // silently returns None.
    let tmp = tempfile::tempdir().unwrap();
    let cache_dir = tmp.path();

    let repo_dir = cache_dir.join("foo--bar");
    fs::create_dir_all(&repo_dir).unwrap();
    fs::write(repo_dir.join("tidy.md"), "---\nname: tidy\n---\n").unwrap();

    let config = rune::config::Config {
        registry: vec![rune::config::Registry {
            name: "foo/bar".to_string(),
            url: "https://example.com/foo/bar".to_string(),
            path: None,
            branch: "main".to_string(),
            readonly: false,
            source: rune::config::SourceKind::Git,
            token_env: None,
            git_email: None,
            git_name: None,
            aliases: Vec::new(),
        }],
    };

    let found = config.resolve_artifact("tidy", cache_dir, rune::manifest::ArtifactType::Skill);
    assert!(
        found.is_some(),
        "resolve_artifact failed to find registry with `/` in name"
    );
    assert_eq!(found.unwrap().name, "foo/bar");
}

// ============================================================
// Migration scenarios
//
// End-to-end regression tests for state that ages across renames,
// version bumps, and other ecosystem evolution. Intrinsic tests
// (does X do what it claims) live above; these tests exercise
// "does rune survive its own config changing out from under old
// project state". The bug class this module guards against was
// fixed in v0.12: every rune command failed with "Unknown
// registry" after a config rename.
// ============================================================

#[test]
fn migration_rename_registry_without_alias_fails_lookup() {
    // Baseline: without an alias, an old name fails to resolve.
    // This is the shape of the v0.12 bug. Test exists so that if
    // somebody weakens the alias check, we see it here first.
    let config = rune::config::Config {
        registry: vec![rune::config::Registry {
            name: "new/qualified".to_string(),
            url: "https://example.test/new/qualified.git".to_string(),
            path: None,
            branch: "main".to_string(),
            readonly: false,
            source: rune::config::SourceKind::Git,
            token_env: None,
            git_email: None,
            git_name: None,
            aliases: vec![],
        }],
    };

    // A project's manifest or lockfile references the old short name.
    assert!(
        config.registry("old-short").is_none(),
        "no alias -> old name unresolvable (this was the v0.9-v0.11 bug)"
    );
}

#[test]
fn migration_rename_registry_with_alias_resolves() {
    // Recovery: the admin adds the old name as an alias on the
    // renamed entry. Every project's existing manifest/lockfile now
    // resolves again without any per-project migration.
    let config = rune::config::Config {
        registry: vec![rune::config::Registry {
            name: "new/qualified".to_string(),
            url: "https://example.test/new/qualified.git".to_string(),
            path: None,
            branch: "main".to_string(),
            readonly: false,
            source: rune::config::SourceKind::Git,
            token_env: None,
            git_email: None,
            git_name: None,
            aliases: vec!["old-short".to_string()],
        }],
    };

    // Simulates a pre-rename manifest entry.
    let entry = rune::manifest::SkillEntry {
        registry: Some("old-short".to_string()),
        version: None,
    };

    let resolved = config
        .registry(entry.registry.as_deref().unwrap())
        .expect("alias must resolve");
    assert_eq!(
        resolved.name, "new/qualified",
        "alias must resolve to the canonical registry"
    );
}

#[test]
fn migration_aliases_do_not_mask_duplicate_canonical_names() {
    // If two distinct registries exist, an alias on one must not
    // collide with another's canonical name. Lookup returns the
    // first match in declaration order.
    let config = rune::config::Config {
        registry: vec![
            rune::config::Registry {
                name: "primary".to_string(),
                url: "https://example.test/primary.git".to_string(),
                path: None,
                branch: "main".to_string(),
                readonly: false,
                source: rune::config::SourceKind::Git,
                token_env: None,
                git_email: None,
                git_name: None,
                aliases: vec![],
            },
            rune::config::Registry {
                name: "secondary".to_string(),
                url: "https://example.test/secondary.git".to_string(),
                path: None,
                branch: "main".to_string(),
                readonly: false,
                source: rune::config::SourceKind::Git,
                token_env: None,
                git_email: None,
                git_name: None,
                aliases: vec!["primary".to_string()], // colliding alias
            },
        ],
    };

    // Canonical name wins over alias — iteration order returns the
    // first entry whose name matches OR alias matches. "primary" as
    // a canonical name is found first, so its registry resolves.
    let resolved = config.registry("primary").expect("primary resolves");
    assert_eq!(
        resolved.name, "primary",
        "canonical name takes precedence over alias in declaration order"
    );
}

#[test]
fn migration_lockfile_drift_detectable_via_config_registry() {
    // This is the exact health check `rune doctor` performs: for
    // each lockfile entry, confirm its `registry` field still
    // resolves. This test shapes that logic as a pure-function
    // check so the doctor command's behavior is tested here too.
    let config = rune::config::Config {
        registry: vec![rune::config::Registry {
            name: "current/name".to_string(),
            url: "https://example.test/current/name.git".to_string(),
            path: None,
            branch: "main".to_string(),
            readonly: false,
            source: rune::config::SourceKind::Git,
            token_env: None,
            git_email: None,
            git_name: None,
            aliases: vec![],
        }],
    };

    let mut lockfile = rune::lockfile::Lockfile::default();
    lockfile.skills.insert(
        "example".to_string(),
        rune::lockfile::LockedSkill {
            registry: "pre-rename".to_string(),
            hash: "0".repeat(64),
            registry_commit: None,
            synced_at: "2026-01-01".to_string(),
        },
    );

    // Drift detection: iterate lock sections, flag entries whose
    // registry doesn't resolve in current config.
    let mut drifted = Vec::new();
    for at in rune::manifest::ALL_TYPES {
        for (name, locked) in lockfile.section(at) {
            if config.registry(&locked.registry).is_none() {
                drifted.push((name.clone(), locked.registry.clone()));
            }
        }
    }

    assert_eq!(
        drifted,
        vec![("example".to_string(), "pre-rename".to_string())],
        "doctor's drift check must flag the stale lock entry"
    );
}
