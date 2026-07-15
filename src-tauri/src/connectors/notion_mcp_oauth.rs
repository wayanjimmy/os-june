//! Dev-only Notion hosted MCP OAuth spike for validating ADR 0025.
//!
//! This is not the production Notion connector. It proves the narrow Phase 0
//! questions: dynamic client registration, localhost PKCE OAuth, Keychain-only
//! custody, and a Rust-owned hosted MCP `tools/list` call. Hermes is not wired
//! to `https://mcp.notion.com/mcp` by this module.

use crate::domain::types::AppError;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::oauth::{self, ConnectFlow, LoopbackPort};

const KEYCHAIN_SERVICE: &str = "co.opensoftware.june.notion";
const DEV_KEYCHAIN_SERVICE: &str = "co.opensoftware.june-dev.notion";
const ACCOUNT_ID: &str = "notion-mcp-oauth-spike";
const MCP_SERVER_URL: &str = "https://mcp.notion.com/mcp";
const PROTECTED_RESOURCE_METADATA_URL: &str =
    "https://mcp.notion.com/.well-known/oauth-protected-resource";
const AUTH_SERVER_METADATA_URL: &str =
    "https://mcp.notion.com/.well-known/oauth-authorization-server";
const NOTION_ORIGIN: &str = "https://mcp.notion.com";
const LOOPBACK_PORTS: &[u16] = &[44751, 44752, 44753];
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

