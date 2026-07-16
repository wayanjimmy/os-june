//! Dev-only Notion hosted MCP Phase 0 prototype for validating ADR 0025.
//!
//! This is not the production Notion connector. It deliberately exposes a small
//! debug command surface that lets a human verify the hard architecture
//! questions: OAuth/PKCE, dynamic-client registration reuse, Keychain-only
//! custody, an `rmcp` Streamable HTTP client owned by Rust, live inventory
//! capture, fail-closed policy classification, and the current selected-resource
//! scoping risk. Hermes is not wired to `https://mcp.notion.com/mcp` here.

use crate::domain::types::AppError;
use reqwest::StatusCode;
use rmcp::{
    model::ClientInfo,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::oauth::{self, ConnectFlow, LoopbackPort};

const KEYCHAIN_SERVICE: &str = "co.opensoftware.june.notion";
const DEV_KEYCHAIN_SERVICE: &str = "co.opensoftware.june-dev.notion";
const TOKEN_ACCOUNT_ID: &str = "notion-mcp-oauth-phase0-token";
const REGISTRATION_ACCOUNT_ID: &str = "notion-mcp-oauth-phase0-registration";
const LEGACY_ACCOUNT_ID: &str = "notion-mcp-oauth-spike";
const MCP_SERVER_URL: &str = "https://mcp.notion.com/mcp";
const PROTECTED_RESOURCE_METADATA_URL: &str =
    "https://mcp.notion.com/.well-known/oauth-protected-resource";
const AUTH_SERVER_METADATA_URL: &str =
    "https://mcp.notion.com/.well-known/oauth-authorization-server";
const NOTION_ORIGIN: &str = "https://mcp.notion.com";
const LOOPBACK_PORT_RANGE_START: u16 = 44751;
const LOOPBACK_PORT_RANGE_END: u16 = 44850;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const ACCESS_TOKEN_EXPIRY_BUFFER_SECS: i64 = 60;
const INVENTORY_RESPONSE_CAP_BYTES: usize = 512 * 1024;
const ACCEPTED_SCHEMA_FINGERPRINTS: &[(&str, &str)] = &[];

static NOTION_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static REFRESH_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn notion_http_client() -> &'static reqwest::Client {
    NOTION_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .user_agent("os-june/0.1 notion-mcp-phase0")
            .build()
            .expect("build hardened Notion MCP HTTP client")
    })
}

fn refresh_lock() -> &'static tokio::sync::Mutex<()> {
    REFRESH_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn rmcp_http_client() -> reqwest13::Client {
    reqwest13::Client::builder()
        .no_proxy()
        .redirect(reqwest13::redirect::Policy::none())
        .timeout(HTTP_TIMEOUT)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Some(Duration::from_secs(30)))
        .user_agent("os-june/0.1 notion-mcp-phase0-rmcp")
        .build()
        .expect("build hardened rmcp Notion MCP HTTP client")
}

#[derive(Debug, Clone, Deserialize)]
struct ProtectedResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionMcpOAuthDiscoverySummary {
    pub protected_resource: String,
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub revocation_endpoint: Option<String>,
    pub supports_s256: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    revocation_endpoint: Option<String>,
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
struct StoredNotionMcpOAuth {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[zeroize(skip)]
    expires_at_epoch_seconds: Option<i64>,
    #[zeroize(skip)]
    token_type: Option<String>,
    #[zeroize(skip)]
    discovery: StoredDiscovery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredDiscovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
    revocation_endpoint: Option<String>,
}

impl StoredDiscovery {
    fn matches(&self, discovery: &NotionMcpOAuthDiscoverySummary) -> bool {
        self.issuer == discovery.issuer
            && self.authorization_endpoint == discovery.authorization_endpoint
            && self.token_endpoint == discovery.token_endpoint
            && self.registration_endpoint == discovery.registration_endpoint
    }
}

impl StoredClientRegistration {
    fn is_reusable_for(
        &self,
        discovery: &NotionMcpOAuthDiscoverySummary,
        redirect_uri: &str,
    ) -> bool {
        self.discovery.matches(discovery)
            && !self.client_id.is_empty()
            && self.registered_redirect_uris() == [redirect_uri]
            && loopback_port_from_redirect_uri(redirect_uri).is_some()
    }

    fn registered_redirect_uris(&self) -> Vec<&str> {
        if !self.redirect_uri.is_empty() {
            return vec![self.redirect_uri.as_str()];
        }
        self.redirect_uris.iter().map(String::as_str).collect()
    }
}

fn loopback_ports() -> Vec<u16> {
    (LOOPBACK_PORT_RANGE_START..=LOOPBACK_PORT_RANGE_END).collect()
}

fn loopback_redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/callback")
}

fn loopback_port_from_redirect_uri(uri: &str) -> Option<u16> {
    let suffix = uri.strip_prefix("http://127.0.0.1:")?;
    let port = suffix.strip_suffix("/callback")?.parse::<u16>().ok()?;
    (LOOPBACK_PORT_RANGE_START..=LOOPBACK_PORT_RANGE_END)
        .contains(&port)
        .then_some(port)
}

#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
struct StoredClientRegistration {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    #[zeroize(skip)]
    redirect_uri: String,
    #[serde(default)]
    #[zeroize(skip)]
    redirect_uris: Vec<String>,
    #[zeroize(skip)]
    discovery: StoredDiscovery,
}

#[derive(Debug, Clone, Serialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
struct ClientRegistration {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    #[zeroize(skip)]
    redirect_uri: String,
    #[serde(default)]
    token_endpoint_auth_method: Option<String>,
}

#[derive(Debug, Deserialize, Zeroize, ZeroizeOnDrop)]
struct ClientRegistrationResponse {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    token_endpoint_auth_method: Option<String>,
}

