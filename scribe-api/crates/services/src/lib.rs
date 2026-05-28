#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used, clippy::panic))]

mod charge_flow;
mod dictate;
mod error;
mod note_generate;
mod note_transcribe;
mod pricing;
mod prompts;
mod util;

pub use dictate::{
    DictateCleanupOutput, DictateCleanupParams, DictateService, DictateServiceDeps,
    DictateTranscribeOutput, DictateTranscribeParams,
};
pub use error::ServiceError;
pub use note_generate::{
    NoteGenerateOutput, NoteGenerateParams, NoteGenerateService, NoteGenerateServiceDeps,
};
pub use note_transcribe::{
    NoteTranscribeOutput, NoteTranscribeParams, NoteTranscribeService, NoteTranscribeServiceDeps,
};
pub use pricing::{PricingError, PricingTable};

#[cfg(test)]
mod tests {
    use super::{
        DictateCleanupParams, DictateService, DictateServiceDeps, NoteGenerateParams,
        NoteGenerateService, NoteGenerateServiceDeps, PricingTable, ServiceError,
    };
    use async_trait::async_trait;
    use pretty_assertions::assert_eq;
    use scribe_config::{ModelPriceConfig, ModelProvider, ModelType, PriceUnit};
    use scribe_domain::{
        AudioDurationProbe, Authorization, AuthorizeRequest, ChargeRequest, CleanedText, Cleaner,
        CleanupRequest, Credits, DomainError, GeneratedNote, GenerationRequest, Generator, ModelId,
        OsAccountsClient, Receipt, TokenUsage, Transcriber, Transcript, TranscriptionRequest,
        UserId,
    };
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
        time::Duration,
    };

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum RecordedCall {
        Authorize {
            user_id: String,
            action: String,
            estimate: u64,
            hold_ttl: u64,
        },
        Charge {
            action_token: String,
            credits: u64,
            idempotency_key: String,
        },
    }

    #[tokio::test]
    async fn note_generate_authorizes_then_charges_actual_clamped_to_cap() {
        let os_accounts = Arc::new(RecordingOsAccounts::with_cap(Some(40)));
        let service = NoteGenerateService::new(NoteGenerateServiceDeps {
            pricing: Arc::new(PricingTable::new(models([(
                "text-model",
                PriceUnit::Tokens,
                2,
                ModelType::Text,
            )]))),
            os_accounts: os_accounts.clone(),
            generator: Arc::new(FixedGenerator),
            hold_ttl_seconds: 300,
            flat_estimate_credits: 8200,
        });

        let output = service
            .generate(NoteGenerateParams {
                user_id: UserId("usr_123".to_string()),
                note_id: "note_1".to_string(),
                prompt_version: "v7".to_string(),
                title: "Title".to_string(),
                transcript: "Transcript".to_string(),
                manual_notes: None,
                language: None,
                existing_generated_note: None,
                model_id: ModelId("text-model".to_string()),
            })
            .await
            .expect("generate succeeds with happy path");

        // Cap clamps the actual 60 down to 40.
        assert_eq!(output.receipt.credits_charged.0, 40);
        assert_eq!(
            os_accounts.events(),
            vec![
                RecordedCall::Authorize {
                    user_id: "usr_123".to_string(),
                    action: "note_generate".to_string(),
                    estimate: 8200,
                    hold_ttl: 300,
                },
                RecordedCall::Charge {
                    action_token: "agt_test".to_string(),
                    credits: 40,
                    idempotency_key: "note_generate:usr_123:note_1:v7:agt_test".to_string(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn dictate_cleanup_uses_deterministic_idempotency_key_with_real_session() {
        let os_accounts = Arc::new(RecordingOsAccounts::default());
        let service = DictateService::new(DictateServiceDeps {
            pricing: Arc::new(PricingTable::new(models([(
                "text-model",
                PriceUnit::Tokens,
                1,
                ModelType::Text,
            )]))),
            os_accounts: os_accounts.clone(),
            transcriber: Arc::new(FixedTranscriber),
            cleaner: Arc::new(FixedCleaner),
            duration_probe: Arc::new(FixedDurationProbe),
            transcribe_hold_ttl_seconds: 30,
            cleanup_hold_ttl_seconds: 30,
            flat_estimate_credits: 1024,
        });

        let output = service
            .cleanup(DictateCleanupParams {
                user_id: UserId("usr_123".to_string()),
                session_id: "session_1".to_string(),
                utterance_id: "utt_2".to_string(),
                text: "hello".to_string(),
                dictionary_context: Some("Jakub".to_string()),
                style: "plain".to_string(),
                model_id: ModelId("text-model".to_string()),
            })
            .await
            .expect("cleanup succeeds with happy path");

        assert_eq!(output.receipt.credits_charged.0, 11);
        let charge_call = os_accounts
            .events()
            .into_iter()
            .find_map(|call| match call {
                RecordedCall::Charge {
                    idempotency_key, ..
                } => Some(idempotency_key),
                RecordedCall::Authorize { .. } => None,
            })
            .unwrap_or_default();
        assert_eq!(
            charge_call,
            "dictate_cleanup:usr_123:session_1:utt_2:agt_test"
        );
    }

    #[tokio::test]
    async fn authorization_denial_maps_to_insufficient_credits() {
        let os_accounts = Arc::new(RecordingOsAccounts {
            allow: false,
            ..RecordingOsAccounts::default()
        });
        let service = NoteGenerateService::new(NoteGenerateServiceDeps {
            pricing: Arc::new(PricingTable::new(models([(
                "text-model",
                PriceUnit::Tokens,
                1,
                ModelType::Text,
            )]))),
            os_accounts,
            generator: Arc::new(FixedGenerator),
            hold_ttl_seconds: 300,
            flat_estimate_credits: 1024,
        });

        let result = service
            .generate(NoteGenerateParams {
                user_id: UserId("usr_123".to_string()),
                note_id: "note_1".to_string(),
                prompt_version: "v7".to_string(),
                title: "Title".to_string(),
                transcript: "Transcript".to_string(),
                manual_notes: None,
                language: None,
                existing_generated_note: None,
                model_id: ModelId("text-model".to_string()),
            })
            .await;

        assert!(matches!(result, Err(ServiceError::InsufficientCredits)));
    }

    fn models<const N: usize>(
        values: [(&str, PriceUnit, u64, ModelType); N],
    ) -> BTreeMap<String, ModelPriceConfig> {
        values
            .into_iter()
            .map(|(id, unit, credits_per_unit, model_type)| {
                (
                    id.to_string(),
                    ModelPriceConfig {
                        unit,
                        credits_per_unit,
                        provider: ModelProvider::Openai,
                        model_type,
                        display_name: id.to_string(),
                    },
                )
            })
            .collect()
    }

    struct RecordingOsAccounts {
        allow: bool,
        cap: Option<u64>,
        events: Mutex<Vec<RecordedCall>>,
    }

    impl Default for RecordingOsAccounts {
        fn default() -> Self {
            Self {
                allow: true,
                cap: None,
                events: Mutex::new(Vec::new()),
            }
        }
    }

    impl RecordingOsAccounts {
        fn with_cap(cap: Option<u64>) -> Self {
            Self {
                allow: true,
                cap,
                events: Mutex::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<RecordedCall> {
            self.events
                .lock()
                .map(|events| events.clone())
                .unwrap_or_default()
        }
    }

    #[async_trait]
    impl OsAccountsClient for RecordingOsAccounts {
        async fn authorize(&self, request: AuthorizeRequest) -> Result<Authorization, DomainError> {
            if let Ok(mut events) = self.events.lock() {
                events.push(RecordedCall::Authorize {
                    user_id: request.user_id.0.clone(),
                    action: request.action.to_string(),
                    estimate: request.estimate.0,
                    hold_ttl: request.hold_ttl_seconds,
                });
            }
            Ok(Authorization {
                allowed: self.allow,
                action_token: self.allow.then(|| "agt_test".to_string()),
                cap_credits: self.cap.map(Credits),
                reason: None,
            })
        }

        async fn charge(&self, request: ChargeRequest) -> Result<Receipt, DomainError> {
            if let Ok(mut events) = self.events.lock() {
                events.push(RecordedCall::Charge {
                    action_token: request.action_token,
                    credits: request.credits.0,
                    idempotency_key: request.idempotency_key,
                });
            }
            Ok(Receipt {
                credits_charged: request.credits,
                idempotent_replay: false,
            })
        }
    }

    struct FixedGenerator;

    #[async_trait]
    impl Generator for FixedGenerator {
        async fn generate(
            &self,
            _request: GenerationRequest,
        ) -> Result<GeneratedNote, DomainError> {
            Ok(GeneratedNote {
                content: "Generated note".to_string(),
                title_suggestion: Some("Title".to_string()),
                provider: "test".to_string(),
                usage: TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: 20,
                },
            })
        }
    }

    struct FixedCleaner;

    #[async_trait]
    impl Cleaner for FixedCleaner {
        async fn cleanup(&self, _request: CleanupRequest) -> Result<CleanedText, DomainError> {
            Ok(CleanedText {
                text: "Hello".to_string(),
                provider: "test".to_string(),
                usage: TokenUsage {
                    prompt_tokens: 5,
                    completion_tokens: 6,
                },
            })
        }
    }

    struct FixedTranscriber;

    #[async_trait]
    impl Transcriber for FixedTranscriber {
        async fn transcribe(
            &self,
            _request: TranscriptionRequest,
        ) -> Result<Transcript, DomainError> {
            Ok(Transcript {
                text: "Transcript".to_string(),
                language: Some("en".to_string()),
                provider: "test".to_string(),
            })
        }
    }

    struct FixedDurationProbe;

    impl AudioDurationProbe for FixedDurationProbe {
        fn probe(&self, _audio: &[u8]) -> Result<Duration, DomainError> {
            Ok(Duration::from_millis(1500))
        }
    }
}
