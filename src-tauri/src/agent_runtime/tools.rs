use super::{AgentRepository, AgentSafetyMode};
use crate::domain::types::AppError;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{query::query, row::Row};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
};
use tauri::{AppHandle, Manager};
use tokio::{io::AsyncReadExt, process::Command, sync::oneshot};

const MAX_TOOL_OUTPUT_BYTES: usize = 1_048_576;
const MAX_SEARCH_FILE_BYTES: u64 = 1_048_576;
const MAX_SEARCH_SCANNED_BYTES: u64 = 32 * 1_048_576;
const MAX_SEARCH_FILES: usize = 10_000;
const MAX_SEARCH_MATCHES: usize = 200;

#[derive(Clone)]
pub struct ToolContext {
    pub app: AppHandle,
    pub repository: AgentRepository,
    pub workspace: PathBuf,
    pub safety_mode: AgentSafetyMode,
    pub session_id: String,
    pub run_id: String,
    pub cancellations: ToolCancellationRegistry,
    pub call_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct ToolCancellationRegistry {
    inner: std::sync::Arc<std::sync::Mutex<CancellationSenders>>,
    next_id: std::sync::Arc<AtomicU64>,
}

type CancellationSenders = std::collections::HashMap<String, Vec<(u64, oneshot::Sender<()>)>>;

pub(crate) struct ToolCancellationRegistration {
    receiver: oneshot::Receiver<()>,
    inner: std::sync::Arc<std::sync::Mutex<CancellationSenders>>,
    run_id: String,
    id: u64,
}

impl Drop for ToolCancellationRegistration {
    fn drop(&mut self) {
        let mut entries = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let remove_run = if let Some(senders) = entries.get_mut(&self.run_id) {
            senders.retain(|(id, _)| *id != self.id);
            senders.is_empty()
        } else {
            false
        };
        if remove_run {
            entries.remove(&self.run_id);
        }
    }
}

impl ToolCancellationRegistry {
    pub(crate) async fn register(&self, run_id: &str) -> ToolCancellationRegistration {
        let (send, receive) = oneshot::channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(run_id.to_string())
            .or_default()
            .push((id, send));
        ToolCancellationRegistration {
            receiver: receive,
            inner: self.inner.clone(),
            run_id: run_id.to_string(),
            id,
        }
    }
    pub async fn cancel(&self, run_id: &str) {
        let senders = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(run_id);
        if let Some(senders) = senders {
            for (_, sender) in senders {
                let _ = sender.send(());
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn registration_count(&self, run_id: &str) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(run_id)
            .map_or(0, Vec::len)
    }
}

impl ToolCancellationRegistration {
    pub(crate) async fn cancelled(&mut self) {
        let _ = (&mut self.receiver).await;
    }
}

pub async fn dispatch_tool(
    context: &ToolContext,
    name: &str,
    arguments: Value,
) -> Result<Value, AppError> {
    if !name.starts_with("mcp_")
        && crate::routines::routine_tool_allowed_for_session(
            &context.repository.pool,
            &context.session_id,
            name,
        )
        .await?
            == Some(false)
    {
        return Err(AppError::new(
            "routine_tool_not_enabled",
            "This tool is not enabled for the routine.",
        ));
    }
    if let Some(result) =
        crate::agent_runtime::native_connectors::dispatch(&context.app, name, arguments.clone())
            .await?
    {
        return Ok(result);
    }
    match name {
        "search_june" => search_june(context, &arguments).await,
        "list_memories" => list_memories(context, &arguments).await,
        "save_memory" => save_memory(context, &arguments).await,
        "forget_memory" => forget_memory(context, &arguments).await,
        "generate_image" => generate_image(context, &arguments).await,
        "edit_image" => edit_image(context, &arguments).await,
        "generate_video" => generate_video(context, &arguments).await,
        "get_obsidian_vault" => Ok(serde_json::to_value(crate::obsidian::discovery_for_app(
            &context.app,
        )?)
        .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string()))?),
        "start_recording" => {
            let source_mode = match arguments.get("sourceMode").and_then(Value::as_str) {
                Some("microphonePlusSystem") => {
                    crate::domain::types::RecordingSourceMode::MicrophonePlusSystem
                }
                _ => crate::domain::types::RecordingSourceMode::MicrophoneOnly,
            };
            context
                .app
                .state::<crate::agent_recorder::AgentRecorderBroker>()
                .request(
                    &context.app,
                    crate::agent_recorder::AgentRecorderAction::Start,
                    Some(source_mode),
                )
                .await
        }
        "stop_recording" => {
            context
                .app
                .state::<crate::agent_recorder::AgentRecorderBroker>()
                .request(
                    &context.app,
                    crate::agent_recorder::AgentRecorderAction::Stop,
                    None,
                )
                .await
        }
        "recording_status" => serde_json::to_value(crate::audio::capture::current_status())
            .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string())),
        "web_search" => web(context, "/v1/web/search", &arguments).await,
        "web_fetch" => web(context, "/v1/web/fetch", &arguments).await,
        "list_files" => list_files(context, &arguments).await,
        "read_file" => read_file(context, &arguments).await,
        "write_file" => write_file(context, &arguments).await,
        "patch_file" => patch_file(context, &arguments).await,
        "replace_file" => replace_file(context, &arguments).await,
        "import_file" => import_file(context, &arguments).await,
        "preview_file" => preview_file(context, &arguments).await,
        "search_files" => search_files(context, &arguments).await,
        "run_shell" => run_shell(context, &arguments).await,
        "list_skills" => list_skills(context).await,
        "load_skill" => load_skill(context, &arguments).await,
        "list_routines" => {
            serde_json::to_value(crate::routines::list(&context.repository.pool).await?)
                .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string()))
        }
        "create_routine" => create_routine(context, arguments).await,
        "update_routine" => update_routine(context, arguments).await,
        "pause_routine" => pause_routine(context, &arguments).await,
        "resume_routine" => resume_routine(context, &arguments).await,
        "delete_routine" => {
            let routine_id = required_string(&arguments, "routineId")?;
            crate::routines::delete(&context.repository.pool, routine_id).await?;
            Ok(json!({ "deleted": true, "routineId": routine_id }))
        }
        "request_clarification" => consume_clarification_answer(context).await,
        "request_secret" => consume_secret_reference(context).await,
        "computer_use" => {
            Ok(crate::computer_use::handle_proxy_action(&context.app, arguments).await)
        }
        "notion_call" | "notion_action" => notion_tool(context, name, &arguments).await,
        name if matches!(
            name,
            "start_session"
                | "close_session"
                | "navigate"
                | "snapshot"
                | "screenshot"
                | "click"
                | "fill"
                | "press"
                | "back"
                | "list_tabs"
                | "open_tab"
                | "switch_tab"
                | "close_tab"
                | "accept_shared_tab"
        ) =>
        {
            let broker = context
                .app
                .state::<std::sync::Arc<crate::browser_broker::BrowserBroker>>();
            broker
                .execute_for(
                    crate::browser_broker::BrowserBrokerContext::Attended,
                    name,
                    arguments,
                )
                .await
        }
        name if name.starts_with("mcp_") => mcp_tool(context, name, arguments).await,
        // These capabilities stay behind Rust-owned seams. Their existing brokers
        // can be connected without granting the runtime direct credentials or UI access.
        name if name.starts_with("browser_")
            || name.starts_with("computer_")
            || name.starts_with("connector_") =>
        {
            Err(AppError::new(
                "agent_tool_unavailable",
                format!("{name} is not enabled for this runtime yet."),
            ))
        }
        _ => Err(AppError::new(
            "agent_tool_unsupported",
            format!("Unsupported agent tool: {name}"),
        )),
    }
}

async fn create_routine(context: &ToolContext, arguments: Value) -> Result<Value, AppError> {
    let request: crate::routines::CreateAgentRoutineRequest = serde_json::from_value(arguments)
        .map_err(|error| AppError::new("invalid_arguments", error.to_string()))?;
    serde_json::to_value(crate::routines::create(&context.repository.pool, request).await?)
        .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string()))
}

async fn update_routine(context: &ToolContext, arguments: Value) -> Result<Value, AppError> {
    let request: crate::routines::UpdateAgentRoutineRequest = serde_json::from_value(arguments)
        .map_err(|error| AppError::new("invalid_arguments", error.to_string()))?;
    serde_json::to_value(crate::routines::update(&context.repository.pool, request).await?)
        .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string()))
}

