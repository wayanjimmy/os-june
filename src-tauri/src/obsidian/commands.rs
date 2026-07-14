use crate::{
    db::repositories::ObsidianVaultGrantRecord,
    domain::types::AppError,
    obsidian::{
        display_name_for_root, root_identity, types::*, validate_vault_root,
        OBSIDIAN_VAULT_CHANGED_EVENT,
    },
};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

#[tauri::command]
pub async fn obsidian_vault_status(app: AppHandle) -> Result<ObsidianVaultStatusDto, AppError> {
    let repos = crate::commands::repositories(&app).await?;
    let grant = repos.get_obsidian_vault_grant().await?.map(grant_dto);
    Ok(ObsidianVaultStatusDto { grant })
}

/// Confirm and persist one local vault grant. The native folder picker lives in
/// the frontend; Rust revalidates the path and stores only local authority
/// metadata. No note content is read in this foundation slice.
#[tauri::command]
pub async fn obsidian_vault_confirm(
    app: AppHandle,
    request: ObsidianVaultConfirmRequest,
) -> Result<ObsidianVaultGrantDto, AppError> {
    let root = validate_vault_root(&request.root_path)?;
    let identity = root_identity(&root)?;
    let display_name = display_name_for_root(&root);
    let repos = crate::commands::repositories(&app).await?;
    let record = repos
        .set_obsidian_vault_grant(
            &Uuid::new_v4().to_string(),
            &display_name,
            &root.to_string_lossy(),
            &identity,
        )
        .await?;
    emit_changed(&app);
    Ok(grant_dto(record))
}

#[tauri::command]
pub async fn obsidian_vault_remove(app: AppHandle, vault_id: String) -> Result<(), AppError> {
    let repos = crate::commands::repositories(&app).await?;
    let removed = repos.delete_obsidian_vault_grant(&vault_id).await?;
    if !removed {
        return Err(AppError::new(
            "vault_not_configured",
            "No matching Obsidian vault is configured.",
        ));
    }
    emit_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn obsidian_vault_set_write_mode(
    app: AppHandle,
    request: ObsidianVaultWriteModeRequest,
) -> Result<ObsidianVaultGrantDto, AppError> {
    let repos = crate::commands::repositories(&app).await?;
    let record = repos
        .set_obsidian_vault_write_enabled(&request.vault_id, request.write_enabled)
        .await?
        .ok_or_else(|| {
            AppError::new(
                "vault_not_configured",
                "No matching Obsidian vault is configured.",
            )
        })?;
    emit_changed(&app);
    Ok(grant_dto(record))
}

fn emit_changed(app: &AppHandle) {
    let _ = app.emit(OBSIDIAN_VAULT_CHANGED_EVENT, ());
}

fn grant_dto(record: ObsidianVaultGrantRecord) -> ObsidianVaultGrantDto {
    ObsidianVaultGrantDto {
        vault_id: record.vault_id,
        display_name: record.display_name,
        display_path: record.canonical_root,
        write_enabled: record.write_enabled,
        status: ObsidianVaultHealth::from_db(&record.status),
        created_at: record.created_at,
        updated_at: record.updated_at,
        last_checked_at: record.last_checked_at,
        last_scan_started_at: record.last_scan_started_at,
        last_scan_completed_at: record.last_scan_completed_at,
        last_successful_scan_at: record.last_successful_scan_at,
        index_version: record.index_version,
        note_count: record.note_count,
        tag_count: record.tag_count,
        unresolved_link_count: record.unresolved_link_count,
        ambiguous_link_count: record.ambiguous_link_count,
        placeholder_file_count: record.placeholder_file_count,
        skipped_file_count: record.skipped_file_count,
        last_error_code: record.last_error_code,
    }
}