#[derive(Debug, Deserialize, Zeroize, ZeroizeOnDrop)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    token_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorBody {
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionMcpOAuthConnection {
    pub connected: bool,
    pub keychain_service: &'static str,
    pub client_id_present: bool,
    pub access_token_present: bool,
    pub refresh_token_present: bool,
    pub registration_reused: bool,
    pub expires_at_epoch_seconds: Option<i64>,
    pub discovery: NotionMcpOAuthDiscoverySummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionMcpOAuthStatus {
    pub connected: bool,
    pub keychain_service: &'static str,
    pub client_id_present: bool,
    pub access_token_present: bool,
    pub refresh_token_present: bool,
    pub expires_at_epoch_seconds: Option<i64>,
    pub keychain_only: bool,
    pub transport: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotionMcpPolicyClassification {
    AllowedReadExactPageCandidate,
    RejectedUnconstrainedRead,
    RejectedWrite,
    RejectedUnknown,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionMcpToolSummary {
    pub hosted_name: String,
    pub june_tool_name: Option<String>,
    pub description_present: bool,
    pub input_schema_present: bool,
    pub output_schema_present: bool,
    pub annotations_present: bool,
    pub schema_fingerprint: String,
    pub classification: NotionMcpPolicyClassification,
    pub enabled: bool,
    pub selected_resource_pre_call_constraint: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionMcpToolListResult {
    pub tool_count: usize,
    pub transport: &'static str,
    pub endpoint: &'static str,
    pub inventory_bytes: usize,
    pub over_cap_rejected: bool,
    pub tools: Vec<NotionMcpToolSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionMcpPhase0Report {
    pub connected: bool,
    pub endpoint: &'static str,
    pub transport: &'static str,
    pub keychain_service: &'static str,
    pub credential_custody: &'static str,
    pub oauth_lifecycle: BTreeMap<&'static str, &'static str>,
    pub standards_client: BTreeMap<&'static str, &'static str>,
    pub inventory_contract: BTreeMap<&'static str, &'static str>,
    pub selected_resource_matrix: BTreeMap<&'static str, &'static str>,
    pub approval_proof: BTreeMap<&'static str, &'static str>,
    pub output_observability: BTreeMap<&'static str, &'static str>,
    pub provider_behavior: BTreeMap<&'static str, &'static str>,
    pub current_recommendation: &'static str,
}

pub async fn connect(flow: &ConnectFlow) -> Result<NotionMcpOAuthConnection, AppError> {
    let discovery = discover().await?;
    let existing_registration = load_registration().await?;
    let selected_registration = Arc::new(std::sync::Mutex::new(None::<(ClientRegistration, bool)>));
    let registration_slot = Arc::clone(&selected_registration);
    let discovery_for_auth = discovery.clone();
    let authorization = oauth::loopback_authorize_async(
        flow,
        "Notion",
        LoopbackPort::Candidates(loopback_ports()),
        move |redirect_uri, code_challenge, state| {
            let existing_registration = existing_registration.clone();
            let discovery = discovery_for_auth.clone();
            let registration_slot = Arc::clone(&registration_slot);
            async move {
                let (registration, registration_reused) = match existing_registration
                    .as_ref()
                    .filter(|stored| stored.is_reusable_for(&discovery, &redirect_uri))
                {
                    Some(stored) => (
                        ClientRegistration {
                            client_id: stored.client_id.clone(),
                            client_secret: stored.client_secret.clone(),
                            redirect_uri: redirect_uri.clone(),
                            token_endpoint_auth_method: None,
                        },
                        true,
                    ),
                    _ => {
                        let registration = register_client(&discovery, &redirect_uri).await?;
                        store_registration(&StoredClientRegistration {
                            client_id: registration.client_id.clone(),
                            client_secret: registration.client_secret.clone(),
                            redirect_uri: registration.redirect_uri.clone(),
                            redirect_uris: Vec::new(),
                            discovery: StoredDiscovery {
                                issuer: discovery.issuer.clone(),
                                authorization_endpoint: discovery.authorization_endpoint.clone(),
                                token_endpoint: discovery.token_endpoint.clone(),
                                registration_endpoint: discovery.registration_endpoint.clone(),
                                revocation_endpoint: discovery.revocation_endpoint.clone(),
                            },
                        })
                        .await?;
                        (registration, false)
                    }
                };
                let auth_url = build_authorize_url(
                    &discovery.authorization_endpoint,
                    &registration.client_id,
                    &redirect_uri,
                    &code_challenge,
                    &state,
                );
                *registration_slot.lock().map_err(|_| {
                    AppError::new(
                        "notion_mcp_oauth_registration_state_unavailable",
                        "The Notion hosted MCP registration state is unavailable.",
                    )
                })? = Some((registration, registration_reused));
                Ok(auth_url)
            }
        },
    )
    .await?;
    let (registration, registration_reused) = selected_registration
        .lock()
        .map_err(|_| {
            AppError::new(
                "notion_mcp_oauth_registration_state_unavailable",
                "The Notion hosted MCP registration state is unavailable.",
            )
        })?
        .clone()
        .ok_or_else(|| {
            AppError::new(
                "notion_mcp_oauth_registration_state_unavailable",
                "The Notion hosted MCP registration was not prepared.",
            )
        })?;

    let tokens = exchange_code(
        &discovery,
        &registration,
        &authorization.code,
        &authorization.verifier,
        &authorization.redirect_uri,
    )
    .await?;
    let stored = stored_from_token_response(&discovery, &registration, tokens);
    store_token(&stored).await?;

    Ok(NotionMcpOAuthConnection {
        connected: true,
        keychain_service: keychain_service(),
        client_id_present: true,
        access_token_present: true,
        refresh_token_present: stored.refresh_token.is_some(),
        registration_reused,
        expires_at_epoch_seconds: stored.expires_at_epoch_seconds,
        discovery,
    })
}

pub async fn status() -> Result<NotionMcpOAuthStatus, AppError> {
    let stored = load_token().await?;
    Ok(NotionMcpOAuthStatus {
        connected: stored.is_some(),
        keychain_service: keychain_service(),
        client_id_present: stored
            .as_ref()
            .is_some_and(|stored| !stored.client_id.is_empty()),
        access_token_present: stored
            .as_ref()
            .is_some_and(|stored| !stored.access_token.is_empty()),
        refresh_token_present: stored
            .as_ref()
            .is_some_and(|stored| stored.refresh_token.is_some()),
        expires_at_epoch_seconds: stored.and_then(|stored| stored.expires_at_epoch_seconds),
        keychain_only: cfg!(any(target_os = "macos", target_os = "windows")),
        transport: "rmcp_streamable_http_with_hardened_reqwest_client",
    })
}

pub async fn disconnect() -> Result<(), AppError> {
    delete_token().await
}

pub async fn list_tools() -> Result<NotionMcpToolListResult, AppError> {
    let tools = list_all_tools_with_refresh().await?;
    summarize_tools(tools)
}

pub async fn phase0_report() -> Result<NotionMcpPhase0Report, AppError> {
    let connected = load_token().await?.is_some();
    Ok(NotionMcpPhase0Report {
        connected,
        endpoint: MCP_SERVER_URL,
        transport: "rmcp_streamable_http_with_hardened_reqwest_client",
        keychain_service: keychain_service(),
        credential_custody: if cfg!(any(target_os = "macos", target_os = "windows")) {
            "token and dynamic-client material are stored in the OS Keychain only, in separate token and registration items"
        } else {
            "blocked: this platform has no approved Keychain-backed storage"
        },
        oauth_lifecycle: BTreeMap::from([
            ("pkce_loopback", "implemented with system-browser loopback ports"),
            ("dynamic_registration_reuse", "implemented when a stored registration matches discovery metadata"),
            ("refresh", "implemented with one coalesced serialized refresh before stale reads and one retry after read failure"),
            ("invalid_grant", "refresh deletes the token grant while preserving dynamic registration"),
            ("provider_revocation", "manual live verification still required"),
        ]),
        standards_client: BTreeMap::from([
            ("sdk", "rmcp 2.2 StreamableHttpClientTransport"),
            ("endpoint", MCP_SERVER_URL),
            ("paginated_inventory", "uses rmcp list_all_tools"),
            ("session_reconnect", "rmcp reinit_on_expired_session remains enabled"),
            ("byte_caps", "not proven: prototype rejects oversized serialized inventory only after rmcp receives it"),
        ]),
        inventory_contract: BTreeMap::from([
            ("fingerprints", "computed from each hosted input schema"),
            ("drift", "fail-closed: tools are disabled unless hosted name and schema fingerprint match an accepted pair"),
            ("write_classification", "June policy classifies create/update/append/delete/comment tools as writes regardless of annotations"),
        ]),
        selected_resource_matrix: BTreeMap::from([
            ("notion-fetch", "narrow go candidate only for exact selected page IDs; arbitrary URL or ID remains rejected until a selected-root validator is wired"),
            ("notion-search", "rejected for v1 unless Notion exposes a pre-call root constraint"),
            ("data_source_query", "unknown until live inventory proves exact data-source constraint"),
        ]),
        approval_proof: BTreeMap::from([
            ("v1", "writes are not exposed"),
            ("v2", "not implemented in this prototype; local policy endpoint still required before writes"),
        ]),
        output_observability: BTreeMap::from([
            ("diagnostics", "errors include operation class/status only, never response bodies or tokens"),
            ("inventory_cap", "serialized inventory over 512 KiB is rejected from the command result"),
            ("sentinel_audit", "manual grep of app logs/Hermes home/SQLite still required after live run"),
        ]),
        provider_behavior: BTreeMap::from([
            ("401", "read path attempts one coalesced forced refresh after an rmcp read failure; exact 401 classification is still SDK-limited"),
            ("429_retry_after", "not yet proven through rmcp surface; manual live/fake-server test required"),
            ("connected_source_search", "notion-search remains rejected unless scoped pre-call"),
        ]),
        current_recommendation: "narrow go only if live inventory confirms exact-page reads can be validated before tools/call; search/traversal remain excluded",
    })
}

async fn list_all_tools_with_refresh() -> Result<Vec<rmcp::model::Tool>, AppError> {
    let stored = fresh_stored_token().await?;
    match list_all_tools_once(&stored.access_token).await {
        Ok(tools) => Ok(tools),
        Err(first_error) if stored.refresh_token.is_some() => {
            tracing::warn!(error_code = %first_error.code, "notion hosted mcp tools/list failed; attempting one forced refresh for read retry");
            let refreshed = refresh_stored_token(true, Some(&stored.access_token)).await?;
            list_all_tools_once(&refreshed.access_token).await
        }
        Err(error) => Err(error),
    }
}

async fn list_all_tools_once(access_token: &str) -> Result<Vec<rmcp::model::Tool>, AppError> {
    let config = StreamableHttpClientTransportConfig::with_uri(MCP_SERVER_URL)
        .auth_header(access_token)
        .reinit_on_expired_session(true);
    let transport = StreamableHttpClientTransport::with_client(rmcp_http_client(), config);
    let client = ClientInfo::default()
        .serve(transport)
        .await
        .map_err(|_error| {
            tracing::warn!(
                outcome = "initialize_failed",
                "notion hosted mcp rmcp initialize failed"
            );
            AppError::new(
                "notion_mcp_oauth_mcp_failed",
                "Could not initialize Notion hosted MCP with rmcp.",
            )
        })?;
    let result = client.peer().list_all_tools().await.map_err(|_error| {
        tracing::warn!(
            outcome = "tools_list_failed",
            "notion hosted mcp rmcp tools/list failed"
        );
        AppError::new(
            "notion_mcp_oauth_mcp_failed",
            "Could not list Notion hosted MCP tools with rmcp.",
        )
    });
    let _ = client.cancel().await;
    result
}

fn summarize_tools(tools: Vec<rmcp::model::Tool>) -> Result<NotionMcpToolListResult, AppError> {
    let raw = serde_json::to_vec(&tools).map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_invalid_response",
            "Could not encode Notion hosted MCP inventory.",
        )
    })?;
    let inventory_bytes = raw.len();
    if inventory_bytes > INVENTORY_RESPONSE_CAP_BYTES {
        return Ok(NotionMcpToolListResult {
            tool_count: tools.len(),
            transport: "rmcp_streamable_http_with_hardened_reqwest_client",
            endpoint: MCP_SERVER_URL,
            inventory_bytes,
            over_cap_rejected: true,
            tools: Vec::new(),
        });
    }
    let summaries = tools
        .into_iter()
        .map(|tool| {
            let hosted_name = tool.name.to_string();
            let schema_fingerprint = schema_fingerprint(&tool.input_schema);
            let classification = classify_hosted_tool(&hosted_name);
            let enabled = matches!(
                classification,
                NotionMcpPolicyClassification::AllowedReadExactPageCandidate
            ) && schema_fingerprint_is_accepted(&hosted_name, &schema_fingerprint);
            let june_tool_name = enabled.then(|| june_tool_name(&hosted_name));
            let selected_resource_pre_call_constraint =
                selected_resource_constraint(&hosted_name).to_string();
            NotionMcpToolSummary {
                hosted_name,
                june_tool_name,
                description_present: tool.description.is_some(),
                input_schema_present: true,
                output_schema_present: tool.output_schema.is_some(),
                annotations_present: tool.annotations.is_some(),
                schema_fingerprint,
                classification,
                enabled,
                selected_resource_pre_call_constraint,
            }
        })
        .collect::<Vec<_>>();
    Ok(NotionMcpToolListResult {
        tool_count: summaries.len(),
        transport: "rmcp_streamable_http_with_hardened_reqwest_client",
        endpoint: MCP_SERVER_URL,
        inventory_bytes,
        over_cap_rejected: false,
        tools: summaries,
    })
}

fn classify_hosted_tool(hosted_name: &str) -> NotionMcpPolicyClassification {
    let lower = hosted_name.to_ascii_lowercase();
    if lower.contains("create")
        || lower.contains("update")
        || lower.contains("append")
        || lower.contains("delete")
        || lower.contains("comment")
    {
        return NotionMcpPolicyClassification::RejectedWrite;
    }
    if hosted_name == "notion-fetch" {
        return NotionMcpPolicyClassification::AllowedReadExactPageCandidate;
    }
    if hosted_name == "notion-search" || lower.contains("search") || lower.contains("query") {
        return NotionMcpPolicyClassification::RejectedUnconstrainedRead;
    }
    NotionMcpPolicyClassification::RejectedUnknown
}

fn june_tool_name(hosted_name: &str) -> String {
    match hosted_name {
        "notion-fetch" => "june_notion_get_selected_page".to_string(),
        other => format!("june_notion_{}", other.replace('-', "_")),
    }
}

fn selected_resource_constraint(hosted_name: &str) -> &'static str {
    match hosted_name {
        "notion-fetch" => "candidate: validate exact page ID/URL against June-selected roots before tools/call; broad IDs rejected",
        "notion-search" => "rejected: no proven pre-call selected-root constraint",
        _ => "rejected until Phase 0 proves a pre-call selected-resource constraint",
    }
}

fn schema_fingerprint(input_schema: &Arc<rmcp::model::JsonObject>) -> String {
    let canonical = serde_json::to_vec(input_schema.as_ref()).unwrap_or_default();
    let digest = Sha256::digest(canonical);
    format!("sha256:{digest:x}")
}

fn schema_fingerprint_is_accepted(hosted_name: &str, fingerprint: &str) -> bool {
    ACCEPTED_SCHEMA_FINGERPRINTS
        .iter()
        .any(|(name, accepted)| *name == hosted_name && *accepted == fingerprint)
}

async fn discover() -> Result<NotionMcpOAuthDiscoverySummary, AppError> {
    let protected = notion_http_client()
        .get(PROTECTED_RESOURCE_METADATA_URL)
        .send()
        .await
        .map_err(|_| network_error("Could not discover Notion hosted MCP OAuth metadata."))?;
    let protected_status = protected.status();
    let protected_body = protected
        .text()
        .await
        .map_err(|_| network_error("Could not read Notion hosted MCP OAuth metadata."))?;
    if !protected_status.is_success() {
        return Err(upstream_error(
            "notion_mcp_oauth_discovery_failed",
            protected_status,
            &protected_body,
            "Could not discover Notion hosted MCP OAuth metadata.",
        ));
    }
    let protected =
        serde_json::from_str::<ProtectedResourceMetadata>(&protected_body).map_err(|_| {
            AppError::new(
                "notion_mcp_oauth_invalid_response",
                "Notion hosted MCP protected-resource metadata had an unexpected shape.",
            )
        })?;
    if protected.resource != NOTION_ORIGIN && protected.resource != MCP_SERVER_URL {
        return Err(AppError::new(
            "notion_mcp_oauth_untrusted_resource",
            "Notion hosted MCP discovery returned an unexpected protected resource.",
        ));
    }
    if !protected
        .authorization_servers
        .iter()
        .any(|server| server == NOTION_ORIGIN)
    {
        return Err(AppError::new(
            "notion_mcp_oauth_untrusted_issuer",
            "Notion hosted MCP discovery returned an unexpected authorization server.",
        ));
    }

    let metadata = notion_http_client()
        .get(AUTH_SERVER_METADATA_URL)
        .send()
        .await
        .map_err(|_| network_error("Could not load Notion hosted MCP OAuth server metadata."))?;
    let metadata_status = metadata.status();
    let metadata_body = metadata
        .text()
        .await
        .map_err(|_| network_error("Could not read Notion hosted MCP OAuth server metadata."))?;
    if !metadata_status.is_success() {
        return Err(upstream_error(
            "notion_mcp_oauth_discovery_failed",
            metadata_status,
            &metadata_body,
            "Could not load Notion hosted MCP OAuth server metadata.",
        ));
    }
    let metadata = serde_json::from_str::<OAuthServerMetadata>(&metadata_body).map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_invalid_response",
            "Notion hosted MCP OAuth server metadata had an unexpected shape.",
        )
    })?;
    validate_notion_url(&metadata.issuer, "issuer")?;
    validate_notion_url(&metadata.authorization_endpoint, "authorization endpoint")?;
    validate_notion_url(&metadata.token_endpoint, "token endpoint")?;
    if let Some(endpoint) = metadata.registration_endpoint.as_deref() {
        validate_notion_url(endpoint, "registration endpoint")?;
    }
    if let Some(endpoint) = metadata.revocation_endpoint.as_deref() {
        validate_notion_url(endpoint, "revocation endpoint")?;
    }
    let supports_s256 = metadata
        .code_challenge_methods_supported
        .iter()
        .any(|method| method == "S256");
    if !supports_s256 {
        return Err(AppError::new(
            "notion_mcp_oauth_pkce_unavailable",
            "Notion hosted MCP OAuth did not advertise S256 PKCE support.",
        ));
    }
    Ok(NotionMcpOAuthDiscoverySummary {
        protected_resource: protected.resource,
        issuer: metadata.issuer,
        authorization_endpoint: metadata.authorization_endpoint,
        token_endpoint: metadata.token_endpoint,
        registration_endpoint: metadata.registration_endpoint,
        revocation_endpoint: metadata.revocation_endpoint,
        supports_s256,
    })
}