async fn pause_routine(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let routine_id = required_string(arguments, "routineId")?;
    serde_json::to_value(crate::routines::pause(&context.repository.pool, routine_id).await?)
        .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string()))
}

async fn resume_routine(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let routine_id = required_string(arguments, "routineId")?;
    serde_json::to_value(crate::routines::resume(&context.repository.pool, routine_id).await?)
        .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string()))
}

async fn mcp_tool(context: &ToolContext, name: &str, arguments: Value) -> Result<Value, AppError> {
    let subsystem = crate::agent_mcp::AgentMcpSubsystem::new(
        crate::agent_mcp::AgentMcpRepository::new(context.repository.pool.clone()),
        crate::agent_mcp::KeychainMcpSecretStore,
    );
    subsystem
        .refresh_registry_for_workspace(
            context.safety_mode == AgentSafetyMode::Sandboxed,
            Some(&context.workspace),
        )
        .await
        .map_err(agent_mcp_error)?;
    let server_name = subsystem
        .server_name_for_tool(name)
        .map_err(agent_mcp_error)?
        .ok_or_else(|| AppError::new("agent_mcp_tool_failed", "MCP tool is unavailable."))?;
    if crate::routines::routine_mcp_server_allowed_for_session(
        &context.repository.pool,
        &context.session_id,
        &server_name,
    )
    .await?
        == Some(false)
    {
        return Err(AppError::new(
            "routine_tool_not_enabled",
            "This MCP server is not enabled for the routine.",
        ));
    }
    let current_policy = subsystem
        .policy_for_tool(name)
        .map_err(agent_mcp_error)?
        .ok_or_else(|| AppError::new("agent_mcp_tool_failed", "MCP tool is unavailable."))?;
    if !crate::agent_mcp::run_policy_matches(
        &context.repository.pool,
        &context.run_id,
        name,
        &current_policy,
    )
    .await
    .map_err(agent_mcp_error)?
    {
        return Err(AppError::new(
            "agent_mcp_policy_changed",
            "This MCP server changed after the run started. Retry the turn to use its current approval policy.",
        ));
    }
    let elicitation_answer = latest_mcp_elicitation_answer(context).await?;
    let invocation = subsystem.invoke_in_workspace_with_elicitation(
        name,
        arguments,
        context.safety_mode == AgentSafetyMode::Sandboxed,
        Some(&context.workspace),
        elicitation_answer
            .as_ref()
            .map(|(_, answer)| answer.as_str()),
    );
    let mut cancelled = context.cancellations.register(&context.run_id).await;
    tokio::select! {
        result = invocation => match result {
            Ok(value) => {
                if let Some((item_id, _)) = elicitation_answer {
                    mark_mcp_elicitation_answer_consumed(context, &item_id).await?;
                }
                Ok(value)
            }
            Err(crate::agent_mcp::AgentMcpError::ElicitationRequired(message)) => Ok(json!({
                "elicitationRequired": true,
                "clarificationQuestion": format!("MCP server {server_name} asks: {message}"),
                "instruction": "Call request_clarification with clarificationQuestion exactly, then retry this MCP tool with the same arguments."
            })),
            Err(error) => Err(agent_mcp_error(error)),
        },
        _ = &mut cancelled.receiver => {
            crate::agent_mcp::retire_server_sessions(&current_policy.server_id).await;
            Err(AppError::new("agent_tool_cancelled", "MCP tool call was cancelled."))
        }
    }
}

async fn latest_mcp_elicitation_answer(
    context: &ToolContext,
) -> Result<Option<(String, String)>, AppError> {
    let row = query("SELECT id, payload_json FROM agent_items WHERE run_id = ? AND kind = 'interruption' AND json_extract(payload_json, '$.kind') = 'clarification' AND json_extract(payload_json, '$.question') LIKE 'MCP server % asks: %' AND json_extract(payload_json, '$.answer') IS NOT NULL AND COALESCE(json_extract(payload_json, '$.mcpAnswerConsumed'), 0) = 0 ORDER BY created_at DESC LIMIT 1")
        .bind(&context.run_id)
        .fetch_optional(&context.repository.pool)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let payload: Value = serde_json::from_str(&row.get::<String, _>("payload_json"))
        .map_err(|error| AppError::new("agent_interruption_invalid", error.to_string()))?;
    Ok(payload
        .get("answer")
        .and_then(Value::as_str)
        .map(|answer| (row.get("id"), answer.to_string())))
}

async fn mark_mcp_elicitation_answer_consumed(
    context: &ToolContext,
    item_id: &str,
) -> Result<(), AppError> {
    let row = query("SELECT payload_json FROM agent_items WHERE id = ? AND run_id = ?")
        .bind(item_id)
        .bind(&context.run_id)
        .fetch_one(&context.repository.pool)
        .await?;
    let mut payload: Value = serde_json::from_str(&row.get::<String, _>("payload_json"))
        .map_err(|error| AppError::new("agent_interruption_invalid", error.to_string()))?;
    payload["mcpAnswerConsumed"] = Value::Bool(true);
    query("UPDATE agent_items SET payload_json = ? WHERE id = ?")
        .bind(payload.to_string())
        .bind(item_id)
        .execute(&context.repository.pool)
        .await?;
    Ok(())
}

fn agent_mcp_error(error: crate::agent_mcp::AgentMcpError) -> AppError {
    AppError::new("agent_mcp_tool_failed", error.to_string())
}

async fn search_june(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let query_text = required_string(arguments, "query")?;
    let profile = crate::commands::active_profile(&context.app);
    search_june_for_profile(&context.repository.pool, &profile, query_text).await
}

async fn search_june_for_profile(
    pool: &sqlx_sqlite::SqlitePool,
    profile: &str,
    query_text: &str,
) -> Result<Value, AppError> {
    let pattern = format!("%{}%", query_text.replace('%', "\\%").replace('_', "\\_"));
    let rows = query(
        "SELECT n.id, n.title, COALESCE(n.edited_content, n.generated_content, '') AS note,
                COALESCE((SELECT text FROM transcripts t WHERE t.note_id = n.id ORDER BY t.created_at DESC LIMIT 1), '') AS transcript,
                n.updated_at
         FROM notes n
         WHERE n.profile = ? AND (
            n.title LIKE ? ESCAPE '\\' OR n.generated_content LIKE ? ESCAPE '\\'
            OR n.edited_content LIKE ? ESCAPE '\\'
            OR EXISTS (SELECT 1 FROM transcripts t WHERE t.note_id = n.id AND t.text LIKE ? ESCAPE '\\')
         )
         ORDER BY n.updated_at DESC LIMIT 20",
    )
    .bind(profile).bind(&pattern).bind(&pattern).bind(&pattern).bind(&pattern)
    .fetch_all(pool).await?;
    let notes: Vec<Value> = rows.into_iter().map(|row| json!({
        "id": row.get::<String, _>("id"), "title": row.get::<String, _>("title"),
        "note": truncate(row.get::<String, _>("note")), "transcript": truncate(row.get::<String, _>("transcript")),
        "updatedAt": row.get::<String, _>("updated_at")
    })).collect();
    let dictations = query("SELECT id, text, language, created_at FROM dictation_history WHERE profile = ? AND text LIKE ? ESCAPE '\\' ORDER BY created_at DESC LIMIT 20")
        .bind(profile).bind(&pattern).fetch_all(pool).await?;
    let dictations: Vec<Value> = dictations.into_iter().map(|row| json!({
        "id": row.get::<String, _>("id"), "text": truncate(row.get::<String, _>("text")),
        "language": row.get::<Option<String>, _>("language"), "createdAt": row.get::<String, _>("created_at")
    })).collect();
    Ok(json!({ "notes": notes, "dictations": dictations }))
}

