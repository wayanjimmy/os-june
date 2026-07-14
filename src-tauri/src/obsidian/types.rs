use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObsidianVaultHealth {
    NoVaultSelected,
    Indexing,
    Healthy,
    Missing,
    Unreadable,
    PermissionDenied,
    RootChanged,
    PartialIndex,
    CloudFilesUnavailable,
    WatcherDegraded,
    Rebuilding,
    WriteConflictDetected,
}

impl ObsidianVaultHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoVaultSelected => "no_vault_selected",
            Self::Indexing => "indexing",
            Self::Healthy => "healthy",
            Self::Missing => "missing",
            Self::Unreadable => "unreadable",
            Self::PermissionDenied => "permission_denied",
            Self::RootChanged => "root_changed",
            Self::PartialIndex => "partial_index",
            Self::CloudFilesUnavailable => "cloud_files_unavailable",
            Self::WatcherDegraded => "watcher_degraded",
            Self::Rebuilding => "rebuilding",
            Self::WriteConflictDetected => "write_conflict_detected",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "healthy" => Self::Healthy,
            "missing" => Self::Missing,
            "unreadable" => Self::Unreadable,
            "permission_denied" => Self::PermissionDenied,
            "root_changed" => Self::RootChanged,
            "partial_index" => Self::PartialIndex,
            "cloud_files_unavailable" => Self::CloudFilesUnavailable,
            "watcher_degraded" => Self::WatcherDegraded,
            "rebuilding" => Self::Rebuilding,
            "write_conflict_detected" => Self::WriteConflictDetected,
            "no_vault_selected" => Self::NoVaultSelected,
            _ => Self::Indexing,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianVaultGrantDto {
    pub vault_id: String,
    pub display_name: String,
    /// Canonical roots stay Rust-side. This is a path suitable for display,
    /// currently the same local path until the frontend has a richer path
    /// presentation type; it is never sent to Hermes or MCP servers.
    pub display_path: String,
    pub write_enabled: bool,
    pub status: ObsidianVaultHealth,
    pub created_at: String,
    pub updated_at: String,
    pub last_checked_at: Option<String>,
    pub last_scan_started_at: Option<String>,
    pub last_scan_completed_at: Option<String>,
    pub last_successful_scan_at: Option<String>,
    pub index_version: i64,
    pub note_count: i64,
    pub tag_count: i64,
    pub unresolved_link_count: i64,
    pub ambiguous_link_count: i64,
    pub placeholder_file_count: i64,
    pub skipped_file_count: i64,
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianVaultStatusDto {
    pub grant: Option<ObsidianVaultGrantDto>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianVaultConfirmRequest {
    pub root_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianVaultWriteModeRequest {
    pub vault_id: String,
    pub write_enabled: bool,
}