async fn register_client(
    discovery: &NotionMcpOAuthDiscoverySummary,
    redirect_uri: &str,
) -> Result<ClientRegistration, AppError> {
    let endpoint = discovery.registration_endpoint.as_deref().ok_or_else(|| {
        AppError::new(
            "notion_mcp_oauth_registration_unavailable",
            "Notion hosted MCP OAuth did not advertise dynamic client registration.",
        )
    })?;
    let response = notion_http_client()
        .post(endpoint)
        .json(&json!({
            "client_name": "June Notion hosted MCP Phase 0",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        }))
        .send()
        .await
        .map_err(|_| network_error("Could not register the Notion hosted MCP OAuth client."))?;
    let status = response.status();
    let body = response.text().await.map_err(|_| {
        network_error("Could not read the Notion hosted MCP registration response.")
    })?;
    if !status.is_success() {
        return Err(upstream_error(
            "notion_mcp_oauth_registration_failed",
            status,
            &body,
            "Could not register the Notion hosted MCP OAuth client.",
        ));
    }
    let registration = serde_json::from_str::<ClientRegistrationResponse>(&body).map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_invalid_response",
            "Notion hosted MCP registration returned an unexpected response.",
        )
    })?;
    if let Some(method) = registration.token_endpoint_auth_method.as_deref() {
        if method != "none" && method != "client_secret_post" {
            return Err(AppError::new(
                "notion_mcp_oauth_unsupported_client_auth",
                "Notion hosted MCP registration returned an unsupported token auth method.",
            ));
        }
    }
    if registration.client_id.is_empty() {
        return Err(AppError::new(
            "notion_mcp_oauth_invalid_response",
            "Notion hosted MCP registration did not return a client id.",
        ));
    }
    Ok(ClientRegistration {
        client_id: registration.client_id.clone(),
        client_secret: registration.client_secret.clone(),
        redirect_uri: redirect_uri.to_string(),
        token_endpoint_auth_method: registration.token_endpoint_auth_method.clone(),
    })
}