async fn list_memories(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let settings_path = crate::commands::memory_settings_path(&context.app)?;
    if !crate::commands::load_memory_settings(&settings_path).enabled {
        return Err(AppError::new(
            "memory_disabled",
            "Memory is disabled for this scope.",
        ));
    }
    let project_id = optional_nonempty_string(arguments, "projectId")?;
    ensure_memory_scope_enabled(context, project_id.as_deref()).await?;
    let include_global = arguments
        .get("includeGlobal")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(8)
        .clamp(1, 20) as usize;
    let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let profile = crate::commands::active_profile(&context.app);
    let repositories = crate::commands::repositories(&context.app).await?;
    let memories = repositories
        .list_memories(&profile, project_id.as_deref(), include_global)
        .await?;
    let has_more = memories.len() > offset.saturating_add(limit);
    let items = memories
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|memory| {
            json!({
                "id": memory.id,
                "content": memory.content,
                "createdAt": memory.created_at,
                "scope": if memory.folder_id.is_some() { "project" } else { "global" }
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "count": items.len(),
        "items": items,
        "offset": offset,
        "hasMore": has_more,
        "nextOffset": has_more.then_some(offset + items.len())
    }))
}

async fn save_memory(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let content = required_string(arguments, "content")?;
    let project_id = optional_nonempty_string(arguments, "projectId")?;
    let profile = crate::commands::active_profile(&context.app);
    let repositories = crate::commands::repositories(&context.app).await?;
    let settings_path = crate::commands::memory_settings_path(&context.app)?;
    let memory = crate::commands::create_memory_with_settings(
        &repositories,
        &settings_path,
        &profile,
        project_id.as_deref(),
        content,
        "agent",
    )
    .await?;
    serde_json::to_value(memory)
        .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string()))
}

async fn forget_memory(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let id = required_string(arguments, "id")?;
    let profile = crate::commands::active_profile(&context.app);
    crate::commands::repositories(&context.app)
        .await?
        .delete_memory(&profile, id)
        .await?;
    Ok(json!({ "forgotten": true, "id": id }))
}

async fn generate_image(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let prompt = required_string(arguments, "prompt")?.trim();
    let generated = crate::providers::generate_image(crate::providers::GenerateImageRequest {
        prompt: prompt.to_string(),
        model: None,
        request_id: context.call_id.clone(),
        safe_mode: None,
    })
    .await?;
    persist_generated_image(context, prompt, generated).await
}

async fn edit_image(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let source = resolve_read_path(context, required_string(arguments, "sourcePath")?)?;
    let instruction = required_string(arguments, "instruction")?.trim();
    let bytes = tokio::fs::read(&source).await.map_err(io_error)?;
    if bytes.len() > 20 * 1024 * 1024 {
        return Err(AppError::new(
            "image_source_too_large",
            "The source image exceeds the 20 MB edit limit.",
        ));
    }
    let mime_type = image_mime_type(&source).ok_or_else(|| {
        AppError::new(
            "image_source_invalid",
            "The source must be a PNG, JPEG, WebP, or GIF image.",
        )
    })?;
    let generated = crate::providers::edit_image(crate::providers::EditImageRequest {
        image: BASE64.encode(bytes),
        prompt: instruction.to_string(),
        request_id: context.call_id.clone(),
        mime_type: Some(mime_type.to_string()),
        model: None,
    })
    .await?;
    persist_generated_image(context, instruction, generated).await
}

async fn generate_video(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let prompt = required_string(arguments, "prompt")?.trim();
    let job = crate::providers::video_generate(crate::providers::GenerateVideoRequest {
        prompt: prompt.to_string(),
        model: None,
        request_id: context.call_id.clone(),
        duration: optional_nonempty_string(arguments, "duration")?,
        resolution: None,
        aspect_ratio: optional_nonempty_string(arguments, "aspectRatio")?,
        audio: arguments.get("audio").and_then(Value::as_bool),
    })
    .await?;
    let mut cancelled = context.cancellations.register(&context.run_id).await;
    loop {
        let status = crate::providers::video_status(
            context.app.clone(),
            crate::providers::VideoStatusRequest {
                job_id: job.job_id.clone(),
            },
        );
        let result = tokio::select! {
            result = status => result?,
            _ = &mut cancelled.receiver => {
                return Err(AppError::new("agent_tool_cancelled", "Video generation was cancelled."));
            }
        };
        match result {
            crate::june_api::VideoStatusDto::Processing { .. } => {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                    _ = &mut cancelled.receiver => {
                        return Err(AppError::new("agent_tool_cancelled", "Video generation was cancelled."));
                    }
                }
            }
            crate::june_api::VideoStatusDto::Failed { reason } => {
                return Err(AppError::new("video_generation_failed", reason));
            }
            crate::june_api::VideoStatusDto::Completed {
                path,
                mime_type,
                model,
                ..
            } => {
                let source = PathBuf::from(path);
                let destination = resolve_write_path(
                    context,
                    &format!(
                        "artifacts/generated-video-{}.mp4",
                        uuid::Uuid::new_v4().simple()
                    ),
                    false,
                )?;
                if let Some(parent) = destination.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(io_error)?;
                }
                tokio::fs::copy(&source, &destination)
                    .await
                    .map_err(io_error)?;
                record_artifact_with_mime(context, &destination, "created", None, &mime_type)
                    .await?;
                return Ok(json!({
                    "mediaType": "video",
                    "path": destination,
                    "mimeType": mime_type,
                    "model": model,
                    "prompt": prompt,
                    "requestId": context.call_id,
                    "name": destination.file_name().map(|name| name.to_string_lossy().into_owned())
                }));
            }
        }
    }
}

async fn persist_generated_image(
    context: &ToolContext,
    prompt: &str,
    generated: crate::june_api::GeneratedImageDto,
) -> Result<Value, AppError> {
    let bytes = BASE64
        .decode(&generated.image_base64)
        .map_err(|error| AppError::new("image_response_invalid", error.to_string()))?;
    let extension = match generated.mime_type.as_str() {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    };
    let path = resolve_write_path(
        context,
        &format!(
            "artifacts/generated-image-{}.{}",
            uuid::Uuid::new_v4().simple(),
            extension
        ),
        false,
    )?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(io_error)?;
    }
    tokio::fs::write(&path, &bytes).await.map_err(io_error)?;
    record_artifact_with_mime(context, &path, "created", None, &generated.mime_type).await?;
    Ok(json!({
        "mediaType": "image",
        "dataUrl": format!("data:{};base64,{}", generated.mime_type, generated.image_base64),
        "path": path,
        "mimeType": generated.mime_type,
        "model": generated.model,
        "prompt": prompt,
        "name": path.file_name().map(|name| name.to_string_lossy().into_owned())
    }))
}

fn image_mime_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("gif") => Some("image/gif"),
        _ => None,
    }
}

async fn ensure_memory_scope_enabled(
    context: &ToolContext,
    project_id: Option<&str>,
) -> Result<(), AppError> {
    let Some(project_id) = project_id else {
        return Ok(());
    };
    let profile = crate::commands::active_profile(&context.app);
    let row = sqlx::query::query(
        "SELECT memory_disabled FROM folders
         WHERE id = ? AND profile = ? AND deleted_at IS NULL",
    )
    .bind(project_id)
    .bind(profile)
    .fetch_optional(&context.repository.pool)
    .await?
    .ok_or_else(|| {
        AppError::new(
            "folder_not_found",
            "Project was not found or has already been deleted.",
        )
    })?;
    if row.get::<i64, _>("memory_disabled") != 0 {
        return Err(AppError::new(
            "memory_disabled",
            "Memory is disabled for this scope.",
        ));
    }
    Ok(())
}

fn optional_nonempty_string(arguments: &Value, key: &str) -> Result<Option<String>, AppError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let value = value.as_str().ok_or_else(|| {
        AppError::new(
            "invalid_arguments",
            format!("{key} must be a non-empty string."),
        )
    })?;
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::new(
            "invalid_arguments",
            format!("{key} must be a non-empty string."),
        ));
    }
    Ok(Some(value.to_string()))
}

async fn web(context: &ToolContext, path: &str, arguments: &Value) -> Result<Value, AppError> {
    let request = web_request(arguments, context.call_id.as_deref());
    let response = crate::june_api::forward_web_request(path, &request).await?;
    if response.status >= 400 {
        return Err(AppError::new(
            "agent_web_failed",
            String::from_utf8_lossy(&response.body).into_owned(),
        ));
    }
    serde_json::from_slice(&response.body)
        .map_err(|error| AppError::new("agent_web_invalid_response", error.to_string()))
}

fn web_request(arguments: &Value, call_id: Option<&str>) -> Value {
    let mut request = arguments.clone();
    request["requestId"] = Value::String(
        call_id
            .map(str::to_string)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
    );
    request
}

