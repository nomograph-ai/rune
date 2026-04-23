use anyhow::{Context, Result};
use fs2::FileExt;
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::{Config, Registry};

// ── Registry pull cache ─────────────────────────────────────────────

static PULLED_REGISTRIES: Mutex<Option<HashSet<String>>> = Mutex::new(None);

pub(super) fn mark_pulled(name: &str) {
    let mut guard = PULLED_REGISTRIES.lock().unwrap();
    let set = guard.get_or_insert_with(HashSet::new);
    set.insert(name.to_string());
}

pub(super) fn already_pulled(name: &str) -> bool {
    let guard = PULLED_REGISTRIES.lock().unwrap();
    guard.as_ref().map(|s| s.contains(name)).unwrap_or(false)
}

/// Global offline flag. Set by --offline CLI flag.
static OFFLINE: AtomicBool = AtomicBool::new(false);

pub fn set_offline(offline: bool) {
    OFFLINE.store(offline, Ordering::Relaxed);
}

pub fn is_offline() -> bool {
    OFFLINE.load(Ordering::Relaxed)
}

/// Global dry-run flag. Set by --dry-run CLI flag.
static DRY_RUN: AtomicBool = AtomicBool::new(false);

pub fn set_dry_run(dry_run: bool) {
    DRY_RUN.store(dry_run, Ordering::Relaxed);
}

pub fn is_dry_run() -> bool {
    DRY_RUN.load(Ordering::Relaxed)
}

// ── File locking ────────────────────────────────────────────────────

/// RAII guard that holds a file lock and releases it on drop.
pub(super) struct LockGuard(std::fs::File);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Acquire an exclusive file lock on a registry's cache directory.
/// Returns a guard that releases the lock on drop.
pub(super) fn lock_registry(reg: &Registry) -> Result<LockGuard> {
    let cache_dir = Config::cache_dir()?;
    std::fs::create_dir_all(&cache_dir)?;
    let lock_path = cache_dir.join(format!(".{}.lock", reg.fs_name()));
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("Failed to create lock file for {}", reg.name))?;

    lock_file
        .try_lock_exclusive()
        .or_else(|_| {
            eprintln!("  waiting for lock on {}...", reg.name);
            lock_file.lock_exclusive()
        })
        .with_context(|| format!("Failed to lock registry {}", reg.name))?;

    Ok(LockGuard(lock_file))
}

/// Parse a cache metadata filename back to the registry fs_name it belongs
/// to. Cache dir siblings of a registry tree include:
///
///   .<fs_name>.lock              (file lock)
///   .<fs_name>.etag              (archive ETag)
///   .<fs_name>-headers.txt       (transient curl headers)
///   .<fs_name>-archive.tar.gz    (in-progress download)
///   .<fs_name>-extract           (in-progress extraction)
///
/// `rune clean` uses this to decide whether a metadata file belongs to a
/// registry that is no longer configured.
pub fn parse_cache_metadata_name(filename: &str) -> &str {
    filename
        .trim_start_matches('.')
        .trim_end_matches(".lock")
        .trim_end_matches(".etag")
        .trim_end_matches("-headers.txt")
        .trim_end_matches("-archive.tar.gz")
        .trim_end_matches("-extract")
}