fn build_authorize_url(
    authorization_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    code_challenge: &str,
    state: &str,
) -> String {
    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope=&state={}&code_challenge={}&code_challenge_method=S256&prompt=consent",
        authorization_endpoint,
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(state),
        urlencoding::encode(code_challenge),
    )
}

async fn exchange_code(
    discovery: &NotionMcpOAuthDiscoverySummary,
    registration: &ClientRegistration,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, AppError> {
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", registration.client_id.as_str()),
        ("redirect_uri", redirect_uri),
        ("code_verifier", verifier),
    ];
    if let Some(client_secret) = registration.client_secret.as_deref() {
        form.push(("client_secret", client_secret));
    }
    let response = notion_http_client()
        .post(&discovery.token_endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|_| network_error("Could not complete the Notion hosted MCP OAuth exchange."))?;
    let status = response.status();
    let body = response.text().await.map_err(|_| {
        network_error("Could not read the Notion hosted MCP OAuth exchange response.")
    })?;
    if !status.is_success() {
        return Err(upstream_error(
            "notion_mcp_oauth_token_exchange_failed",
            status,
            &body,
            "Could not complete the Notion hosted MCP OAuth exchange.",
        ));
    }
    parse_token_response(&body)
}

async fn fresh_stored_token() -> Result<StoredNotionMcpOAuth, AppError> {
    let stored = load_token().await?.ok_or_else(not_connected_error)?;
    if stored
        .expires_at_epoch_seconds
        .is_some_and(|expires| expires <= now_epoch_seconds() + ACCESS_TOKEN_EXPIRY_BUFFER_SECS)
        && stored.refresh_token.is_some()
    {
        return refresh_stored_token(false, None).await;
    }
    Ok(stored)
}