async fn list_files(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let path = resolve_read_path(
        context,
        arguments.get("path").and_then(Value::as_str).unwrap_or("."),
    )?;
    let mut entries = tokio::fs::read_dir(&path).await.map_err(io_error)?;
    let mut result = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(io_error)? {
        let metadata = entry.metadata().await.map_err(io_error)?;
        result.push(json!({ "name": entry.file_name().to_string_lossy(), "path": entry.path(), "directory": metadata.is_dir(), "sizeBytes": metadata.len() }));
        if result.len() >= 500 {
            break;
        }
    }
    Ok(json!({ "path": path, "entries": result }))
}

async fn read_file(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let path = resolve_read_path(context, required_string(arguments, "path")?)?;
    let bytes = tokio::fs::read(&path).await.map_err(io_error)?;
    if bytes.len() > MAX_TOOL_OUTPUT_BYTES {
        return Err(AppError::new(
            "agent_tool_output_too_large",
            "File exceeds the 1 MB read limit.",
        ));
    }
    let revision = file_revision(&bytes);
    let line_ending = detect_line_ending(&bytes).as_str();
    let content = String::from_utf8(bytes)
        .map_err(|_| AppError::new("agent_file_not_text", "File is not UTF-8 text."))?;
    Ok(json!({ "path": path, "content": content, "revision": revision, "lineEnding": line_ending }))
}

async fn write_file(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let path = resolve_write_path(context, required_string(arguments, "path")?, false)?;
    let content = string_argument(arguments, "content")?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(io_error)?;
    }
    create_text_file(&path, content.as_bytes())?;
    let artifact_recorded = record_artifact_best_effort(context, &path, "created").await;
    Ok(
        json!({ "path": path, "sizeBytes": content.len(), "revision": file_revision(content.as_bytes()), "artifactRecorded": artifact_recorded }),
    )
}

async fn patch_file(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let path = resolve_write_path(context, required_string(arguments, "path")?, true)?;
    let before = required_string(arguments, "before")?;
    let after = string_argument(arguments, "after")?;
    let result = patch_text_file(&path, before, after)?;
    let artifact_recorded = record_artifact_best_effort(context, &path, "updated").await;
    Ok(
        json!({ "path": path, "updated": true, "sizeBytes": result.size_bytes, "revision": result.revision, "artifactRecorded": artifact_recorded }),
    )
}

async fn replace_file(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let path = resolve_write_path(context, required_string(arguments, "path")?, true)?;
    let content = string_argument(arguments, "content")?;
    let expected_revision = required_string(arguments, "expectedRevision")?;
    let result = replace_text_file(&path, content, expected_revision)?;
    let artifact_recorded = record_artifact_best_effort(context, &path, "updated").await;
    Ok(
        json!({ "path": path, "updated": true, "sizeBytes": result.size_bytes, "revision": result.revision, "artifactRecorded": artifact_recorded }),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineEnding {
    Lf,
    Crlf,
    Mixed,
    None,
}

impl LineEnding {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "lf",
            Self::Crlf => "crlf",
            Self::Mixed => "mixed",
            Self::None => "none",
        }
    }
}

#[derive(Debug)]
struct MutationResult {
    size_bytes: usize,
    revision: String,
}

fn file_revision(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn valid_revision(revision: &str) -> bool {
    revision.len() == 71
        && revision.starts_with("sha256:")
        && revision[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn detect_line_ending(bytes: &[u8]) -> LineEnding {
    let mut lf = false;
    let mut crlf = false;
    let mut standalone_cr = false;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if index > 0 && bytes[index - 1] == b'\r' {
                crlf = true;
            } else {
                lf = true;
            }
        } else if *byte == b'\r' && bytes.get(index + 1) != Some(&b'\n') {
            standalone_cr = true;
        }
    }
    match (lf, crlf, standalone_cr) {
        (false, false, false) => LineEnding::None,
        (true, false, false) => LineEnding::Lf,
        (false, true, false) => LineEnding::Crlf,
        _ => LineEnding::Mixed,
    }
}

fn create_text_file(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::new("agent_file_write_failed", "File has no parent directory."))?;
    let temp_path = parent.join(format!(".june-create-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(io_error)?;
    let result = file
        .write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(io_error)
        .and_then(|()| {
            drop(file);
            crate::filesystem::publish_new_file(&temp_path, path).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists
                    || path.try_exists().unwrap_or(false)
                {
                    AppError::new("agent_file_exists", "The file already exists.")
                } else {
                    io_error(error)
                }
            })
        });
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn normalize_newlines(content: &str, line_ending: LineEnding) -> String {
    match line_ending {
        LineEnding::Lf => content.replace("\r\n", "\n"),
        LineEnding::Crlf => content.replace("\r\n", "\n").replace('\n', "\r\n"),
        LineEnding::Mixed | LineEnding::None => content.to_string(),
    }
}

fn replace_text_file(
    path: &Path,
    content: &str,
    expected_revision: &str,
) -> Result<MutationResult, AppError> {
    if !valid_revision(expected_revision) {
        return Err(AppError::new(
            "agent_file_revision_invalid",
            "expectedRevision must be an exact sha256 revision returned by read_file.",
        ));
    }
    let original = fs::read(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::new("agent_file_not_found", "The file does not exist.")
        } else {
            io_error(error)
        }
    })?;
    if file_revision(&original) != expected_revision {
        return Err(AppError::new(
            "agent_file_revision_conflict",
            "The file changed since June read it. Read it again before replacing it.",
        ));
    }
    std::str::from_utf8(&original)
        .map_err(|_| AppError::new("agent_file_not_text", "File is not UTF-8 text."))?;
    let line_ending = detect_line_ending(&original);
    if line_ending == LineEnding::Mixed {
        return Err(AppError::new(
            "agent_file_line_endings_mixed",
            "The file has mixed line endings and cannot be safely replaced.",
        ));
    }
    let has_bom = original.starts_with(&[0xef, 0xbb, 0xbf]);
    let normalized = normalize_newlines(content.trim_start_matches('\u{feff}'), line_ending);
    let mut replacement = Vec::with_capacity(normalized.len() + usize::from(has_bom) * 3);
    if has_bom {
        replacement.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    replacement.extend_from_slice(normalized.as_bytes());
    stage_and_replace(path, &replacement, expected_revision)
}

fn patch_text_file(path: &Path, before: &str, after: &str) -> Result<MutationResult, AppError> {
    let original = fs::read(path).map_err(io_error)?;
    let original_revision = file_revision(&original);
    let has_bom = original.starts_with(&[0xef, 0xbb, 0xbf]);
    let body = if has_bom { &original[3..] } else { &original };
    let content = std::str::from_utf8(body)
        .map_err(|_| AppError::new("agent_file_not_text", "File is not UTF-8 text."))?;
    let line_ending = detect_line_ending(&original);
    let before = normalize_newlines(before.trim_start_matches('\u{feff}'), line_ending);
    let after = normalize_newlines(after.trim_start_matches('\u{feff}'), line_ending);
    let occurrences = content.matches(&before).count();
    if occurrences != 1 {
        return Err(AppError::new(
            "agent_patch_ambiguous",
            format!("Patch target occurred {occurrences} times. Reread the file and use a fresh, smaller exact patch that occurs once."),
        ));
    }
    let replaced = content.replacen(&before, &after, 1);
    let mut replacement = Vec::with_capacity(replaced.len() + usize::from(has_bom) * 3);
    if has_bom {
        replacement.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    replacement.extend_from_slice(replaced.as_bytes());
    stage_and_replace(path, &replacement, &original_revision)
}

fn stage_and_replace(
    path: &Path,
    replacement: &[u8],
    expected_revision: &str,
) -> Result<MutationResult, AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::new("agent_file_write_failed", "File has no parent directory."))?;
    let source = File::open(path).map_err(io_error)?;
    let permissions = source.metadata().map_err(io_error)?.permissions();
    let temp_path = parent.join(format!(".june-write-{}.tmp", uuid::Uuid::new_v4()));
    let backup_path = parent.join(format!(".june-backup-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut staged = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(io_error)?;
        staged.write_all(replacement).map_err(io_error)?;
        staged.set_permissions(permissions).map_err(io_error)?;
        crate::filesystem::preserve_replacement_metadata(&source, &staged).map_err(io_error)?;
        staged.sync_all().map_err(io_error)?;
        drop(staged);
        let current = fs::read(path).map_err(io_error)?;
        if file_revision(&current) != expected_revision {
            return Err(AppError::new(
                "agent_file_revision_conflict",
                "The file changed before the edit could be applied. Read it again and retry.",
            ));
        }
        match crate::filesystem::replace_existing_file(&temp_path, path, &backup_path) {
            crate::filesystem::ReplaceExistingFileOutcome::Replaced => {}
            crate::filesystem::ReplaceExistingFileOutcome::RecoveryRequired(error) => {
                return Err(AppError {
                    code: "agent_file_recovery_required".into(),
                    message: format!(
                        "Windows could not finish replacing the file ({error}). Recovery copies were preserved. Target: {}. Staged replacement: {}. Backup: {}.",
                        path.display(),
                        temp_path.display(),
                        backup_path.display(),
                    ),
                    details: Some(json!({
                        "targetPath": path,
                        "stagedPath": temp_path,
                        "backupPath": backup_path,
                    })),
                });
            }
            crate::filesystem::ReplaceExistingFileOutcome::NotReplaced(error) => {
                return Err(io_error(error));
            }
        }
        Ok(MutationResult {
            size_bytes: replacement.len(),
            revision: file_revision(replacement),
        })
    })();
    if result
        .as_ref()
        .is_err_and(|error| error.code != "agent_file_recovery_required")
    {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

async fn record_artifact_best_effort(context: &ToolContext, path: &Path, action: &str) -> bool {
    match record_artifact(context, path, action, None).await {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(
                error_code = %error.code,
                run_id = %context.run_id,
                action,
                "file mutation succeeded but artifact recording failed"
            );
            false
        }
    }
}