static NOTION_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn notion_http_client() -> &'static reqwest::Client {
    NOTION_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .user_agent("os-june/0.1 notion-mcp-oauth-spike")
            .build()
            .expect("build hardened Notion MCP HTTP client")
    })
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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionMcpToolSummary {
    pub name: String,
    pub description_present: bool,
    pub input_schema_present: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionMcpToolListResult {
    pub tool_count: usize,
    pub tools: Vec<NotionMcpToolSummary>,
}

pub async fn connect(flow: &ConnectFlow) -> Result<NotionMcpOAuthConnection, AppError> {
    let discovery = discover().await?;
    let registration = register_client(&discovery).await?;
    let client_id_for_url = registration.client_id.clone();
    let authorization = oauth::loopback_authorize(
        flow,
        "Notion",
        LoopbackPort::Candidates(LOOPBACK_PORTS.to_vec()),
        |redirect_uri, code_challenge, state| {
            build_authorize_url(
                &discovery.authorization_endpoint,
                &client_id_for_url,
                redirect_uri,
                code_challenge,
                state,
            )
        },
    )
    .await?;

    let tokens = exchange_code(
        &discovery,
        &registration,
        &authorization.code,
        &authorization.verifier,
        &authorization.redirect_uri,
    )
    .await?;
    let expires_at_epoch_seconds = tokens
        .expires_in
        .map(|seconds| now_epoch_seconds() + seconds);
    let stored = StoredNotionMcpOAuth {
        client_id: registration.client_id.clone(),
        client_secret: registration.client_secret.clone(),
        access_token: tokens.access_token.clone(),
        refresh_token: tokens.refresh_token.clone(),
        expires_at_epoch_seconds,
        token_type: tokens.token_type.clone(),
        discovery: StoredDiscovery {
            issuer: discovery.issuer.clone(),
            authorization_endpoint: discovery.authorization_endpoint.clone(),
            token_endpoint: discovery.token_endpoint.clone(),
            registration_endpoint: discovery.registration_endpoint.clone(),
            revocation_endpoint: discovery.revocation_endpoint.clone(),
        },
    };
    store_token(&stored).await?;

    Ok(NotionMcpOAuthConnection {
        connected: true,
        keychain_service: keychain_service(),
        client_id_present: true,
        access_token_present: true,
        refresh_token_present: stored.refresh_token.is_some(),
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
    })
}

pub async fn disconnect() -> Result<(), AppError> {
    delete_token().await
}

pub async fn list_tools() -> Result<NotionMcpToolListResult, AppError> {
    let stored = load_token().await?.ok_or_else(not_connected_error)?;
    let requested_protocol_version = "2025-06-18";
    let initialized = mcp_post(
        &stored.access_token,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": requested_protocol_version,
                "capabilities": {},
                "clientInfo": { "name": "june-notion-mcp-oauth-spike", "version": "0.0.0" }
            }
        }),
    )
    .await?;
    reject_json_rpc_error(&initialized.body)?;
    let protocol_version = initialized
        .body
        .get("result")
        .and_then(|result| result.get("protocolVersion"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::new(
                "notion_mcp_oauth_invalid_response",
                "Notion hosted MCP did not return a protocol version.",
            )
        })?;
    if protocol_version != requested_protocol_version {
        return Err(AppError::new(
            "notion_mcp_oauth_protocol_mismatch",
            "Notion hosted MCP returned an unsupported protocol version.",
        ));
    }
    let session_id = initialized.session_id;
    mcp_send_notification(
        &stored.access_token,
        session_id.as_deref(),
        protocol_version,
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    )
    .await?;
    let listed = mcp_post_with_session(
        &stored.access_token,
        session_id.as_deref(),
        Some(protocol_version),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
    )
    .await?;
    reject_json_rpc_error(&listed.body)?;
    let tools_value = listed
        .body
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::new(
                "notion_mcp_oauth_invalid_response",
                "Notion hosted MCP returned an unexpected tools/list response.",
            )
        })?;
    let tools = tools_value
        .iter()
        .filter_map(|tool| {
            let name = tool.get("name")?.as_str()?.to_string();
            Some(NotionMcpToolSummary {
                name,
                description_present: tool.get("description").and_then(Value::as_str).is_some(),
                input_schema_present: tool.get("inputSchema").is_some(),
            })
        })
        .collect::<Vec<_>>();
    Ok(NotionMcpToolListResult {
        tool_count: tools.len(),
        tools,
    })
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
) -> Result<ClientRegistrationResponse, AppError> {
    let endpoint = discovery.registration_endpoint.as_deref().ok_or_else(|| {
        AppError::new(
            "notion_mcp_oauth_registration_unavailable",
            "Notion hosted MCP OAuth did not advertise dynamic client registration.",
        )
    })?;
    let redirect_uris = LOOPBACK_PORTS
        .iter()
        .map(|port| format!("http://127.0.0.1:{port}/callback"))
        .collect::<Vec<_>>();
    let response = notion_http_client()
        .post(endpoint)
        .json(&json!({
            "client_name": "June Notion hosted MCP OAuth spike",
            "redirect_uris": redirect_uris,
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
    Ok(registration)
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
    registration: &ClientRegistrationResponse,
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
    let tokens = serde_json::from_str::<TokenResponse>(&body).map_err(|_| {
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
    Ok(tokens)
}

struct McpResponse {
    body: Value,
    session_id: Option<String>,
}

async fn mcp_post(access_token: &str, body: Value) -> Result<McpResponse, AppError> {
    mcp_post_with_session(access_token, None, None, body).await
}

async fn mcp_post_with_session(
    access_token: &str,
    session_id: Option<&str>,
    protocol_version: Option<&str>,
    body: Value,
) -> Result<McpResponse, AppError> {
    let mut request = notion_http_client()
        .post(MCP_SERVER_URL)
        .bearer_auth(access_token)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&body);
    if let Some(session_id) = session_id {
        request = request.header("mcp-session-id", session_id);
    }
    if let Some(protocol_version) = protocol_version {
        request = request.header("mcp-protocol-version", protocol_version);
    }
    let response = request
        .send()
        .await
        .map_err(|_| network_error("Could not call Notion hosted MCP."))?;
    let status = response.status();
    let session_id = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = response
        .text()
        .await
        .map_err(|_| network_error("Could not read Notion hosted MCP response."))?;
    if !status.is_success() {
        return Err(upstream_error(
            "notion_mcp_oauth_mcp_failed",
            status,
            &text,
            "Notion hosted MCP request failed.",
        ));
    }
    let body = parse_mcp_response_body(&content_type, &text)?;
    Ok(McpResponse { body, session_id })
}

async fn mcp_send_notification(
    access_token: &str,
    session_id: Option<&str>,
    protocol_version: &str,
    body: Value,
) -> Result<(), AppError> {
    let mut request = notion_http_client()
        .post(MCP_SERVER_URL)
        .bearer_auth(access_token)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", protocol_version)
        .json(&body);
    if let Some(session_id) = session_id {
        request = request.header("mcp-session-id", session_id);
    }
    let response = request
        .send()
        .await
        .map_err(|_| network_error("Could not notify Notion hosted MCP initialization."))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|_| network_error("Could not read Notion hosted MCP notification response."))?;
    if status.is_success() {
        return Ok(());
    }
    Err(upstream_error(
        "notion_mcp_oauth_mcp_failed",
        status,
        &text,
        "Notion hosted MCP notification failed.",
    ))
}

fn reject_json_rpc_error(body: &Value) -> Result<(), AppError> {
    let Some(error) = body.get("error") else {
        return Ok(());
    };
    let code = error
        .get("code")
        .and_then(Value::as_i64)
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    Err(AppError::new(
        "notion_mcp_oauth_json_rpc_error",
        format!("Notion hosted MCP returned a JSON-RPC error ({code})."),
    ))
}

fn parse_mcp_response_body(content_type: &str, text: &str) -> Result<Value, AppError> {
    if content_type.contains("text/event-stream") {
        for line in text.lines() {
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            return serde_json::from_str::<Value>(data).map_err(|_| {
                AppError::new(
                    "notion_mcp_oauth_invalid_response",
                    "Notion hosted MCP returned an invalid event-stream payload.",
                )
            });
        }
        return Err(AppError::new(
            "notion_mcp_oauth_invalid_response",
            "Notion hosted MCP returned an empty event-stream response.",
        ));
    }
    serde_json::from_str::<Value>(text).map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_invalid_response",
            "Notion hosted MCP returned an invalid JSON response.",
        )
    })
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
    store_platform_token(json).await
}

