//! Integration tests for the archive-source registry path.
//!
//! Uses httpmock to serve canned responses on a local port so the
//! 200/304/404/truncated-archive cases are exercised against a real
//! HTTP client (ureq) without hitting the network. The only thing
//! stubbed is the remote — the tar + flate2 + filesystem layers are
//! the real implementations.
//!
//! The RUNE_ARCHIVE_URL_<FS_NAME> env override in
//! src/registry/archive.rs::resolve_archive_url points ureq at the
//! mock server. Test registry names are unique per test so the tests
//! can run in parallel without stepping on each other's env vars.

use httpmock::prelude::*;
use rune::config::{Registry, SourceKind};
use rune::registry::archive::ensure_archive_registry;

/// Build an in-memory `.tar.gz` containing a minimal skill tree.
///
/// Layout (what GitHub/GitLab serve):
///
///   <top-level>/
///     skills/
///       tidy/
///         SKILL.md
///
/// The archive's top-level directory is stripped on extraction
/// (strip-components=1), so the extracted tree starts at `skills/`.
fn build_tarball(top_level: &str, skill_body: &str) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut tar = tar::Builder::new(&mut gz);

        let top_dir_header = {
            let mut h = tar::Header::new_gnu();
            h.set_path(format!("{top_level}/")).unwrap();
            h.set_size(0);
            h.set_entry_type(tar::EntryType::Directory);
            h.set_mode(0o755);
            h.set_cksum();
            h
        };
        tar.append(&top_dir_header, std::io::empty()).unwrap();

        let skills_dir_header = {
            let mut h = tar::Header::new_gnu();
            h.set_path(format!("{top_level}/skills/")).unwrap();
            h.set_size(0);
            h.set_entry_type(tar::EntryType::Directory);
            h.set_mode(0o755);
            h.set_cksum();
            h
        };
        tar.append(&skills_dir_header, std::io::empty()).unwrap();

        let tidy_dir_header = {
            let mut h = tar::Header::new_gnu();
            h.set_path(format!("{top_level}/skills/tidy/")).unwrap();
            h.set_size(0);
            h.set_entry_type(tar::EntryType::Directory);
            h.set_mode(0o755);
            h.set_cksum();
            h
        };
        tar.append(&tidy_dir_header, std::io::empty()).unwrap();

        let body_bytes = skill_body.as_bytes();
        let mut skill_header = tar::Header::new_gnu();
        skill_header
            .set_path(format!("{top_level}/skills/tidy/SKILL.md"))
            .unwrap();
        skill_header.set_size(body_bytes.len() as u64);
        skill_header.set_mode(0o644);
        skill_header.set_entry_type(tar::EntryType::Regular);
        skill_header.set_cksum();
        tar.append(&skill_header, body_bytes).unwrap();
        tar.finish().unwrap();
    }
    gz.finish().unwrap()
}

/// Make a test Registry pointing at `mock_url` via the env override.
/// Also sets the env var — caller must keep the returned name unique
/// per test so env var scopes stay isolated.
fn test_registry(name: &str, mock_url: &str) -> Registry {
    let env_var = format!("RUNE_ARCHIVE_URL_{}", name.to_uppercase().replace('-', "_"));
    // SAFETY: tests access this env var within their own thread only,
    // via the archive code path. Parallel tests use different names.
    unsafe {
        std::env::set_var(&env_var, mock_url);
    }

    Registry {
        name: name.to_string(),
        url: format!("https://example.test/{name}"),
        path: None,
        branch: "main".to_string(),
        readonly: true,
        source: SourceKind::Archive,
        token_env: None,
        git_email: None,
        git_name: None,
    }
}

#[test]
fn archive_200_fresh_extracts() {
    let server = MockServer::start();
    let tarball = build_tarball("test-200-fresh-main", "---\nname: tidy\n---\n# Tidy\n");
    let mock = server.mock(|when, then| {
        when.method(GET).path("/archive.tar.gz");
        then.status(200)
            .header("ETag", "\"abc123\"")
            .header("Content-Type", "application/gzip")
            .body(tarball);
    });

    let cache_tmp = tempfile::tempdir().unwrap();
    let dest = cache_tmp.path().join("test-200-fresh");
    let reg = test_registry("test-200-fresh", &server.url("/archive.tar.gz"));

    ensure_archive_registry(&reg, &dest).expect("archive fetch ok");

    // Top-level stripped: archive root inside `skills/tidy/SKILL.md`.
    let skill = dest.join("skills").join("tidy").join("SKILL.md");
    assert!(skill.exists(), "extracted SKILL.md at {}", skill.display());
    let content = std::fs::read_to_string(&skill).unwrap();
    assert!(content.contains("name: tidy"));

    // ETag got persisted.
    let etag_path = cache_tmp.path().join(".test-200-fresh.etag");
    assert_eq!(
        std::fs::read_to_string(&etag_path).unwrap(),
        "\"abc123\"",
        "etag cached for next If-None-Match"
    );

    mock.assert();
}

