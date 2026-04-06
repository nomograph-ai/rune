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
        let result = rune::registry::validate_skill_name(name);
        assert!(
            result.is_err(),
            "SECURITY: validate_skill_name accepted malicious name: {name:?}"
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
        let result = rune::registry::validate_skill_name(name);
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

    let result = rune::registry::copy_skill(
        &tmp.path().join("evil-skill"),
        &target,
    );
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

    let files = rune::registry::collect_files_public(&dir);
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

    let files = rune::registry::collect_files_public(&dir);
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

    let hash1 = rune::registry::skill_hash(&dir);
    let hash2 = rune::registry::skill_hash(&dir);
    assert_eq!(hash1, hash2, "Hash should be deterministic");
    assert!(hash1.is_some());
}

#[test]
fn skill_hash_differs_on_content_change() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("skill");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), "version 1").unwrap();

    let hash1 = rune::registry::skill_hash(&dir);

    fs::write(dir.join("SKILL.md"), "version 2").unwrap();
    let hash2 = rune::registry::skill_hash(&dir);

    assert_ne!(hash1, hash2, "Hash should change when content changes");
}

#[test]
fn skill_hash_returns_none_for_symlink_file() {
    let tmp = tempfile::tempdir().unwrap();
    unix_fs::symlink("/etc/passwd", tmp.path().join("link.md")).unwrap();

    let hash = rune::registry::skill_hash(&tmp.path().join("link.md"));
    assert!(
        hash.is_none(),
        "SECURITY: skill_hash should return None for symlinks"
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
        ("", false),                              // empty
        ("no frontmatter here", false),           // no ---
        ("---\n", false),                         // unclosed
        ("---\nname: test\n---\n", true),         // valid
        ("---\n\n---\nbody", false),              // empty frontmatter (no name)
        ("---\nname: has:colons:in:value\n---\n", true), // colons in value
    ];
    for (i, (content, should_have_name)) in cases.iter().enumerate() {
        let file = tmp.path().join(format!("test-{i}.md"));
        fs::write(&file, content).unwrap();
        let result = rune::pedigree::Pedigree::from_skill(&file);
        assert!(result.is_ok(), "Should not panic on: {content:?}");
        if *should_have_name {
            assert!(result.unwrap().name.is_some(), "Expected name in: {content:?}");
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
    assert!(year >= 2024 && year <= 2100);
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
    fs::write(base.join("valid-skill").join("SKILL.md"), "---\nname: valid\n---\n").unwrap();

    // Directory WITHOUT SKILL.md -- should NOT be listed
    fs::create_dir_all(base.join("not-a-skill")).unwrap();
    fs::write(base.join("not-a-skill").join("README.md"), "not a skill").unwrap();

    // Dotdir -- should NOT be listed
    fs::create_dir_all(base.join(".hidden")).unwrap();
    fs::write(base.join(".hidden").join("SKILL.md"), "---\nname: hidden\n---\n").unwrap();

    let reg = rune::config::Registry {
        name: "test".to_string(),
        url: "unused".to_string(),
        path: None,
        branch: "main".to_string(),
        readonly: false,
        source: "git".to_string(),
    };
    let skills = rune::registry::list_skills(base, &reg).unwrap();
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
        source: "git".to_string(),
    };
    let skills = rune::registry::list_skills(base, &reg).unwrap();
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
