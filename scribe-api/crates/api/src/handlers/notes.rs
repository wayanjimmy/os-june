use crate::{
    audio::validate_audio, auth::authenticated_user, envelope::ApiResponse, error::ApiError,
    multipart::MultipartFields, state::ApiState,
};
use axum::{
    Json,
    extract::{Multipart, State},
    http::HeaderMap,
};
use scribe_domain::ModelId;
use scribe_services::{
    NoteGenerateOutput, NoteGenerateParams, NoteTranscribeOutput, NoteTranscribeParams,
};
use serde::{Deserialize, Serialize};

pub(crate) async fn transcribe(
    State(state): State<ApiState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<ApiResponse<TranscribeResponse>>, ApiError> {
    let user_id = authenticated_user(&state, &headers).await?;
    let limits = state.limits();
    let mut form = MultipartFields::collect(multipart, limits.max_audio_bytes).await?;
    let audio = form.required_audio()?;
    validate_audio(&audio)?;
    let model_id = form.required_text("model")?;
    require_priced_model(&state, &model_id)?;
    let filename = form
        .take_filename()
        .unwrap_or_else(|| "recording.wav".to_string());
    let output = state
        .note_transcribe()
        .transcribe(NoteTranscribeParams {
            user_id,
            note_id: form.required_text("noteId")?,
            audio,
            filename,
            title: form.required_text("title")?,
            context: form.optional_text("context"),
            model_id: ModelId(model_id),
        })
        .await?;
    Ok(Json(ApiResponse::ok(TranscribeResponse::from(output))))
}

pub(crate) async fn generate(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<GenerateRequest>,
) -> Result<Json<ApiResponse<GenerateResponse>>, ApiError> {
    let user_id = authenticated_user(&state, &headers).await?;
    let model_id = required(request.model, "model_required")?;
    require_priced_model(&state, &model_id)?;
    let output = state
        .note_generate()
        .generate(NoteGenerateParams {
            user_id,
            note_id: required(request.note_id, "note_id_required")?,
            prompt_version: required(request.prompt_version, "prompt_version_required")?,
            title: request.title,
            transcript: request.transcript,
            manual_notes: request.manual_notes,
            language: request.language,
            existing_generated_note: request.existing_generated_note,
            model_id: ModelId(model_id),
        })
        .await?;
    Ok(Json(ApiResponse::ok(GenerateResponse::from(output))))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateRequest {
    pub note_id: Option<String>,
    pub prompt_version: Option<String>,
    pub title: String,
    pub transcript: String,
    pub manual_notes: Option<String>,
    pub language: Option<String>,
    pub existing_generated_note: Option<String>,
    pub model: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeResponse {
    pub text: String,
    pub language: Option<String>,
    pub provider: String,
    pub credits_charged: u64,
    pub idempotent_replay: bool,
}

impl From<NoteTranscribeOutput> for TranscribeResponse {
    fn from(output: NoteTranscribeOutput) -> Self {
        Self {
            text: output.transcript.text,
            language: output.transcript.language,
            provider: output.transcript.provider,
            credits_charged: output.receipt.credits_charged.0,
            idempotent_replay: output.receipt.idempotent_replay,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateResponse {
    pub content: String,
    pub title_suggestion: Option<String>,
    pub provider: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub credits_charged: u64,
    pub idempotent_replay: bool,
}

impl From<NoteGenerateOutput> for GenerateResponse {
    fn from(output: NoteGenerateOutput) -> Self {
        Self {
            content: output.generated.content,
            title_suggestion: output.generated.title_suggestion,
            provider: output.generated.provider,
            prompt_tokens: output.generated.usage.prompt_tokens,
            completion_tokens: output.generated.usage.completion_tokens,
            credits_charged: output.receipt.credits_charged.0,
            idempotent_replay: output.receipt.idempotent_replay,
        }
    }
}

pub(crate) fn required(value: Option<String>, message: &str) -> Result<String, ApiError> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request(message))
}

pub(crate) fn require_priced_model(state: &ApiState, model_id: &str) -> Result<(), ApiError> {
    if state.pricing().has_model(model_id) {
        Ok(())
    } else {
        Err(ApiError::unprocessable("model_not_priced"))
    }
}
