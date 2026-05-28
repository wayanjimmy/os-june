use clap::{Parser, Subcommand};
use scribe_api::{ApiLimits, ApiState, ApiStateParams};
use scribe_config::{AppConfig, ModelProvider};
use scribe_providers::{
    JwksTokenVerifier, MultiFormatDurationProbe, OsAccountsHttpClient, RoutingTranscriber,
    VeniceCleaner, VeniceGenerator, default_client, jwks_client,
};
use scribe_services::{
    DictateService, DictateServiceDeps, NoteGenerateService, NoteGenerateServiceDeps,
    NoteTranscribeService, NoteTranscribeServiceDeps, PricingTable,
};
use std::{net::SocketAddr, sync::Arc};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    init_tracing();
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve().await,
    }
}

async fn serve() -> anyhow::Result<()> {
    let config = scribe_config::load()?;
    let address: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    let app = build_router(&config);
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "scribe-api listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn build_router(config: &AppConfig) -> axum::Router {
    let http = default_client();
    let openai_model_ids = config
        .pricing
        .iter()
        .filter(|(_, model)| model.provider == ModelProvider::Openai)
        .map(|(model_id, _)| model_id.clone())
        .collect::<Vec<_>>();

    let pricing = Arc::new(PricingTable::new(config.pricing.clone()));
    let os_accounts: Arc<dyn scribe_domain::OsAccountsClient> = Arc::new(
        OsAccountsHttpClient::from_config(http.clone(), &config.os_accounts),
    );
    let transcriber: Arc<dyn scribe_domain::Transcriber> = Arc::new(
        RoutingTranscriber::from_config(http.clone(), &config.upstreams, openai_model_ids),
    );
    let generator: Arc<dyn scribe_domain::Generator> = Arc::new(VeniceGenerator::from_config(
        http.clone(),
        &config.upstreams.venice,
    ));
    let cleaner: Arc<dyn scribe_domain::Cleaner> = Arc::new(VeniceCleaner::from_config(
        http.clone(),
        &config.upstreams.venice,
    ));
    let duration_probe: Arc<dyn scribe_domain::AudioDurationProbe> =
        Arc::new(MultiFormatDurationProbe);
    let token_verifier: Arc<dyn scribe_domain::TokenVerifier> = Arc::new(
        JwksTokenVerifier::from_config(jwks_client(), &config.os_accounts),
    );

    let flat_estimate_credits = config.os_accounts.flat_estimate_credits;

    let note_transcribe = Arc::new(NoteTranscribeService::new(NoteTranscribeServiceDeps {
        pricing: pricing.clone(),
        os_accounts: os_accounts.clone(),
        transcriber: transcriber.clone(),
        duration_probe: duration_probe.clone(),
        hold_ttl_seconds: config.os_accounts.authorize_hold_ttl_note_transcribe_secs,
        flat_estimate_credits,
    }));
    let note_generate = Arc::new(NoteGenerateService::new(NoteGenerateServiceDeps {
        pricing: pricing.clone(),
        os_accounts: os_accounts.clone(),
        generator,
        hold_ttl_seconds: config.os_accounts.authorize_hold_ttl_note_generate_secs,
        flat_estimate_credits,
    }));
    let dictate = Arc::new(DictateService::new(DictateServiceDeps {
        pricing: pricing.clone(),
        os_accounts,
        transcriber,
        cleaner,
        duration_probe,
        transcribe_hold_ttl_seconds: config
            .os_accounts
            .authorize_hold_ttl_dictate_transcribe_secs,
        cleanup_hold_ttl_seconds: config.os_accounts.authorize_hold_ttl_dictate_cleanup_secs,
        flat_estimate_credits,
    }));

    let state = ApiState::new(ApiStateParams {
        pricing,
        token_verifier,
        note_transcribe,
        note_generate,
        dictate,
        limits: ApiLimits {
            max_audio_bytes: config.server.max_audio_bytes,
            max_json_bytes: config.server.max_json_bytes,
            request_timeout_secs: config.server.request_timeout_secs,
        },
    });
    scribe_api::router(state)
}

fn init_tracing() {
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "scribe=info,scribe_api=info,scribe_services=info,scribe_providers=info,tower_http=info".into()
            }),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .try_init();
}