async fn refresh_stored_token(
    force: bool,
    observed_access_token: Option<&str>,
) -> Result<StoredNotionMcpOAuth, AppError> {
    let _guard = refresh_lock().lock().await;
    let stored = load_token().await?.ok_or_else(not_connected_error)?;
    if should_reuse_stored_token_after_lock(&stored, force, observed_access_token) {
        return Ok(stored);
    }
    let Some(refresh_token) = stored.refresh_token.as_deref() else {
        return Err(reconnect_required_error());
    };
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", stored.client_id.as_str()),
    ];
    if let Some(client_secret) = stored.client_secret.as_deref() {
        form.push(("client_secret", client_secret));
    }
    let response = notion_http_client()
        .post(&stored.discovery.token_endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|_| network_error("Could not refresh the Notion hosted MCP OAuth token."))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|_| network_error("Could not read the Notion hosted MCP refresh response."))?;
    if !status.is_success() {
        let oauth_error = parse_oauth_error(&body);
        if oauth_error.as_deref() == Some("invalid_grant") {
            delete_token().await?;
            return Err(reconnect_required_error());
        }
        return Err(upstream_error(
            "notion_mcp_oauth_refresh_failed",
            status,
            &body,
            "Could not refresh the Notion hosted MCP OAuth token.",
        ));
    }
    let discovery = NotionMcpOAuthDiscoverySummary {
        protected_resource: MCP_SERVER_URL.to_string(),
        issuer: stored.discovery.issuer.clone(),
        authorization_endpoint: stored.discovery.authorization_endpoint.clone(),
        token_endpoint: stored.discovery.token_endpoint.clone(),
        registration_endpoint: stored.discovery.registration_endpoint.clone(),
        revocation_endpoint: stored.discovery.revocation_endpoint.clone(),
        supports_s256: true,
    };
    let registration = ClientRegistration {
        client_id: stored.client_id.clone(),
        client_secret: stored.client_secret.clone(),
        redirect_uri: String::new(),
        token_endpoint_auth_method: None,
    };
    let mut refreshed =
        stored_from_token_response(&discovery, &registration, parse_token_response(&body)?);
    if refreshed.refresh_token.is_none() {
        refreshed.refresh_token = stored.refresh_token.clone();
    }
    store_token(&refreshed).await?;
    Ok(refreshed)
}

