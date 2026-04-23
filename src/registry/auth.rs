use anyhow::Result;
use std::path::PathBuf;

use crate::config::{Config, Registry};

/// Resolve a PAT for a registry.
///
/// Token resolution order:
/// 1. `token_env` -- explicit env var in config (highest priority)
/// 2. `glab auth token` -- if URL contains gitlab.com and glab is installed
/// 3. `gh auth token` -- if URL contains github.com and gh is installed
/// 4. No auth -- rely on system credential helpers or public access
pub fn resolve_token(reg: &Registry) -> Result<Option<String>> {
    // 1. Explicit env var takes priority
    if let Some(ref env_var) = reg.token_env {
        return match std::env::var(env_var) {
            Ok(t) if !t.is_empty() => Ok(Some(t)),
            Ok(_) => anyhow::bail!("${env_var} is set but empty (registry {})", reg.name),
            Err(_) => anyhow::bail!(
                "Registry {} requires token from ${env_var} but the variable is not set",
                reg.name
            ),
        };
    }

    // 2. Auto-detect from glab/gh CLI based on URL host
    if (reg.url.contains("gitlab.com") || reg.url.contains("gitlab."))
        && let Some(token) = cli_token("glab", &["auth", "token"])
    {
        return Ok(Some(token));
    }

    if (reg.url.contains("github.com") || reg.url.contains("github."))
        && let Some(token) = cli_token("gh", &["auth", "token"])
    {
        return Ok(Some(token));
    }

    // 3. No auth -- rely on system credential helpers or public access
    Ok(None)
}

/// Try to get a token from a CLI tool. Returns None on any failure.
pub(super) fn cli_token(cmd: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Build the curl auth header for archive downloads.
/// Returns None if no token is available.
pub(super) fn curl_auth_header(reg: &Registry) -> Result<Option<String>> {
    let token = resolve_token(reg)?;
    let Some(token) = token else {
        return Ok(None);
    };
    if reg.url.contains("github.com") || reg.url.contains("github.") {
        Ok(Some(format!("Authorization: Bearer {token}")))
    } else {
        Ok(Some(format!("PRIVATE-TOKEN: {token}")))
    }
}

/// Create (or reuse) a tiny askpass script that echoes RUNE_GIT_TOKEN.
pub(super) fn create_askpass_path() -> Result<PathBuf> {
    let cache_dir = Config::cache_dir()?;
    std::fs::create_dir_all(&cache_dir)?;
    let path = cache_dir.join(".rune-askpass");
    if !path.exists() {
        std::fs::write(&path, "#!/bin/sh\necho \"$RUNE_GIT_TOKEN\"\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(path)
}

/// Build a clone URL with `oauth2@` username so GIT_ASKPASS triggers.
/// The username tells git to ask for credentials; GIT_ASKPASS provides the token.
/// This URL is only used for the clone command -- the persisted remote is the plain URL.
pub(super) fn clone_url(reg: &Registry) -> Result<String> {
    let has_token = resolve_token(reg)?.is_some();
    if !has_token {
        return Ok(reg.url.clone());
    }
    // https://gitlab.com/path → https://oauth2@gitlab.com/path
    if let Some(rest) = reg.url.strip_prefix("https://") {
        Ok(format!("https://oauth2@{rest}"))
    } else {
        Ok(reg.url.clone())
    }
}
