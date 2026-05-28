use crate::transcription::TranscriptionWireResponse;
use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use scribe_config::UpstreamConfig;
use scribe_domain::{
    CleanedText, Cleaner, CleanupRequest, DomainError, GeneratedNote, GenerationRequest, Generator,
    TokenUsage, Transcriber, Transcript, TranscriptionRequest,
};
use serde::{Deserialize, Serialize};

pub const PROVIDER_NAME: &str = "venice";

pub struct VeniceTranscriber {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl VeniceTranscriber {
    pub fn from_config(http: reqwest::Client, config: &UpstreamConfig) -> Self {
        Self {
            http,
            api_key: config.api_key.clone(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
        }
    }
}

#[async_trait]
impl Transcriber for VeniceTranscriber {
    async fn transcribe(&self, request: TranscriptionRequest) -> Result<Transcript, DomainError> {
        let model_id = request.model.0.clone();
        let url = format!("{}/audio/transcriptions", self.base_url);
        let audio_part = Part::bytes(request.audio)
            .file_name(request.filename.clone())
            .mime_str(crate::transcription::audio_mime(&request.filename))
            .map_err(|error| {
                tracing::error!(%error, %url, model = %model_id, "venice: audio mime build failed");
                DomainError::UpstreamProvider
            })?;
        let form = Form::new()
            .text("model", model_id.clone())
            .text("response_format", "json")
            .part("file", audio_part);
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|error| {
                tracing::error!(%error, %url, model = %model_id, "venice: transport error");
                DomainError::UpstreamProvider
            })?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            tracing::error!(%status, %url, model = %model_id, body = %body, "venice: non-success response");
            return Err(DomainError::UpstreamProvider);
        }
        let parsed = response
            .json::<TranscriptionWireResponse>()
            .await
            .map_err(|error| {
                tracing::error!(%error, %url, model = %model_id, "venice: response JSON parse failed");
                DomainError::UpstreamProvider
            })?;
        let text = parsed.text.trim().to_string();
        if text.is_empty() {
            tracing::error!(%url, model = %model_id, "venice: empty transcript");
            return Err(DomainError::UpstreamProvider);
        }
        Ok(Transcript {
            text,
            language: parsed.language,
            provider: PROVIDER_NAME.to_string(),
        })
    }
}

pub struct VeniceGenerator {
    chat: VeniceChat,
}

impl VeniceGenerator {
    pub fn from_config(http: reqwest::Client, config: &UpstreamConfig) -> Self {
        Self {
            chat: VeniceChat::new(http, config),
        }
    }
}

#[async_trait]
impl Generator for VeniceGenerator {
    async fn generate(&self, request: GenerationRequest) -> Result<GeneratedNote, DomainError> {
        let transcript = request.transcript.trim();
        if transcript.is_empty() {
            return Err(DomainError::InvalidInput {
                reason: "transcript_empty".to_string(),
            });
        }
        let title_hint = request.title.trim();
        let user_message = format!(
            "Current title: {}\nDetected language: {}\n\n{}",
            if title_hint.is_empty() {
                "New note"
            } else {
                title_hint
            },
            request.language.as_deref().unwrap_or("unknown"),
            generation_source_text(
                request.existing_generated_note.as_deref(),
                request.manual_notes.as_deref(),
                transcript,
            )
        );
        let parsed = self
            .chat
            .complete(ChatCompletionRequest {
                model: request.model.0,
                messages: vec![
                    ChatMessage::system(request.system_prompt),
                    ChatMessage::user(user_message),
                ],
            })
            .await?;
        let content = parsed
            .first_choice_text()
            .filter(|text| !text.is_empty())
            .ok_or(DomainError::UpstreamProvider)?;
        Ok(GeneratedNote {
            content,
            title_suggestion: Some(if title_hint.is_empty() {
                "New note".to_string()
            } else {
                title_hint.to_string()
            }),
            provider: PROVIDER_NAME.to_string(),
            usage: parsed.usage_or_error()?,
        })
    }
}

pub struct VeniceCleaner {
    chat: VeniceChat,
}

impl VeniceCleaner {
    pub fn from_config(http: reqwest::Client, config: &UpstreamConfig) -> Self {
        Self {
            chat: VeniceChat::new(http, config),
        }
    }
}

