use crate::{domain::processing::PROMPT_VERSION, domain::types::AppError};
use serde::{Deserialize, Serialize};

pub const DEFAULT_GENERATION_PROVIDER: &str = "mock";

#[derive(Debug, Clone)]
pub struct GenerationRequest {
    pub provider: String,
    pub title: String,
    pub transcript: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GenerationProviderResult {
    pub content: String,
    pub title_suggestion: Option<String>,
    pub provider: String,
    pub prompt_version: String,
}

pub async fn generate_note_from_transcript(
    request: GenerationRequest,
) -> Result<GenerationProviderResult, AppError> {
    let transcript = request.transcript.trim();
    if transcript.is_empty() {
        return Err(AppError::new(
            "transcription_empty",
            "Transcript is empty, so a note cannot be generated.",
        ));
    }

    match request.provider.as_str() {
        "mock" | "" => Ok(GenerationProviderResult {
            content: format!("{}\n\n{}", heading_for(&request.title), transcript),
            title_suggestion: if request.title.trim().is_empty() {
                Some("New note".to_string())
            } else {
                Some(request.title.trim().to_string())
            },
            provider: DEFAULT_GENERATION_PROVIDER.to_string(),
            prompt_version: PROMPT_VERSION.to_string(),
        }),
        _ => Err(AppError::new(
            "provider_not_configured",
            "Only the mock generation provider is configured for local MVP verification.",
        )),
    }
}

fn heading_for(title: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        "# Generated note".to_string()
    } else {
        format!("# {title}")
    }
}
