use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Copy a skill from registry to local. Rejects symlinks.
pub fn copy_skill(src: &Path, dest: &Path) -> Result<()> {
    if src.symlink_metadata()?.file_type().is_symlink() {
        anyhow::bail!("Refusing to copy symlink: {}", src.display());
    }
    if src.is_dir() {
        copy_dir_recursive(src, dest)?;
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dest)?;
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let name = entry.file_name();

        if ft.is_symlink() || name.to_string_lossy().starts_with('.') {
            continue;
        }

        let src_path = entry.path();
        let dest_path = dest.join(&name);

        if ft.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

pub fn is_directory_skill(path: &Path) -> bool {
    path.symlink_metadata()
        .map(|m| m.file_type().is_dir())
        .unwrap_or(false)
}

// ── Hash operations ─────────────────────────────────────────────────

/// Hash all files in a skill for drift detection. Rejects symlinks and
/// I/O errors with a hard `Err`. Bundled Enforcement: if we can't
/// compute a hash, the caller must see why.
pub fn skill_hash(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};

    let meta = path
        .symlink_metadata()
        .with_context(|| format!("reading metadata for {}", path.display()))?;
    if meta.file_type().is_symlink() {
        anyhow::bail!("refusing to hash symlink: {}", path.display());
    }

    let mut hasher = Sha256::new();
    if meta.is_dir() {
        let mut files = collect_files(path);
        files.sort();
        for file in files {
            let relative = file.strip_prefix(path).unwrap_or(&file);
            hasher.update(relative.to_string_lossy().as_bytes());
            let content =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            hasher.update(&content);
        }
    } else if meta.is_file() {
        let content = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        hasher.update(&content);
    } else {
        anyhow::bail!(
            "cannot hash {}: not a regular file or directory",
            path.display()
        );
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Collect all files recursively. Skips symlinks and dotfiles.
/// Public for integration tests.
#[allow(dead_code)]
pub fn collect_files_public(dir: &Path) -> Vec<PathBuf> {
    collect_files(dir)
}

pub(super) fn collect_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return files,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ft.is_dir() {
            files.extend(collect_files(&path));
        } else {
            files.push(path);
        }
    }
    files
}