fn stored_from_token_response(
    discovery: &NotionMcpOAuthDiscoverySummary,
    registration: &ClientRegistration,
    tokens: TokenResponse,
) -> StoredNotionMcpOAuth {
    StoredNotionMcpOAuth {
        client_id: registration.client_id.clone(),
        client_secret: registration.client_secret.clone(),
        access_token: tokens.access_token.clone(),
        refresh_token: tokens.refresh_token.clone(),
        expires_at_epoch_seconds: tokens
            .expires_in
            .map(|seconds| now_epoch_seconds() + seconds),
        token_type: tokens.token_type.clone(),
        discovery: StoredDiscovery {
            issuer: discovery.issuer.clone(),
            authorization_endpoint: discovery.authorization_endpoint.clone(),
            token_endpoint: discovery.token_endpoint.clone(),
            registration_endpoint: discovery.registration_endpoint.clone(),
            revocation_endpoint: discovery.revocation_endpoint.clone(),
        },
    }
}

fn should_reuse_stored_token_after_lock(
    stored: &StoredNotionMcpOAuth,
    force: bool,
    observed_access_token: Option<&str>,
) -> bool {
    if force {
        return observed_access_token.is_some_and(|observed| stored.access_token != observed);
    }
    stored
        .expires_at_epoch_seconds
        .is_some_and(|expires| expires > now_epoch_seconds() + ACCESS_TOKEN_EXPIRY_BUFFER_SECS)
}

fn parse_token_response(body: &str) -> Result<TokenResponse, AppError> {
    let tokens = serde_json::from_str::<TokenResponse>(body).map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_invalid_response",
            "Notion hosted MCP OAuth returned an unexpected token response.",
        )
    })?;
    if tokens.access_token.is_empty() {
        return Err(AppError::new(
            "notion_mcp_oauth_invalid_response",
            "Notion hosted MCP OAuth did not return an access token.",
        ));
    }
    if let Some(token_type) = tokens.token_type.as_deref() {
        if !token_type.eq_ignore_ascii_case("bearer") {
            return Err(AppError::new(
                "notion_mcp_oauth_unsupported_token_type",
                "Notion hosted MCP OAuth returned an unsupported token type.",
            ));
        }
    }
    Ok(tokens)
}

fn validate_notion_url(value: &str, label: &str) -> Result<(), AppError> {
    if value == NOTION_ORIGIN || value.starts_with(&format!("{NOTION_ORIGIN}/")) {
        return Ok(());
    }
    Err(AppError::new(
        "notion_mcp_oauth_untrusted_endpoint",
        format!("Notion hosted MCP discovery returned an unexpected {label}."),
    ))
}

async fn store_token(tokens: &StoredNotionMcpOAuth) -> Result<(), AppError> {
    let json = serde_json::to_string(tokens).map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_token_store_failed",
            "Could not encode Notion hosted MCP OAuth material.",
        )
    })?;
    store_keychain_json(TOKEN_ACCOUNT_ID, json).await
}

async fn load_token() -> Result<Option<StoredNotionMcpOAuth>, AppError> {
    load_keychain_json(TOKEN_ACCOUNT_ID)
        .await?
        .map(|raw| serde_json::from_str::<StoredNotionMcpOAuth>(&raw))
        .transpose()
        .map_err(|_| {
            AppError::new(
                "notion_mcp_oauth_keychain_read_failed",
                "Stored Notion hosted MCP OAuth material could not be parsed.",
            )
        })
}

async fn delete_token() -> Result<(), AppError> {
    delete_keychain_entries(&[TOKEN_ACCOUNT_ID, LEGACY_ACCOUNT_ID]).await
}

async fn store_registration(registration: &StoredClientRegistration) -> Result<(), AppError> {
    let json = serde_json::to_string(registration).map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_token_store_failed",
            "Could not encode Notion hosted MCP registration material.",
        )
    })?;
    store_keychain_json(REGISTRATION_ACCOUNT_ID, json).await
}