#[async_trait]
impl Cleaner for VeniceCleaner {
    async fn cleanup(&self, request: CleanupRequest) -> Result<CleanedText, DomainError> {
        let text = request.text.trim();
        if text.is_empty() {
            return Err(DomainError::InvalidInput {
                reason: "dictation_text_empty".to_string(),
            });
        }
        let user_message =
            cleanup_source_text(text, request.dictionary_context.as_deref(), &request.style);
        let parsed = self
            .chat
            .complete(ChatCompletionRequest {
                model: request.model.0,
                messages: vec![
                    ChatMessage::system(request.system_prompt),
                    ChatMessage::user(user_message),
                ],
            })
            .await?;
        let cleaned = parsed
            .first_choice_text()
            .filter(|text| !text.is_empty())
            .ok_or(DomainError::UpstreamProvider)?;
        Ok(CleanedText {
            text: cleaned,
            provider: PROVIDER_NAME.to_string(),
            usage: parsed.usage_or_error()?,
        })
    }
}

struct VeniceChat {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl VeniceChat {
    fn new(http: reqwest::Client, config: &UpstreamConfig) -> Self {
        Self {
            http,
            api_key: config.api_key.clone(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
        }
    }

    async fn complete(
        &self,
        body: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, DomainError> {
        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|_| DomainError::UpstreamProvider)?;
        if !response.status().is_success() {
            return Err(DomainError::UpstreamProvider);
        }
        response
            .json::<ChatCompletionResponse>()
            .await
            .map_err(|_| DomainError::UpstreamProvider)
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

impl ChatMessage {
    fn system(content: String) -> Self {
        Self {
            role: "system",
            content,
        }
    }

    fn user(content: String) -> Self {
        Self {
            role: "user",
            content,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatCompletionChoice>,
    usage: Option<ChatCompletionUsage>,
}

impl ChatCompletionResponse {
    fn first_choice_text(&self) -> Option<String> {
        let message = &self.choices.first()?.message;
        Some(message.content.trim().to_string())
    }

    fn usage_or_error(&self) -> Result<TokenUsage, DomainError> {
        let usage = self.usage.as_ref().ok_or(DomainError::UpstreamProvider)?;
        Ok(TokenUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChoice {
    message: ChatCompletionMessage,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

fn generation_source_text(
    existing_generated_note: Option<&str>,
    manual_notes: Option<&str>,
    transcript: &str,
) -> String {
    let mut sections = Vec::new();
    if let Some(existing_generated_note) = existing_generated_note
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(format!(
            "<existing_generated_note_context>\n{existing_generated_note}\n</existing_generated_note_context>"
        ));
    }
    if let Some(manual_notes) = manual_notes
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(format!(
            "<new_manual_notes_context>\n{manual_notes}\n</new_manual_notes_context>"
        ));
    }
    sections.push(format!(
        "<new_transcript>\n{}\n</new_transcript>",
        transcript.trim()
    ));
    sections.push(
        "<output_contract>\nReturn only the new note block for the new transcript. Do not repeat existing note content. Do not output manual note labels. Do not add wrapper headings.\n</output_contract>".to_string(),
    );
    sections.join("\n\n")
}

fn cleanup_source_text(text: &str, dictionary_context: Option<&str>, style: &str) -> String {
    let mut sections = Vec::new();
    if let Some(dictionary_context) = dictionary_context
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(format!(
            "<dictionary_context>\n{dictionary_context}\n</dictionary_context>"
        ));
    }
    if !style.trim().is_empty() {
        sections.push(format!("<style>\n{}\n</style>", style.trim()));
    }
    sections.push(format!(
        "<asr_transcript>\n{}\n</asr_transcript>",
        text.trim()
    ));
    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::VeniceGenerator;
    use crate::http;
    use pretty_assertions::assert_eq;
    use scribe_config::UpstreamConfig;
    use scribe_domain::{GenerationRequest, Generator, ModelId};
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    #[tokio::test]
    async fn parses_content_and_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer venice_key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [
                    { "message": { "content": "Generated note block" } }
                ],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5
                }
            })))
            .mount(&server)
            .await;
        let generator = VeniceGenerator::from_config(
            http::default_client(),
            &UpstreamConfig {
                api_key: "venice_key".to_string(),
                base_url: server.uri(),
            },
        );

        let generated = generator
            .generate(GenerationRequest {
                title: "Title".to_string(),
                transcript: "Transcript".to_string(),
                manual_notes: None,
                language: Some("en".to_string()),
                existing_generated_note: None,
                model: ModelId("zai-org-glm-5".to_string()),
                system_prompt: "system".to_string(),
            })
            .await;

        assert_eq!(
            generated.map(|value| (
                value.content,
                value.provider,
                value.usage.prompt_tokens,
                value.usage.completion_tokens,
            )),
            Ok((
                "Generated note block".to_string(),
                "venice".to_string(),
                10,
                5
            ))
        );
    }
}