async fn import_file(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let source = resolve_read_path(context, required_string(arguments, "sourcePath")?)?;
    if !source.is_file() {
        return Err(AppError::new(
            "agent_import_invalid",
            "Import source is not a file.",
        ));
    }
    let name = source
        .file_name()
        .ok_or_else(|| AppError::new("agent_import_invalid", "Import source has no file name."))?;
    let destination = resolve_write_path(
        context,
        &format!("imports/{}", name.to_string_lossy()),
        false,
    )?;
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(io_error)?;
    }
    tokio::fs::copy(&source, &destination)
        .await
        .map_err(io_error)?;
    record_artifact(context, &destination, "imported", Some(&source)).await?;
    Ok(json!({ "path": destination, "sourcePath": source }))
}

async fn preview_file(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let path = resolve_read_path(context, required_string(arguments, "path")?)?;
    let metadata = tokio::fs::metadata(&path).await.map_err(io_error)?;
    let preview = if metadata.len() <= 64 * 1024 {
        tokio::fs::read_to_string(&path).await.ok()
    } else {
        None
    };
    Ok(json!({ "path": path, "sizeBytes": metadata.len(), "text": preview }))
}

async fn record_artifact(
    context: &ToolContext,
    path: &Path,
    action: &str,
    original: Option<&Path>,
) -> Result<(), AppError> {
    let metadata = tokio::fs::metadata(path).await.map_err(io_error)?;
    sqlx::query::query("INSERT INTO agent_artifacts(id, session_id, run_id, provenance, action, path, original_path, size_bytes, available, created_at) VALUES (?, ?, ?, 'tool', ?, ?, ?, ?, 1, ?)")
        .bind(uuid::Uuid::new_v4().to_string()).bind(&context.session_id).bind(&context.run_id)
        .bind(action).bind(path.to_string_lossy().as_ref()).bind(original.map(|value| value.to_string_lossy().into_owned()))
        .bind(metadata.len() as i64).bind(chrono::Utc::now().to_rfc3339()).execute(&context.repository.pool).await?;
    Ok(())
}

async fn record_artifact_with_mime(
    context: &ToolContext,
    path: &Path,
    action: &str,
    original: Option<&Path>,
    mime_type: &str,
) -> Result<(), AppError> {
    let metadata = tokio::fs::metadata(path).await.map_err(io_error)?;
    sqlx::query::query("INSERT INTO agent_artifacts(id, session_id, run_id, provenance, action, path, original_path, mime_type, size_bytes, available, created_at) VALUES (?, ?, ?, 'tool', ?, ?, ?, ?, ?, 1, ?)")
        .bind(uuid::Uuid::new_v4().to_string()).bind(&context.session_id).bind(&context.run_id)
        .bind(action).bind(path.to_string_lossy().as_ref()).bind(original.map(|value| value.to_string_lossy().into_owned()))
        .bind(mime_type).bind(metadata.len() as i64).bind(chrono::Utc::now().to_rfc3339()).execute(&context.repository.pool).await?;
    Ok(())
}

async fn search_files(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let needle = required_string(arguments, "query")?.to_string();
    let root = resolve_read_path(
        context,
        arguments.get("path").and_then(Value::as_str).unwrap_or("."),
    )?;
    let result = tokio::task::spawn_blocking(move || search_text_files(&root, &needle))
        .await
        .map_err(|error| AppError::new("agent_file_search_failed", error.to_string()))?;
    Ok(json!({ "matches": result.matches, "truncated": result.truncated }))
}

struct FileSearchResult {
    matches: String,
    truncated: bool,
}

fn search_text_files(root: &Path, needle: &str) -> FileSearchResult {
    let mut output = String::new();
    let mut stack = vec![root.to_path_buf()];
    let mut scanned_bytes = 0_u64;
    let mut scanned_files = 0_usize;
    let mut match_count = 0_usize;
    let mut truncated = false;

    while let Some(path) = stack.pop() {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            let Ok(entries) = fs::read_dir(&path) else {
                continue;
            };
            let mut children = entries
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    (!name.starts_with('.')).then(|| entry.path())
                })
                .collect::<Vec<_>>();
            children.sort_by(|left, right| right.cmp(left));
            stack.extend(children);
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        if scanned_files >= MAX_SEARCH_FILES
            || scanned_bytes.saturating_add(metadata.len()) > MAX_SEARCH_SCANNED_BYTES
        {
            truncated = true;
            break;
        }
        scanned_files += 1;
        if metadata.len() > MAX_SEARCH_FILE_BYTES {
            truncated = true;
            continue;
        }
        scanned_bytes = scanned_bytes.saturating_add(metadata.len());
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        for (line_index, line) in text.lines().enumerate() {
            if !line.contains(needle) {
                continue;
            }
            let entry = format!("{}:{}:{}\n", path.display(), line_index + 1, line);
            if match_count >= MAX_SEARCH_MATCHES
                || output.len().saturating_add(entry.len()) > MAX_TOOL_OUTPUT_BYTES
            {
                truncated = true;
                break;
            }
            output.push_str(&entry);
            match_count += 1;
        }
        if truncated && (match_count >= MAX_SEARCH_MATCHES || output.len() >= MAX_TOOL_OUTPUT_BYTES)
        {
            break;
        }
    }

    FileSearchResult {
        matches: output,
        truncated,
    }
}

async fn run_shell(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    #[cfg(target_os = "windows")]
    if context.safety_mode == AgentSafetyMode::Sandboxed {
        return Err(AppError::new(
            "agent_shell_sandbox_unavailable",
            "Shell execution is unavailable in Sandboxed mode on Windows.",
        ));
    }
    let script = required_string(arguments, "command")?;
    let mut command = if cfg!(target_os = "windows") {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(script);
        command
    } else if cfg!(target_os = "macos") && context.safety_mode == AgentSafetyMode::Sandboxed {
        let profile = sandbox_profile(&context.workspace);
        let mut command = Command::new("/usr/bin/sandbox-exec");
        command
            .arg("-p")
            .arg(profile)
            .arg("/bin/zsh")
            .arg("-lc")
            .arg(script);
        command
    } else {
        let mut command = Command::new("/bin/sh");
        command.arg("-lc").arg(script);
        command
    };
    command
        .current_dir(&context.workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(secret_env) = arguments.get("secretEnv").and_then(Value::as_object) {
        for (name, secret_ref) in secret_env {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|character| character == '_' || character.is_ascii_alphanumeric())
                || !name
                    .chars()
                    .next()
                    .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
            {
                return Err(AppError::new(
                    "agent_secret_environment_invalid",
                    "A secret environment variable name is invalid.",
                ));
            }
            let secret_ref = secret_ref.as_str().ok_or_else(|| {
                AppError::new(
                    "agent_secret_reference_invalid",
                    "A secret reference is invalid.",
                )
            })?;
            let value = super::secrets::take(secret_ref).await?.ok_or_else(|| {
                AppError::new(
                    "agent_secret_reference_expired",
                    "The requested secret is no longer available.",
                )
            })?;
            command.env(name, value);
        }
    }
    let mut child = command.spawn().map_err(io_error)?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::new("agent_shell_failed", "Shell stdout was unavailable."))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::new("agent_shell_failed", "Shell stderr was unavailable."))?;
    let stdout_task = tokio::spawn(async move { read_bounded(&mut stdout).await });
    let stderr_task = tokio::spawn(async move { read_bounded(&mut stderr).await });
    let mut cancelled = context.cancellations.register(&context.run_id).await;
    let status = tokio::select! {
        status = child.wait() => status.map_err(io_error)?,
        _ = &mut cancelled.receiver => { let _ = child.kill().await; return Err(AppError::new("agent_tool_cancelled", "Shell command was cancelled.")); }
    };
    let stdout_text = stdout_task
        .await
        .map_err(|error| AppError::new("agent_shell_failed", error.to_string()))??;
    let stderr_text = stderr_task
        .await
        .map_err(|error| AppError::new("agent_shell_failed", error.to_string()))??;
    Ok(json!({ "exitCode": status.code(), "stdout": stdout_text, "stderr": stderr_text }))
}

