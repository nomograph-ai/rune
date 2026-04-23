use anyhow::Result;

/// Validate an item name contains only safe characters.
/// Prevents path traversal attacks via names like `../../etc/passwd`.
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Name cannot be empty");
    }
    if name != name.trim() || name.contains(char::is_whitespace) {
        anyhow::bail!("Invalid name: {name:?} (must not contain whitespace)");
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
        anyhow::bail!(
            "Invalid name: {name}. Use [a-zA-Z0-9_-]+ only (no slashes, dots, or null bytes)."
        );
    }
    if name.starts_with('.') || name.starts_with('-') {
        anyhow::bail!(
            "Invalid name: {name}. Use [a-zA-Z0-9_-]+ only (must not start with . or -)."
        );
    }
    Ok(())
}