async fn load_registration() -> Result<Option<StoredClientRegistration>, AppError> {
    if let Some(raw) = load_keychain_json(REGISTRATION_ACCOUNT_ID).await? {
        return serde_json::from_str::<StoredClientRegistration>(&raw)
            .map(Some)
            .map_err(|_| {
                AppError::new(
                    "notion_mcp_oauth_keychain_read_failed",
                    "Stored Notion hosted MCP registration material could not be parsed.",
                )
            });
    }
    let Some(legacy) = load_keychain_json(LEGACY_ACCOUNT_ID).await? else {
        return Ok(None);
    };
    let legacy = serde_json::from_str::<StoredNotionMcpOAuth>(&legacy).map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_keychain_read_failed",
            "Stored Notion hosted MCP OAuth material could not be parsed.",
        )
    })?;
    Ok(Some(StoredClientRegistration {
        client_id: legacy.client_id.clone(),
        client_secret: legacy.client_secret.clone(),
        redirect_uri: String::new(),
        redirect_uris: Vec::new(),
        discovery: legacy.discovery.clone(),
    }))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn store_keychain_json(account_id: &'static str, json: String) -> Result<(), AppError> {
    let service = keychain_service().to_string();
    tokio::task::spawn_blocking(move || {
        keyring::Entry::new(&service, account_id).and_then(|entry| entry.set_password(&json))
    })
    .await
    .map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_keychain_write_failed",
            "Keychain write task failed.",
        )
    })?
    .map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_keychain_write_failed",
            "Could not store Notion hosted MCP OAuth material in Keychain.",
        )
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn store_keychain_json(_account_id: &'static str, _json: String) -> Result<(), AppError> {
    Err(secure_storage_unavailable())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn load_keychain_json(account_id: &'static str) -> Result<Option<String>, AppError> {
    let service = keychain_service().to_string();
    tokio::task::spawn_blocking(move || {
        keyring::Entry::new(&service, account_id)
            .and_then(|entry| entry.get_password())
            .map(Some)
            .or_else(|error| match error {
                keyring::Error::NoEntry => Ok(None),
                _ => Err(AppError::new(
                    "notion_mcp_oauth_keychain_read_failed",
                    "Could not read Notion hosted MCP OAuth material from Keychain.",
                )),
            })
    })
    .await
    .map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_keychain_read_failed",
            "Keychain read task failed.",
        )
    })?
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn load_keychain_json(_account_id: &'static str) -> Result<Option<String>, AppError> {
    Err(secure_storage_unavailable())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn delete_keychain_entries(account_ids: &[&'static str]) -> Result<(), AppError> {
    let service = keychain_service().to_string();
    let account_ids = account_ids.to_vec();
    tokio::task::spawn_blocking(move || {
        for account_id in account_ids {
            match keyring::Entry::new(&service, account_id)
                .and_then(|entry| entry.delete_credential())
            {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(_) => {
                    return Err(AppError::new(
                        "notion_mcp_oauth_keychain_delete_failed",
                        "Could not delete Notion hosted MCP OAuth material from Keychain.",
                    ));
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_keychain_delete_failed",
            "Keychain delete task failed.",
        )
    })?
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn delete_keychain_entries(_account_ids: &[&'static str]) -> Result<(), AppError> {
    Err(secure_storage_unavailable())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn secure_storage_unavailable() -> AppError {
    AppError::new(
        "notion_mcp_oauth_keychain_unavailable",
        "The Notion hosted MCP OAuth prototype requires Keychain-backed storage.",
    )
}

fn keychain_service() -> &'static str {
    if cfg!(debug_assertions) {
        DEV_KEYCHAIN_SERVICE
    } else {
        KEYCHAIN_SERVICE
    }
}

fn not_connected_error() -> AppError {
    AppError::new(
        "notion_mcp_oauth_not_connected",
        "Connect the Notion hosted MCP OAuth prototype before listing tools.",
    )
}

fn reconnect_required_error() -> AppError {
    AppError::new(
        "notion_mcp_oauth_reconnect_required",
        "Notion access expired. Reconnect Notion in settings.",
    )
}

fn network_error(message: &str) -> AppError {
    AppError::new("notion_mcp_oauth_network", message)
}

fn upstream_error(code: &'static str, status: StatusCode, body: &str, message: &str) -> AppError {
    let error_code = parse_oauth_error(body).and_then(|value| safe_oauth_error_code(&value));
    tracing::warn!(
        status = status.as_u16(),
        error_code = error_code.as_deref().unwrap_or("unclassified"),
        "notion hosted mcp oauth upstream request failed"
    );
    let suffix = error_code
        .map(|value| format!(" ({value})"))
        .unwrap_or_default();
    AppError::new(code, format!("{message}{suffix}"))
}

fn parse_oauth_error(body: &str) -> Option<String> {
    serde_json::from_str::<OAuthErrorBody>(body)
        .ok()
        .and_then(|body| body.error)
}

fn safe_oauth_error_code(value: &str) -> Option<String> {
    let is_safe = !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    is_safe.then(|| value.to_string())
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_rejects_writes_and_unconstrained_search() {
        assert!(matches!(
            classify_hosted_tool("notion-create-pages"),
            NotionMcpPolicyClassification::RejectedWrite
        ));
        assert!(matches!(
            classify_hosted_tool("notion-search"),
            NotionMcpPolicyClassification::RejectedUnconstrainedRead
        ));
        assert!(matches!(
            classify_hosted_tool("notion-fetch"),
            NotionMcpPolicyClassification::AllowedReadExactPageCandidate
        ));
    }

    #[test]
    fn schema_drift_disables_fetch_without_reviewed_fingerprint() {
        assert!(!schema_fingerprint_is_accepted(
            "notion-fetch",
            "sha256:unreviewed"
        ));
    }

    #[test]
    fn forced_refresh_reuses_newer_stored_grant() {
        let stored = StoredNotionMcpOAuth {
            client_id: "client".to_string(),
            client_secret: None,
            access_token: "token-b".to_string(),
            refresh_token: Some("refresh-b".to_string()),
            expires_at_epoch_seconds: Some(now_epoch_seconds() + 3600),
            token_type: Some("Bearer".to_string()),
            discovery: StoredDiscovery {
                issuer: NOTION_ORIGIN.to_string(),
                authorization_endpoint: format!("{NOTION_ORIGIN}/authorize"),
                token_endpoint: format!("{NOTION_ORIGIN}/token"),
                registration_endpoint: None,
                revocation_endpoint: None,
            },
        };
        assert!(should_reuse_stored_token_after_lock(
            &stored,
            true,
            Some("token-a")
        ));
        assert!(!should_reuse_stored_token_after_lock(
            &stored,
            true,
            Some("token-b")
        ));
    }

    #[test]
    fn oauth_error_codes_are_safe_identifiers_only() {
        assert_eq!(
            safe_oauth_error_code("invalid_grant").as_deref(),
            Some("invalid_grant")
        );
        assert!(safe_oauth_error_code("invalid grant with spaces").is_none());
        assert!(safe_oauth_error_code("tok_秘密").is_none());
    }

    #[test]
    fn loopback_parser_accepts_only_current_callback_range() {
        assert_eq!(
            loopback_port_from_redirect_uri("http://127.0.0.1:44751/callback"),
            Some(44751)
        );
        assert_eq!(
            loopback_port_from_redirect_uri("http://127.0.0.1:44850/callback"),
            Some(44850)
        );
        assert_eq!(
            loopback_port_from_redirect_uri("http://127.0.0.1:44750/callback"),
            None
        );
        assert_eq!(
            loopback_port_from_redirect_uri("http://127.0.0.1:44851/callback"),
            None
        );
        assert_eq!(
            loopback_port_from_redirect_uri("http://localhost:44751/callback"),
            None
        );
        assert_eq!(
            loopback_port_from_redirect_uri("http://127.0.0.1:44751/other"),
            None
        );
    }

    #[test]
    fn stored_registration_reuse_requires_exact_selected_redirect_uri() {
        let discovery = NotionMcpOAuthDiscoverySummary {
            protected_resource: MCP_SERVER_URL.to_string(),
            issuer: NOTION_ORIGIN.to_string(),
            authorization_endpoint: format!("{NOTION_ORIGIN}/authorize"),
            token_endpoint: format!("{NOTION_ORIGIN}/token"),
            registration_endpoint: Some(format!("{NOTION_ORIGIN}/register")),
            revocation_endpoint: None,
            supports_s256: true,
        };
        let stored_discovery = StoredDiscovery {
            issuer: discovery.issuer.clone(),
            authorization_endpoint: discovery.authorization_endpoint.clone(),
            token_endpoint: discovery.token_endpoint.clone(),
            registration_endpoint: discovery.registration_endpoint.clone(),
            revocation_endpoint: discovery.revocation_endpoint.clone(),
        };
        let selected_redirect_uri = "http://127.0.0.1:44751/callback";
        let reusable = StoredClientRegistration {
            client_id: "client".to_string(),
            client_secret: None,
            redirect_uri: selected_redirect_uri.to_string(),
            redirect_uris: Vec::new(),
            discovery: stored_discovery.clone(),
        };
        assert!(reusable.is_reusable_for(&discovery, selected_redirect_uri));
        assert!(!reusable.is_reusable_for(&discovery, "http://127.0.0.1:44752/callback"));

        let old_three_port_registration = StoredClientRegistration {
            client_id: "client".to_string(),
            client_secret: None,
            redirect_uri: String::new(),
            redirect_uris: vec![
                "http://127.0.0.1:44751/callback".to_string(),
                "http://127.0.0.1:44752/callback".to_string(),
                "http://127.0.0.1:44753/callback".to_string(),
            ],
            discovery: stored_discovery.clone(),
        };
        assert!(!old_three_port_registration.is_reusable_for(&discovery, selected_redirect_uri));

        let malformed_registration = StoredClientRegistration {
            client_id: "client".to_string(),
            client_secret: None,
            redirect_uri: "http://127.0.0.1:44851/callback".to_string(),
            redirect_uris: Vec::new(),
            discovery: stored_discovery,
        };
        assert!(!malformed_registration.is_reusable_for(&discovery, selected_redirect_uri));
    }

    #[test]
    fn old_registration_json_deserializes_without_redirect_uris() {
        let raw = serde_json::json!({
            "clientId": "client",
            "clientSecret": null,
            "discovery": {
                "issuer": NOTION_ORIGIN,
                "authorizationEndpoint": format!("{NOTION_ORIGIN}/authorize"),
                "tokenEndpoint": format!("{NOTION_ORIGIN}/token"),
                "registrationEndpoint": format!("{NOTION_ORIGIN}/register"),
                "revocationEndpoint": null
            }
        });
        let registration: StoredClientRegistration = serde_json::from_value(raw).unwrap();
        assert!(registration.redirect_uri.is_empty());
        assert!(registration.redirect_uris.is_empty());
    }

    #[test]
    fn legacy_combined_registration_migration_is_not_reusable() {
        let discovery = NotionMcpOAuthDiscoverySummary {
            protected_resource: MCP_SERVER_URL.to_string(),
            issuer: NOTION_ORIGIN.to_string(),
            authorization_endpoint: format!("{NOTION_ORIGIN}/authorize"),
            token_endpoint: format!("{NOTION_ORIGIN}/token"),
            registration_endpoint: Some(format!("{NOTION_ORIGIN}/register")),
            revocation_endpoint: None,
            supports_s256: true,
        };
        let stored = StoredClientRegistration {
            client_id: "legacy-client".to_string(),
            client_secret: None,
            redirect_uri: String::new(),
            redirect_uris: Vec::new(),
            discovery: StoredDiscovery {
                issuer: discovery.issuer.clone(),
                authorization_endpoint: discovery.authorization_endpoint.clone(),
                token_endpoint: discovery.token_endpoint.clone(),
                registration_endpoint: discovery.registration_endpoint.clone(),
                revocation_endpoint: discovery.revocation_endpoint.clone(),
            },
        };
        assert!(!stored.is_reusable_for(&discovery, "http://127.0.0.1:44751/callback"));
    }
}
