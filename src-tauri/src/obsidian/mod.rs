//! Local Obsidian vault plugin foundation.
//!
//! This module owns the local vault grant lifecycle. It is intentionally not a
//! connector: a vault grant is filesystem authority for a selected local root,
//! not a third-party account grant with OAuth, scopes, or token custody.

pub mod commands;
pub mod types;

use crate::domain::types::AppError;
use std::path::{Path, PathBuf};

pub const OBSIDIAN_VAULT_CHANGED_EVENT: &str = "june://obsidian-vault-changed";

fn display_name_for_root(root: &Path) -> String {
    root.file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Obsidian vault")
        .to_string()
}

/// Canonicalize and validate a candidate local vault root. V1 requires the
/// `.obsidian/` marker so this plugin remains graph-vault-specific rather than
/// becoming generic Markdown folder support.
fn validate_vault_root(root_path: &str) -> Result<PathBuf, AppError> {
    let trimmed = root_path.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(
            "vault_path_invalid",
            "Choose an Obsidian vault folder.",
        ));
    }
    let root = PathBuf::from(trimmed);
    let canonical = root.canonicalize().map_err(|_| {
        AppError::new(
            "vault_unavailable",
            "The selected vault folder could not be opened.",
        )
    })?;
    if !canonical.is_dir() {
        return Err(AppError::new(
            "vault_path_invalid",
            "Choose an Obsidian vault folder.",
        ));
    }
    if !canonical.join(".obsidian").is_dir() {
        return Err(AppError::new(
            "vault_marker_missing",
            "Choose a folder that contains an .obsidian directory.",
        ));
    }
    Ok(canonical)
}

/// Stable-enough root identity for the foundation slice. Follow-up path-safety
/// work replaces this with platform file ids before any MCP path access lands.
fn root_identity(root: &Path) -> Result<String, AppError> {
    let metadata = root.metadata().map_err(|_| {
        AppError::new(
            "vault_unavailable",
            "The selected vault folder could not be inspected.",
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!("unix:{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        Ok(format!(
            "metadata:{}:{}",
            metadata.len(),
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ))
    }
}
