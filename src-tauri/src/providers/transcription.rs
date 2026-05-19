use crate::domain::types::AppError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_TRANSCRIPTION_PROVIDER: &str = "mock";

#[derive(Debug, Clone)]
pub struct TranscriptionRequest {
    pub provider: String,
    pub audio_path: PathBuf,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionProviderResult {
    pub text: String,
    pub language: Option<String>,
    pub provider: String,
}

pub async fn transcribe_saved_audio(
    request: TranscriptionRequest,
) -> Result<TranscriptionProviderResult, AppError> {
    if !request.audio_path.exists() {
        return Err(AppError::new(
            "audio_artifact_missing",
            "Saved audio is missing and cannot be transcribed.",
        ));
    }

    match request.provider.as_str() {
        "mock" | "" => Ok(TranscriptionProviderResult {
            text: format!(
                "{}: This is a local mock transcript generated from saved audio.",
                request.title.trim()
            ),
            language: Some("en".to_string()),
            provider: DEFAULT_TRANSCRIPTION_PROVIDER.to_string(),
        }),
        _ => Err(AppError::new(
            "provider_not_configured",
            "Only the mock transcription provider is configured for local MVP verification.",
        )),
    }
}