async fn load_token() -> Result<Option<StoredNotionMcpOAuth>, AppError> {
    load_platform_token().await
}

async fn delete_token() -> Result<(), AppError> {
    delete_platform_token().await
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn store_platform_token(json: String) -> Result<(), AppError> {
    let service = keychain_service().to_string();
    tokio::task::spawn_blocking(move || {
        keyring::Entry::new(&service, ACCOUNT_ID).and_then(|entry| entry.set_password(&json))
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
async fn store_platform_token(_json: String) -> Result<(), AppError> {
    Err(secure_storage_unavailable())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn load_platform_token() -> Result<Option<StoredNotionMcpOAuth>, AppError> {
    let service = keychain_service().to_string();
    let raw = tokio::task::spawn_blocking(move || {
        match keyring::Entry::new(&service, ACCOUNT_ID).and_then(|entry| entry.get_password()) {
            Ok(raw) => Ok(Some(raw)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(AppError::new(
                "notion_mcp_oauth_keychain_read_failed",
                "Could not read Notion hosted MCP OAuth material from Keychain.",
            )),
        }
    })
    .await
    .map_err(|_| {
        AppError::new(
            "notion_mcp_oauth_keychain_read_failed",
            "Keychain read task failed.",
        )
    })??;
    let Some(raw) = raw else {
        return Ok(None);
    };
    serde_json::from_str::<StoredNotionMcpOAuth>(&raw)
        .map(Some)
        .map_err(|_| {
            AppError::new(
                "notion_mcp_oauth_keychain_read_failed",
                "Stored Notion hosted MCP OAuth material could not be parsed.",
            )
        })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn load_platform_token() -> Result<Option<StoredNotionMcpOAuth>, AppError> {
    Err(secure_storage_unavailable())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn delete_platform_token() -> Result<(), AppError> {
    let service = keychain_service().to_string();
    tokio::task::spawn_blocking(move || {
        match keyring::Entry::new(&service, ACCOUNT_ID).and_then(|entry| entry.delete_credential())
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(AppError::new(
                "notion_mcp_oauth_keychain_delete_failed",
                "Could not delete Notion hosted MCP OAuth material from Keychain.",
            )),
        }
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
async fn delete_platform_token() -> Result<(), AppError> {
    Err(secure_storage_unavailable())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn secure_storage_unavailable() -> AppError {
    AppError::new(
        "notion_mcp_oauth_keychain_unavailable",
        "The Notion hosted MCP OAuth spike requires Keychain-backed storage.",
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
        "Connect the Notion hosted MCP OAuth spike before listing tools.",
    )
}

fn network_error(message: &str) -> AppError {
    AppError::new("notion_mcp_oauth_network", message)
}

fn upstream_error(code: &'static str, status: StatusCode, body: &str, message: &str) -> AppError {
    let error_code = serde_json::from_str::<OAuthErrorBody>(body)
        .ok()
        .and_then(|body| body.error);
    tracing::warn!(status = status.as_u16(), error_code = ?error_code, "notion hosted mcp oauth upstream request failed");
    let suffix = error_code
        .filter(|value| !value.is_empty())
        .map(|value| format!(" ({value})"))
        .unwrap_or_default();
    AppError::new(code, format!("{message}{suffix}"))
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