#[test]
fn archive_304_preserves_cache() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/archive.tar.gz")
            .header("If-None-Match", "\"cached-etag\"");
        then.status(304);
    });

    let cache_tmp = tempfile::tempdir().unwrap();
    let dest = cache_tmp.path().join("test-304");

    // Seed a cache: dest dir with sentinel file, plus etag.
    std::fs::create_dir_all(&dest).unwrap();
    let sentinel = dest.join("SENTINEL.md");
    std::fs::write(&sentinel, "do not touch").unwrap();
    let etag_path = cache_tmp.path().join(".test-304.etag");
    std::fs::write(&etag_path, "\"cached-etag\"").unwrap();

    let reg = test_registry("test-304", &server.url("/archive.tar.gz"));
    ensure_archive_registry(&reg, &dest).expect("304 path ok");

    // Cache untouched.
    assert!(sentinel.exists(), "304 must not delete cached tree");
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "do not touch");

    mock.assert();
}

#[test]
fn archive_404_without_cache_errors() {
    let server = MockServer::start();
    let _mock = server.mock(|when, then| {
        when.method(GET).path("/archive.tar.gz");
        then.status(404).body("not found");
    });

    let cache_tmp = tempfile::tempdir().unwrap();
    let dest = cache_tmp.path().join("test-404-no-cache");
    let reg = test_registry("test-404-no-cache", &server.url("/archive.tar.gz"));

    let result = ensure_archive_registry(&reg, &dest);
    assert!(result.is_err(), "404 + no cache must error");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("404") || err.to_lowercase().contains("http"),
        "error should name the HTTP failure: got {err:?}"
    );
    assert!(
        err.contains("rune doctor"),
        "error should prescribe `rune doctor`: got {err:?}"
    );
}

#[test]
fn archive_404_with_cache_warns_and_returns_ok() {
    let server = MockServer::start();
    let _mock = server.mock(|when, then| {
        when.method(GET).path("/archive.tar.gz");
        then.status(404).body("not found");
    });

    let cache_tmp = tempfile::tempdir().unwrap();
    let dest = cache_tmp.path().join("test-404-with-cache");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("OLD.md"), "prior version").unwrap();

    let reg = test_registry("test-404-with-cache", &server.url("/archive.tar.gz"));

    // Should succeed (warning on stderr), not error, because we have a
    // cached dest to fall back on.
    ensure_archive_registry(&reg, &dest).expect("404 + cache falls back to cached tree");

    assert!(
        dest.join("OLD.md").exists(),
        "cached tree preserved on 404 fallback"
    );
}

#[test]
fn archive_truncated_gz_errors() {
    let server = MockServer::start();
    // First 20 bytes of a gzip are a valid header but won't decode.
    let truncated = {
        let full = build_tarball("truncated-main", "body");
        full.into_iter().take(20).collect::<Vec<u8>>()
    };
    let _mock = server.mock(|when, then| {
        when.method(GET).path("/archive.tar.gz");
        then.status(200).body(truncated);
    });

    let cache_tmp = tempfile::tempdir().unwrap();
    let dest = cache_tmp.path().join("test-truncated");
    let reg = test_registry("test-truncated", &server.url("/archive.tar.gz"));

    let result = ensure_archive_registry(&reg, &dest);
    assert!(
        result.is_err(),
        "truncated gzip must error, not silently succeed"
    );
    // dest must not exist — atomic swap guarantees no half-written state.
    assert!(
        !dest.exists(),
        "failed extract must not leave a half-extracted dest"
    );
}

#[test]
fn archive_etag_roundtrip() {
    let server = MockServer::start();
    let tarball = build_tarball("etag-test-main", "---\nname: tidy\n---\n");

    let mut mock_fresh = server.mock(|when, then| {
        when.method(GET).path("/archive.tar.gz");
        then.status(200)
            .header("ETag", "\"v1\"")
            .body(tarball.clone());
    });

    let cache_tmp = tempfile::tempdir().unwrap();
    let dest = cache_tmp.path().join("test-etag");
    let reg = test_registry("test-etag", &server.url("/archive.tar.gz"));

    // First call: 200 + ETag.
    ensure_archive_registry(&reg, &dest).expect("first fetch ok");
    mock_fresh.assert();

    // Remove the first mock so it can't swallow the second request.
    mock_fresh.delete();

    // Second call: server expects If-None-Match header from the cached ETag.
    let mock_conditional = server.mock(|when, then| {
        when.method(GET)
            .path("/archive.tar.gz")
            .header("If-None-Match", "\"v1\"");
        then.status(304);
    });

    ensure_archive_registry(&reg, &dest).expect("second fetch ok");
    mock_conditional.assert();
}

// Path-traversal defense: the tar crate rejects `..` components at its
// own entry-parsing layer (confirmed in manual testing — constructing a
// tarball with `..` in set_path panics, and reading one containing raw
// `..` header bytes returns Err from archive.entries()). Our explicit
// check in extract_into() is defense-in-depth for the case where tar's
// behavior ever weakens. Exercising it requires bypassing the tar crate,
// which isn't worth the byte-level tarball construction. We rely on
// tar's own test suite for that layer.