async fn notion_tool(
    context: &ToolContext,
    kind: &str,
    arguments: &Value,
) -> Result<Value, AppError> {
    let tool_name = required_string(arguments, "toolName")?.to_string();
    let tool_arguments = arguments
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let request = crate::connectors::notion::NotionHostedToolCallRequest {
        tool_name,
        arguments: tool_arguments,
        deadline_unix_ms: None,
    };
    let result = if kind == "notion_action" {
        crate::connectors::notion::call_hosted_action_tool_approved(&context.app, request).await?
    } else {
        crate::connectors::notion::call_hosted_tool(&context.app, request).await?
    };
    serde_json::to_value(result)
        .map_err(|error| AppError::new("agent_connector_response_invalid", error.to_string()))
}

async fn list_skills(context: &ToolContext) -> Result<Value, AppError> {
    let enabled_skill_ids = context
        .repository
        .run_enabled_skills(&context.run_id)
        .await?;
    let skills = super::api::enabled_skill_descriptors(
        &context.app,
        &context.repository,
        &enabled_skill_ids,
    )
    .await?;
    Ok(json!({ "skills": skills }))
}

async fn load_skill(context: &ToolContext, arguments: &Value) -> Result<Value, AppError> {
    let name = required_string(arguments, "name")?;
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(AppError::new(
            "agent_skill_invalid",
            "Skill name is invalid.",
        ));
    }
    if !context
        .repository
        .run_enabled_skills(&context.run_id)
        .await?
        .iter()
        .any(|allowed| allowed == name)
    {
        return Err(AppError::new(
            "agent_skill_disabled",
            "This skill is not enabled for the current run.",
        ));
    }
    for root in skill_roots(&context.app) {
        let path = root.join(name).join("SKILL.md");
        if path.is_file() {
            let content = tokio::fs::read_to_string(&path).await.map_err(io_error)?;
            return Ok(json!({ "name": name, "content": content, "path": path }));
        }
    }
    Err(AppError::new(
        "agent_skill_not_found",
        "Skill was not found.",
    ))
}

async fn consume_clarification_answer(context: &ToolContext) -> Result<Value, AppError> {
    consume_clarification_answer_from_pool(&context.repository.pool, &context.run_id).await
}

async fn consume_secret_reference(context: &ToolContext) -> Result<Value, AppError> {
    let row = query("SELECT id, payload_json FROM agent_items WHERE run_id = ? AND kind = 'interruption' AND json_extract(payload_json, '$.kind') = 'secret' AND json_extract(payload_json, '$.secretRef') IS NOT NULL AND COALESCE(json_extract(payload_json, '$.secretReferenceConsumed'), 0) = 0 ORDER BY created_at DESC LIMIT 1")
        .bind(&context.run_id)
        .fetch_one(&context.repository.pool)
        .await?;
    let id: String = row.get("id");
    let mut payload: Value = serde_json::from_str(&row.get::<String, _>("payload_json"))
        .map_err(|error| AppError::new("agent_interruption_invalid", error.to_string()))?;
    let secret_ref = payload
        .get("secretRef")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::new(
                "agent_secret_unanswered",
                "The secret request has not been answered.",
            )
        })?
        .to_string();
    payload["secretReferenceConsumed"] = Value::Bool(true);
    query("UPDATE agent_items SET payload_json = ? WHERE id = ?")
        .bind(payload.to_string())
        .bind(id)
        .execute(&context.repository.pool)
        .await?;
    Ok(json!({ "secretRef": secret_ref, "available": true }))
}

async fn consume_clarification_answer_from_pool(
    pool: &sqlx_sqlite::SqlitePool,
    run_id: &str,
) -> Result<Value, AppError> {
    // The Agents SDK assigns a fresh execution call id after an approval resumes.
    // That id is not guaranteed to match the provider tool-call id persisted on
    // the interruption. The run can only resume one answered clarification at a
    // time, so select the newest answered, unconsumed clarification for the run.
    let row = query("SELECT id, payload_json FROM agent_items WHERE run_id = ? AND kind = 'interruption' AND json_extract(payload_json, '$.kind') = 'clarification' AND json_extract(payload_json, '$.answer') IS NOT NULL AND COALESCE(json_extract(payload_json, '$.answerConsumed'), 0) = 0 ORDER BY created_at DESC LIMIT 1")
        .bind(run_id).fetch_one(pool).await?;
    let id: String = row.get("id");
    let mut payload: Value = serde_json::from_str(&row.get::<String, _>("payload_json"))
        .map_err(|error| AppError::new("agent_interruption_invalid", error.to_string()))?;
    let answer = payload
        .get("answer")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::new(
                "agent_clarification_unanswered",
                "The clarification has not been answered.",
            )
        })?
        .to_string();
    payload["answerConsumed"] = Value::Bool(true);
    query("UPDATE agent_items SET payload_json = ? WHERE id = ?")
        .bind(payload.to_string())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(json!({ "answer": answer }))
}

fn requested_path(workspace: &Path, requested: &str) -> PathBuf {
    let requested = Path::new(requested);
    if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        workspace.join(requested)
    }
}

fn resolve_read_path(context: &ToolContext, requested: &str) -> Result<PathBuf, AppError> {
    resolve_read_path_from(&context.workspace, requested)
}

fn resolve_read_path_from(workspace: &Path, requested: &str) -> Result<PathBuf, AppError> {
    requested_path(workspace, requested)
        .canonicalize()
        .map_err(io_error)
}

fn resolve_write_path(
    context: &ToolContext,
    requested: &str,
    must_exist: bool,
) -> Result<PathBuf, AppError> {
    resolve_write_path_for_mode(
        &context.workspace,
        context.safety_mode,
        requested,
        must_exist,
    )
}

fn resolve_write_path_for_mode(
    workspace: &Path,
    safety_mode: AgentSafetyMode,
    requested: &str,
    must_exist: bool,
) -> Result<PathBuf, AppError> {
    let requested = Path::new(requested);
    let joined = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        workspace.join(requested)
    };
    let resolved = if must_exist || joined.exists() {
        joined.canonicalize().map_err(io_error)?
    } else {
        let mut existing = joined.as_path();
        while !existing.exists() {
            existing = existing.parent().ok_or_else(|| {
                AppError::new("agent_path_invalid", "Path has no existing ancestor.")
            })?;
        }
        let canonical_existing = existing.canonicalize().map_err(io_error)?;
        let suffix = joined
            .strip_prefix(existing)
            .map_err(|_| AppError::new("agent_path_invalid", "Path could not be resolved."))?;
        if suffix
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(AppError::new(
                "agent_path_invalid",
                "Path contains unresolved traversal.",
            ));
        }
        canonical_existing.join(suffix)
    };
    if safety_mode == AgentSafetyMode::Sandboxed {
        let workspace = workspace.canonicalize().map_err(io_error)?;
        if !resolved.starts_with(workspace) {
            return Err(AppError::new(
                "agent_path_denied",
                "Sandboxed mode can only change files in this session's workspace.",
            ));
        }
    }
    Ok(resolved)
}

fn skill_roots(app: &AppHandle) -> Vec<PathBuf> {
    super::api::skill_root_paths(app)
}

pub(crate) fn sandbox_profile(workspace: &Path) -> String {
    let escaped = workspace.to_string_lossy().replace('"', "\\\"");
    format!("(version 1) (allow default) (deny file-write*) (allow file-write* (subpath \"{escaped}\")) (allow file-write* (subpath \"/private/tmp\"))")
}

async fn read_bounded(
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
) -> Result<String, AppError> {
    let mut bytes = Vec::with_capacity(MAX_TOOL_OUTPUT_BYTES.min(8 * 1_024));
    let mut buffer = [0_u8; 8 * 1_024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await.map_err(io_error)?;
        if read == 0 {
            break;
        }
        let remaining = MAX_TOOL_OUTPUT_BYTES.saturating_sub(bytes.len());
        let retained = remaining.min(read);
        bytes.extend_from_slice(&buffer[..retained]);
        truncated |= retained < read;
    }
    if truncated {
        bytes.extend_from_slice(b"\n[output truncated]");
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, AppError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "agent_tool_arguments_invalid",
                format!("{key} is required."),
            )
        })
}
fn string_argument<'a>(value: &'a Value, key: &str) -> Result<&'a str, AppError> {
    value.get(key).and_then(Value::as_str).ok_or_else(|| {
        AppError::new(
            "agent_tool_arguments_invalid",
            format!("{key} is required."),
        )
    })
}
fn truncate(mut value: String) -> String {
    if value.len() > MAX_TOOL_OUTPUT_BYTES {
        value.truncate(MAX_TOOL_OUTPUT_BYTES);
        value.push_str("\n[output truncated]");
    }
    value
}
fn io_error(error: std::io::Error) -> AppError {
    AppError::new("agent_tool_io_failed", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx_sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn settled_cancellation_registration_is_removed() {
        let registry = ToolCancellationRegistry::default();
        let registration = registry.register("run-1").await;
        assert_eq!(registry.registration_count("run-1"), 1);

        drop(registration);

        assert_eq!(registry.registration_count("run-1"), 0);
    }

    #[tokio::test]
    async fn cancelling_a_run_notifies_and_clears_every_registration() {
        let registry = ToolCancellationRegistry::default();
        let mut first = registry.register("run-1").await;
        let mut second = registry.register("run-1").await;

        registry.cancel("run-1").await;

        assert_eq!(registry.registration_count("run-1"), 0);
        assert!((&mut first.receiver).await.is_ok());
        assert!((&mut second.receiver).await.is_ok());
    }

    #[tokio::test]
    async fn bounded_reader_drains_after_retaining_its_prefix() {
        let bytes = vec![b'x'; MAX_TOOL_OUTPUT_BYTES + 4_096];
        let mut input = bytes.as_slice();

        let output = read_bounded(&mut input).await.unwrap();

        assert!(input.is_empty());
        assert_eq!(output.matches('x').count(), MAX_TOOL_OUTPUT_BYTES);
        assert!(output.ends_with("\n[output truncated]"));
    }

    #[test]
    fn sandbox_profile_only_grants_workspace_and_tmp_writes() {
        let profile = sandbox_profile(Path::new("/Users/example/June Workspace"));
        assert!(profile.contains("(deny file-write*)"));
        assert!(profile.contains("/Users/example/June Workspace"));
        assert!(!profile.contains("(allow file-write*)"));
    }

    #[test]
    fn sandboxed_reads_allow_existing_paths_outside_the_workspace() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let external = root.path().join("vault").join("note.md");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(external.parent().unwrap()).unwrap();
        fs::write(&external, "# Note").unwrap();

        assert_eq!(
            resolve_read_path_from(&workspace, external.to_str().unwrap()).unwrap(),
            external.canonicalize().unwrap()
        );
    }

    #[test]
    fn sandboxed_writes_stay_inside_the_workspace() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let external = root.path().join("vault").join("note.md");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(external.parent().unwrap()).unwrap();
        fs::write(&external, "# Note").unwrap();

        let internal = resolve_write_path_for_mode(
            &workspace,
            AgentSafetyMode::Sandboxed,
            "new/note.md",
            false,
        )
        .unwrap();
        assert!(internal.starts_with(workspace.canonicalize().unwrap()));

        let patch_error = resolve_write_path_for_mode(
            &workspace,
            AgentSafetyMode::Sandboxed,
            external.to_str().unwrap(),
            true,
        )
        .unwrap_err();
        assert_eq!(patch_error.code, "agent_path_denied");

        let create_error = resolve_write_path_for_mode(
            &workspace,
            AgentSafetyMode::Sandboxed,
            root.path().join("outside.md").to_str().unwrap(),
            false,
        )
        .unwrap_err();
        assert_eq!(create_error.code, "agent_path_denied");
    }

    #[test]
    fn unrestricted_writes_may_target_external_paths() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let external_parent = root.path().join("vault");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&external_parent).unwrap();
        let external = external_parent.join("new-note.md");

        let resolved = resolve_write_path_for_mode(
            &workspace,
            AgentSafetyMode::Unrestricted,
            external.to_str().unwrap(),
            false,
        )
        .unwrap();

        assert_eq!(
            resolved,
            external_parent.canonicalize().unwrap().join("new-note.md")
        );
    }

    #[test]
    fn unresolved_write_traversal_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        let error = resolve_write_path_for_mode(
            &workspace,
            AgentSafetyMode::Sandboxed,
            "missing/../../outside.md",
            false,
        )
        .unwrap_err();

        assert!(matches!(
            error.code.as_str(),
            "agent_path_invalid" | "agent_path_denied"
        ));
    }

    #[test]
    fn web_requests_use_the_tool_call_id() {
        let request = web_request(&json!({ "query": "OpenAI Agents SDK" }), Some("call-42"));

        assert_eq!(request["query"], "OpenAI Agents SDK");
        assert_eq!(request["requestId"], "call-42");
    }

    #[tokio::test]
    async fn clarification_resume_consumes_answer_when_sdk_changes_call_id() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        query(
            "CREATE TABLE agent_items (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        query("INSERT INTO agent_items (id, run_id, kind, payload_json, created_at) VALUES (?, ?, 'interruption', ?, ?)")
            .bind("item-1")
            .bind("run-1")
            .bind(json!({
                "id": "provider-call-id",
                "kind": "clarification",
                "answer": "Bullets"
            }).to_string())
            .bind("2026-07-25T00:00:00Z")
            .execute(&pool)
            .await
            .unwrap();

        let result = consume_clarification_answer_from_pool(&pool, "run-1")
            .await
            .unwrap();

        assert_eq!(result, json!({ "answer": "Bullets" }));
        let payload: String = query("SELECT payload_json FROM agent_items WHERE id = 'item-1'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("payload_json");
        let payload: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["answerConsumed"], true);
    }

    #[tokio::test]
    async fn june_search_is_scoped_to_the_active_profile() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for statement in [
            "CREATE TABLE notes (id TEXT PRIMARY KEY, title TEXT NOT NULL, generated_content TEXT, edited_content TEXT, profile TEXT NOT NULL, updated_at TEXT NOT NULL)",
            "CREATE TABLE transcripts (id TEXT PRIMARY KEY, note_id TEXT NOT NULL, text TEXT NOT NULL, created_at TEXT NOT NULL)",
            "CREATE TABLE dictation_history (id TEXT PRIMARY KEY, text TEXT NOT NULL, language TEXT, profile TEXT NOT NULL, created_at TEXT NOT NULL)",
        ] {
            query(statement).execute(&pool).await.unwrap();
        }
        query("INSERT INTO notes (id, title, generated_content, profile, updated_at) VALUES ('mine', 'Shared needle', 'mine', 'profile-a', '2026-01-01'), ('theirs', 'Shared needle', 'theirs', 'profile-b', '2026-01-02')")
            .execute(&pool).await.unwrap();
        query("INSERT INTO dictation_history (id, text, profile, created_at) VALUES ('dictation-a', 'needle mine', 'profile-a', '2026-01-01'), ('dictation-b', 'needle theirs', 'profile-b', '2026-01-02')")
            .execute(&pool).await.unwrap();

        let result = search_june_for_profile(&pool, "profile-a", "needle")
            .await
            .unwrap();

        assert_eq!(result["notes"].as_array().unwrap().len(), 1);
        assert_eq!(result["notes"][0]["id"], "mine");
        assert_eq!(result["dictations"].as_array().unwrap().len(), 1);
        assert_eq!(result["dictations"][0]["id"], "dictation-a");
    }

    #[test]
    fn file_search_is_native_bounded_and_skips_hidden_or_binary_files() {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(directory.path().join("first.txt"), "needle one\nother\n").unwrap();
        fs::write(nested.join("second.txt"), "other\nneedle two\n").unwrap();
        fs::write(nested.join("binary.bin"), b"needle\0hidden").unwrap();
        fs::write(directory.path().join(".hidden.txt"), "needle hidden\n").unwrap();

        let result = search_text_files(directory.path(), "needle");

        assert!(!result.truncated);
        assert!(result.matches.contains("first.txt:1:needle one"));
        assert!(result.matches.contains("second.txt:2:needle two"));
        assert!(!result.matches.contains("binary.bin"));
        assert!(!result.matches.contains("hidden.txt"));
    }

    #[test]
    fn exact_crlf_revision_and_metadata_include_all_bytes() {
        let bytes = b"\xef\xbb\xbffirst\r\nsecond\r\n";
        assert_eq!(detect_line_ending(bytes), LineEnding::Crlf);
        assert_eq!(
            file_revision(bytes),
            "sha256:7d00111004b3faa8a6cb23f25bf6fecddb5e885636e3c3b6c8ba8a4d8c21bcf1"
        );
    }

    #[test]
    fn create_only_refuses_existing_file_without_changing_it_and_creates_new_file() {
        let directory = tempfile::tempdir().unwrap();
        let existing = directory.path().join("existing.md");
        fs::write(&existing, b"original").unwrap();
        let error = create_text_file(&existing, b"replacement").unwrap_err();
        assert_eq!(error.code, "agent_file_exists");
        assert_eq!(fs::read(&existing).unwrap(), b"original");

        let created = directory.path().join("created.md");
        create_text_file(&created, b"new").unwrap();
        assert_eq!(fs::read(created).unwrap(), b"new");
    }

    #[test]
    fn replacement_rejects_missing_invalid_and_stale_files_and_succeeds_when_current() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.md");
        let missing = replace_text_file(
            &path,
            "new",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap_err();
        assert_eq!(missing.code, "agent_file_not_found");

        fs::write(&path, b"old\n").unwrap();
        let invalid = replace_text_file(&path, "new", "bad").unwrap_err();
        assert_eq!(invalid.code, "agent_file_revision_invalid");
        let stale = replace_text_file(
            &path,
            "new",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap_err();
        assert_eq!(stale.code, "agent_file_revision_conflict");
        assert_eq!(fs::read(&path).unwrap(), b"old\n");

        let revision = file_revision(b"old\n");
        let result = replace_text_file(&path, "new\n", &revision).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new\n");
        assert_eq!(result.revision, file_revision(b"new\n"));
    }

    #[test]
    fn replacement_preserves_crlf_bom_and_mixed_files_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let crlf = directory.path().join("crlf.md");
        let original = b"\xef\xbb\xbfold\r\ntext\r\n";
        fs::write(&crlf, original).unwrap();
        replace_text_file(&crlf, "new\ntext\n", &file_revision(original)).unwrap();
        assert_eq!(fs::read(&crlf).unwrap(), b"\xef\xbb\xbfnew\r\ntext\r\n");

        let mixed = directory.path().join("mixed.md");
        let original = b"one\r\ntwo\n";
        fs::write(&mixed, original).unwrap();
        let error = replace_text_file(&mixed, "changed", &file_revision(original)).unwrap_err();
        assert_eq!(error.code, "agent_file_line_endings_mixed");
        assert_eq!(fs::read(mixed).unwrap(), original);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn patch_and_replacement_preserve_macos_acl_and_extended_attributes() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt, process::Command};

        fn acl(path: &Path) -> String {
            let output = Command::new("/bin/ls")
                .env("LC_ALL", "C")
                .arg("-led")
                .arg(path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "ls failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .skip(1)
                .collect::<Vec<_>>()
                .join("\n")
        }

        fn extended_attribute(path: &Path) -> Vec<u8> {
            let path = CString::new(path.as_os_str().as_bytes()).unwrap();
            let name = c"com.opensoftware.june-test";
            let size = unsafe {
                libc::getxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0, 0, 0)
            };
            assert!(
                size >= 0,
                "getxattr failed: {}",
                std::io::Error::last_os_error()
            );
            let mut value = vec![0_u8; size as usize];
            let read = unsafe {
                libc::getxattr(
                    path.as_ptr(),
                    name.as_ptr(),
                    value.as_mut_ptr().cast(),
                    value.len(),
                    0,
                    0,
                )
            };
            assert_eq!(
                read,
                size,
                "getxattr failed: {}",
                std::io::Error::last_os_error()
            );
            value
        }

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.md");
        fs::write(&path, b"old text\n").unwrap();
        let path_string = CString::new(path.as_os_str().as_bytes()).unwrap();
        let xattr_value = b"preserve me";
        let set_xattr = unsafe {
            libc::setxattr(
                path_string.as_ptr(),
                c"com.opensoftware.june-test".as_ptr(),
                xattr_value.as_ptr().cast(),
                xattr_value.len(),
                0,
                0,
            )
        };
        assert_eq!(
            set_xattr,
            0,
            "setxattr failed: {}",
            std::io::Error::last_os_error()
        );
        let set_acl = Command::new("/bin/chmod")
            .env("LC_ALL", "C")
            .args(["+a", "everyone allow readattr"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            set_acl.status.success(),
            "chmod failed: {}",
            String::from_utf8_lossy(&set_acl.stderr)
        );
        let original_acl = acl(&path);
        assert!(original_acl.contains("everyone allow readattr"));

        patch_text_file(&path, "old", "patched").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"patched text\n");
        assert_eq!(extended_attribute(&path), xattr_value);
        assert_eq!(acl(&path), original_acl);

        let revision = file_revision(&fs::read(&path).unwrap());
        replace_text_file(&path, "replacement\n", &revision).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"replacement\n");
        assert_eq!(extended_attribute(&path), xattr_value);
        assert_eq!(acl(&path), original_acl);
    }

    #[test]
    fn staged_revision_conflict_preserves_external_edit_and_removes_temp_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.md");
        fs::write(&path, b"external edit\n").unwrap();

        let error = stage_and_replace(
            &path,
            b"stale replacement\n",
            &file_revision(b"older content\n"),
        )
        .unwrap_err();

        assert_eq!(error.code, "agent_file_revision_conflict");
        assert_eq!(fs::read(&path).unwrap(), b"external edit\n");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn patch_adapts_lf_anchors_to_crlf_and_ambiguous_patches_do_not_mutate() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.md");
        fs::write(&path, b"one\r\ntwo\r\nthree\r\n").unwrap();
        patch_text_file(&path, "one\ntwo", "one\nchanged").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one\r\nchanged\r\nthree\r\n");

        for before in ["missing", "one"] {
            let original = if before == "one" {
                b"one\none\n".as_slice()
            } else {
                b"one\n".as_slice()
            };
            fs::write(&path, original).unwrap();
            let error = patch_text_file(&path, before, "changed").unwrap_err();
            assert_eq!(error.code, "agent_patch_ambiguous");
            assert_eq!(fs::read(&path).unwrap(), original);
            assert!(error.message.contains("Reread"));
            assert!(!error.message.contains("replace"));
        }
    }

    #[test]
    fn patch_preserves_bom_and_accepts_an_empty_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.md");
        fs::write(&path, b"\xef\xbb\xbfone\r\ntwo\r\n").unwrap();

        patch_text_file(&path, "\u{feff}one\ntwo\n", "").unwrap();

        assert_eq!(fs::read(path).unwrap(), b"\xef\xbb\xbf");
    }

    #[test]
    fn patch_uses_raw_exact_matching_for_mixed_line_endings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.md");
        let original = b"one\r\ntwo\nthree\r\n";
        fs::write(&path, original).unwrap();

        patch_text_file(&path, "one\r\ntwo\n", "one\r\nchanged\n").unwrap();

        assert_eq!(fs::read(path).unwrap(), b"one\r\nchanged\nthree\r\n");
    }
}
