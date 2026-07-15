//! Dev-only Notion connector prototype for validating ADR 0024.
//!
//! This module is intentionally not a production connector. It exercises the
//! load-bearing seams from the ADR: Notion token custody in a dedicated
//! Keychain service, direct device -> Notion REST calls, selected-root boundary
//! checks, and the existing loopback provider-proxy bearer-token path. It must
//! not use the debug plaintext token-store fallback.

use crate::domain::types::AppError;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock},
    time::Duration,
};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const KEYCHAIN_SERVICE: &str = "co.opensoftware.june.notion";
const DEV_KEYCHAIN_SERVICE: &str = "co.opensoftware.june-dev.notion";
const ACCOUNT_ID: &str = "notion-prototype";
const NOTION_BASE_URL: &str = "https://api.notion.com";
const NOTION_VERSION: &str = "2026-03-11";
const DEV_PLAINTEXT_TOKEN_STORE_ENV: &str = "OS_JUNE_DEV_PLAINTEXT_TOKEN_STORE";
const MAX_SEARCH_RESULTS: u8 = 20;
const MAX_ROOTS: usize = 25;
const MAX_ANCESTRY_DEPTH: usize = 20;
const MAX_ANCESTRY_CALLS: usize = 25;
const PAGE_CONTENT_BLOCK_LIMIT: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotionPrototypeTokenType {
    InternalConnection,
    PersonalAccessToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotionObjectKind {
    Page,
    Block,
    DataSource,
    Database,
}

impl NotionObjectKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Block => "block",
            Self::DataSource => "data_source",
            Self::Database => "database",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedRoot {
    pub kind: NotionObjectKind,
    pub id: String,
}

#[derive(Debug, Clone)]
struct PrototypeAccount {
    account_id: String,
    token_type: NotionPrototypeTokenType,
    selected_roots: Vec<SelectedRoot>,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
struct StoredNotionPrototypeToken {
    token: String,
    #[zeroize(skip)]
    token_type: NotionPrototypeTokenType,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionPrototypeStatus {
    pub connected: bool,
    pub account_id: Option<String>,
    pub token_type: Option<NotionPrototypeTokenType>,
    pub selected_roots: Vec<SelectedRoot>,
    pub keychain_service: &'static str,
    pub plaintext_fallback_refused: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionPrototypeConnection {
    pub account_id: String,
    pub token_type: NotionPrototypeTokenType,
    pub bot_id: Option<String>,
    pub workspace_id: Option<String>,
    pub owner_type: Option<String>,
    pub selected_roots: Vec<SelectedRoot>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionPrototypeSearchResult {
    pub kind: NotionObjectKind,
    pub id: String,
    pub title_preview: Option<String>,
    pub url_present: bool,
    pub parent: Option<ParentRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentRef {
    pub kind: String,
    pub id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionPrototypePageSummary {
    pub id: String,
    pub title_preview: Option<String>,
    pub url_present: bool,
    pub parent: Option<ParentRef>,
    pub ancestry: BoundaryDecision,
    pub block_preview_count: usize,
    pub block_previews: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryDecision {
    pub allowed: bool,
    pub matched_root: Option<SelectedRoot>,
    pub calls_used: usize,
    pub depth_used: usize,
    pub reason: String,
}

#[derive(Debug, Clone)]
struct TraversalNode {
    kind: NotionObjectKind,
    id: String,
    parent: Parent,
}

#[derive(Debug, Clone)]
enum Parent {
    Object(NotionObjectKind, String),
    Workspace,
    Unsupported(String),
    Missing,
}

#[derive(Debug)]
pub enum NotionPrototypeApiError {
    Unauthorized,
    Permission { status: u16, code: Option<String> },
    RateLimited { status: u16, code: Option<String> },
    Api { status: u16, code: Option<String> },
    Network(String),
    InvalidResponse,
    InvalidInput(String),
}

impl From<NotionPrototypeApiError> for AppError {
    fn from(error: NotionPrototypeApiError) -> Self {
        match error {
            NotionPrototypeApiError::Unauthorized => AppError::new(
                "notion_prototype_unauthorized",
                "Notion rejected the prototype token.",
            ),
            NotionPrototypeApiError::Permission { status, code } => AppError::new(
                "notion_prototype_permission",
                format!(
                    "Notion returned a permission outcome ({status}{}).",
                    code_suffix(code.as_deref())
                ),
            ),
            NotionPrototypeApiError::RateLimited { status, code } => AppError::new(
                "notion_prototype_rate_limited",
                format!(
                    "Notion rate limited or overloaded the prototype request ({status}{}). Try again later.",
                    code_suffix(code.as_deref())
                ),
            ),
            NotionPrototypeApiError::Api { status, code } => AppError::new(
                "notion_prototype_api_error",
                format!(
                    "Notion API request failed ({status}{}).",
                    code_suffix(code.as_deref())
                ),
            ),
            NotionPrototypeApiError::Network(message) => {
                AppError::new("notion_prototype_network", message)
            }
            NotionPrototypeApiError::InvalidResponse => AppError::new(
                "notion_prototype_invalid_response",
                "Notion returned an unexpected response shape.",
            ),
            NotionPrototypeApiError::InvalidInput(message) => {
                AppError::new("notion_prototype_invalid_input", message)
            }
        }
    }
}

fn code_suffix(code: Option<&str>) -> String {
    code.filter(|value| !value.is_empty())
        .map(|value| format!(", code {value}"))
        .unwrap_or_default()
}

static REGISTRY: OnceLock<Mutex<HashMap<String, PrototypeAccount>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, PrototypeAccount>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub async fn connect(
    token: String,
    token_type: NotionPrototypeTokenType,
) -> Result<NotionPrototypeConnection, AppError> {
    refuse_plaintext_fallback()?;
    let token = Zeroizing::new(token.trim().to_string());
    if token.is_empty() {
        return Err(AppError::new(
            "notion_prototype_invalid_input",
            "Paste a Notion token before connecting.",
        ));
    }
    let client = NotionClient::new()?;
    let me = client.users_me(&token).await?;
    let stored = StoredNotionPrototypeToken {
        token: token.to_string(),
        token_type,
    };
    store_token(&stored).await?;
    let account = PrototypeAccount {
        account_id: ACCOUNT_ID.to_string(),
        token_type,
        selected_roots: Vec::new(),
    };
    registry()
        .lock()
        .map_err(|_| registry_error())?
        .insert(ACCOUNT_ID.to_string(), account);
    Ok(NotionPrototypeConnection {
        account_id: ACCOUNT_ID.to_string(),
        token_type,
        bot_id: me
            .get("bot")
            .and_then(|bot| bot.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        workspace_id: None,
        owner_type: me.get("type").and_then(Value::as_str).map(str::to_string),
        selected_roots: Vec::new(),
    })
}

pub async fn status() -> Result<NotionPrototypeStatus, AppError> {
    refuse_plaintext_fallback()?;
    let stored = load_token().await?;
    let selected_roots = registry()
        .lock()
        .map_err(|_| registry_error())?
        .get(ACCOUNT_ID)
        .map(|account| account.selected_roots.clone())
        .unwrap_or_default();
    Ok(NotionPrototypeStatus {
        connected: stored.is_some(),
        account_id: stored.as_ref().map(|_| ACCOUNT_ID.to_string()),
        token_type: stored.as_ref().map(|stored| stored.token_type),
        selected_roots,
        keychain_service: keychain_service(),
        plaintext_fallback_refused: plaintext_fallback_requested(),
    })
}

pub async fn disconnect() -> Result<(), AppError> {
    refuse_plaintext_fallback()?;
    delete_token().await?;
    registry()
        .lock()
        .map_err(|_| registry_error())?
        .remove(ACCOUNT_ID);
    Ok(())
}

pub async fn select_roots(roots: Vec<SelectedRoot>) -> Result<Vec<SelectedRoot>, AppError> {
    refuse_plaintext_fallback()?;
    if roots.len() > MAX_ROOTS {
        return Err(AppError::new(
            "notion_prototype_too_many_roots",
            format!("Select at most {MAX_ROOTS} prototype roots."),
        ));
    }
    let mut canonical = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        let id = canonical_id(&root.id)?;
        let root = SelectedRoot {
            kind: root.kind,
            id,
        };
        if seen.insert((root.kind, root.id.clone())) {
            canonical.push(root);
        }
    }
    ensure_account_loaded().await?;
    let mut guard = registry().lock().map_err(|_| registry_error())?;
    let account = guard
        .get_mut(ACCOUNT_ID)
        .ok_or_else(missing_account_error)?;
    account.selected_roots = canonical.clone();
    Ok(canonical)
}

pub async fn search(
    query: Option<String>,
    max: Option<u8>,
) -> Result<Vec<NotionPrototypeSearchResult>, AppError> {
    let token = load_token_string().await?;
    let client = NotionClient::new()?;
    let max = max.unwrap_or(10).clamp(1, MAX_SEARCH_RESULTS);
    let results = client.search(&token, query.as_deref(), max).await?;
    Ok(results
        .into_iter()
        .filter_map(|value| search_result_from_value(&value))
        .collect())
}

pub async fn get_page_via_proxy(
    account_id: &str,
    page_id: &str,
) -> Result<NotionPrototypePageSummary, AppError> {
    refuse_plaintext_fallback()?;
    let account = ensure_account_loaded().await?;
    if account.account_id != account_id {
        return Err(AppError::new(
            "notion_prototype_account_mismatch",
            "The Notion prototype account id does not match the route account.",
        ));
    }
    let page_id = canonical_id(page_id)?;
    let token = load_token_string().await?;
    let client = NotionClient::new()?;
    let page = client.retrieve_page(&token, &page_id).await?;
    let page_node = node_from_page(&page, &page_id)?;
    let decision = verify_boundary(&client, &token, &account.selected_roots, page_node).await?;
    if !decision.allowed {
        return Err(AppError::new(
            "notion_prototype_boundary_denied",
            format!(
                "The requested page is outside the selected Notion roots: {}",
                decision.reason
            ),
        ));
    }
    let block_previews = client
        .retrieve_block_children_preview(&token, &page_id, PAGE_CONTENT_BLOCK_LIMIT)
        .await
        .unwrap_or_default();
    Ok(NotionPrototypePageSummary {
        id: page_id,
        title_preview: title_preview(&page),
        url_present: page.get("url").and_then(Value::as_str).is_some(),
        parent: parent_ref(page.get("parent")),
        ancestry: decision,
        block_preview_count: block_previews.len(),
        block_previews,
    })
}

async fn verify_boundary(
    client: &NotionClient,
    token: &str,
    roots: &[SelectedRoot],
    mut current: TraversalNode,
) -> Result<BoundaryDecision, AppError> {
    if roots.is_empty() {
        return Ok(BoundaryDecision {
            allowed: false,
            matched_root: None,
            calls_used: 0,
            depth_used: 0,
            reason: "no selected roots".to_string(),
        });
    }
    let root_set = roots
        .iter()
        .map(|root| ((root.kind, root.id.clone()), root.clone()))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut calls = 0;
    for depth in 0..=MAX_ANCESTRY_DEPTH {
        if let Some(root) = root_set.get(&(current.kind, current.id.clone())) {
            return Ok(BoundaryDecision {
                allowed: true,
                matched_root: Some(root.clone()),
                calls_used: calls,
                depth_used: depth,
                reason: "matched selected root".to_string(),
            });
        }
        if !seen.insert((current.kind, current.id.clone())) {
            return Ok(denied(calls, depth, "cycle detected"));
        }
        let Parent::Object(parent_kind, parent_id) = current.parent.clone() else {
            return Ok(match current.parent {
                Parent::Workspace => denied(calls, depth, "reached workspace before selected root"),
                Parent::Unsupported(kind) => {
                    denied(calls, depth, &format!("unsupported parent type {kind}"))
                }
                Parent::Missing => denied(calls, depth, "missing parent"),
                Parent::Object(_, _) => unreachable!(),
            });
        };
        if let Some(root) = root_set.get(&(parent_kind, parent_id.clone())) {
            return Ok(BoundaryDecision {
                allowed: true,
                matched_root: Some(root.clone()),
                calls_used: calls,
                depth_used: depth + 1,
                reason: "matched selected parent root".to_string(),
            });
        }
        if calls >= MAX_ANCESTRY_CALLS {
            return Ok(denied(calls, depth, "ancestry request budget exhausted"));
        }
        calls += 1;
        current = client.retrieve_node(token, parent_kind, &parent_id).await?;
    }
    Ok(denied(
        calls,
        MAX_ANCESTRY_DEPTH,
        "ancestry depth budget exhausted",
    ))
}

fn denied(calls: usize, depth: usize, reason: &str) -> BoundaryDecision {
    BoundaryDecision {
        allowed: false,
        matched_root: None,
        calls_used: calls,
        depth_used: depth,
        reason: reason.to_string(),
    }
}

#[derive(Clone)]
struct NotionClient {
    http: reqwest::Client,
}

impl NotionClient {
    fn new() -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("June Notion prototype/0.1")
            .build()
            .map_err(|error| AppError::new("notion_prototype_client_failed", error.to_string()))?;
        Ok(Self { http })
    }

    async fn users_me(&self, token: &str) -> Result<Value, NotionPrototypeApiError> {
        self.send(
            self.http
                .get(format!("{NOTION_BASE_URL}/v1/users/me"))
                .bearer_auth(token),
        )
        .await
    }

    async fn search(
        &self,
        token: &str,
        query: Option<&str>,
        max: u8,
    ) -> Result<Vec<Value>, NotionPrototypeApiError> {
        let mut body = serde_json::json!({ "page_size": max });
        if let Some(query) = query.map(str::trim).filter(|value| !value.is_empty()) {
            body["query"] = Value::String(query.to_string());
        }
        let value: Value = self
            .send(
                self.http
                    .post(format!("{NOTION_BASE_URL}/v1/search"))
                    .bearer_auth(token)
                    .json(&body),
            )
            .await?;
        Ok(value
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    async fn retrieve_page(&self, token: &str, id: &str) -> Result<Value, NotionPrototypeApiError> {
        self.send(
            self.http
                .get(format!("{NOTION_BASE_URL}/v1/pages/{id}"))
                .bearer_auth(token),
        )
        .await
    }

    async fn retrieve_block(
        &self,
        token: &str,
        id: &str,
    ) -> Result<Value, NotionPrototypeApiError> {
        self.send(
            self.http
                .get(format!("{NOTION_BASE_URL}/v1/blocks/{id}"))
                .bearer_auth(token),
        )
        .await
    }

    async fn retrieve_data_source(
        &self,
        token: &str,
        id: &str,
    ) -> Result<Value, NotionPrototypeApiError> {
        self.send(
            self.http
                .get(format!("{NOTION_BASE_URL}/v1/data_sources/{id}"))
                .bearer_auth(token),
        )
        .await
    }

    async fn retrieve_database(
        &self,
        token: &str,
        id: &str,
    ) -> Result<Value, NotionPrototypeApiError> {
        self.send(
            self.http
                .get(format!("{NOTION_BASE_URL}/v1/databases/{id}"))
                .bearer_auth(token),
        )
        .await
    }

    async fn retrieve_node(
        &self,
        token: &str,
        kind: NotionObjectKind,
        id: &str,
    ) -> Result<TraversalNode, AppError> {
        let value = match kind {
            NotionObjectKind::Page => self.retrieve_page(token, id).await?,
            NotionObjectKind::Block => self.retrieve_block(token, id).await?,
            NotionObjectKind::DataSource => self.retrieve_data_source(token, id).await?,
            NotionObjectKind::Database => self.retrieve_database(token, id).await?,
        };
        node_from_value(kind, &value, id)
    }

    async fn retrieve_block_children_preview(
        &self,
        token: &str,
        id: &str,
        limit: usize,
    ) -> Result<Vec<String>, NotionPrototypeApiError> {
        let value: Value = self
            .send(
                self.http
                    .get(format!(
                        "{NOTION_BASE_URL}/v1/blocks/{id}/children?page_size={limit}"
                    ))
                    .bearer_auth(token),
            )
            .await?;
        Ok(value
            .get("results")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(block_preview).collect())
            .unwrap_or_default())
    }

    async fn send<T: serde::de::DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<T, NotionPrototypeApiError> {
        let response = builder
            .header("Notion-Version", NOTION_VERSION)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    NotionPrototypeApiError::Network("Notion request timed out.".to_string())
                } else if error.is_redirect() {
                    NotionPrototypeApiError::Network("Notion redirect was refused.".to_string())
                } else {
                    NotionPrototypeApiError::Network(
                        "Notion request failed before a response was received.".to_string(),
                    )
                }
            })?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|_| NotionPrototypeApiError::InvalidResponse)?;
        if !status.is_success() {
            let code = notion_error_code(&bytes);
            return Err(classify_status(status, code));
        }
        serde_json::from_slice(&bytes).map_err(|_| NotionPrototypeApiError::InvalidResponse)
    }
}

fn classify_status(status: StatusCode, code: Option<String>) -> NotionPrototypeApiError {
    match status.as_u16() {
        401 => NotionPrototypeApiError::Unauthorized,
        403 | 404 => NotionPrototypeApiError::Permission {
            status: status.as_u16(),
            code,
        },
        429 | 529 => NotionPrototypeApiError::RateLimited {
            status: status.as_u16(),
            code,
        },
        _ => NotionPrototypeApiError::Api {
            status: status.as_u16(),
            code,
        },
    }
}

fn notion_error_code(bytes: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| {
            value
                .get("code")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .map(|code| {
            code.chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(80)
                .collect()
        })
}

fn node_from_page(value: &Value, fallback_id: &str) -> Result<TraversalNode, AppError> {
    node_from_value(NotionObjectKind::Page, value, fallback_id)
}

fn node_from_value(
    kind: NotionObjectKind,
    value: &Value,
    fallback_id: &str,
) -> Result<TraversalNode, AppError> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(canonical_id)
        .transpose()?
        .unwrap_or_else(|| fallback_id.to_string());
    Ok(TraversalNode {
        kind,
        id,
        parent: parent_from_value(value.get("parent")),
    })
}

fn parent_from_value(parent: Option<&Value>) -> Parent {
    let Some(parent) = parent else {
        return Parent::Missing;
    };
    let Some(parent_type) = parent.get("type").and_then(Value::as_str) else {
        return Parent::Missing;
    };
    match parent_type {
        "page_id" => parent_id(parent, "page_id")
            .map(|id| Parent::Object(NotionObjectKind::Page, id))
            .unwrap_or(Parent::Missing),
        "block_id" => parent_id(parent, "block_id")
            .map(|id| Parent::Object(NotionObjectKind::Block, id))
            .unwrap_or(Parent::Missing),
        "data_source_id" => parent_id(parent, "data_source_id")
            .map(|id| Parent::Object(NotionObjectKind::DataSource, id))
            .unwrap_or(Parent::Missing),
        "database_id" => parent_id(parent, "database_id")
            .map(|id| Parent::Object(NotionObjectKind::Database, id))
            .unwrap_or(Parent::Missing),
        "workspace" => Parent::Workspace,
        other => Parent::Unsupported(other.to_string()),
    }
}

fn parent_id(parent: &Value, key: &str) -> Option<String> {
    parent
        .get(key)
        .and_then(Value::as_str)
        .and_then(|id| canonical_id(id).ok())
}

fn parent_ref(parent: Option<&Value>) -> Option<ParentRef> {
    let parent = parent?;
    let kind = parent.get("type")?.as_str()?.to_string();
    let id = match kind.as_str() {
        "page_id" => parent_id(parent, "page_id"),
        "block_id" => parent_id(parent, "block_id"),
        "data_source_id" => parent_id(parent, "data_source_id"),
        "database_id" => parent_id(parent, "database_id"),
        _ => None,
    };
    Some(ParentRef { kind, id })
}

fn search_result_from_value(value: &Value) -> Option<NotionPrototypeSearchResult> {
    let object = value.get("object").and_then(Value::as_str)?;
    let kind = match object {
        "page" => NotionObjectKind::Page,
        "data_source" => NotionObjectKind::DataSource,
        "database" => NotionObjectKind::Database,
        "block" => NotionObjectKind::Block,
        _ => return None,
    };
    Some(NotionPrototypeSearchResult {
        kind,
        id: canonical_id(value.get("id")?.as_str()?).ok()?,
        title_preview: title_preview(value),
        url_present: value.get("url").and_then(Value::as_str).is_some(),
        parent: parent_ref(value.get("parent")),
    })
}

fn title_preview(value: &Value) -> Option<String> {
    if let Some(properties) = value.get("properties").and_then(Value::as_object) {
        for property in properties.values() {
            if property.get("type").and_then(Value::as_str) == Some("title") {
                return rich_text_plain(property.get("title"));
            }
        }
    }
    value
        .get("title")
        .and_then(|value| rich_text_plain(Some(value)))
        .or_else(|| {
            value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .map(|title| truncate_chars(&title.split_whitespace().collect::<Vec<_>>().join(" "), 120))
}

fn rich_text_plain(value: Option<&Value>) -> Option<String> {
    let text = value?
        .as_array()?
        .iter()
        .filter_map(|item| item.get("plain_text").and_then(Value::as_str))
        .collect::<String>();
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn block_preview(value: &Value) -> Option<String> {
    let block_type = value.get("type").and_then(Value::as_str)?;
    let container = value.get(block_type);
    let text = container
        .and_then(|container| container.get("rich_text"))
        .and_then(|value| rich_text_plain(Some(value)))
        .unwrap_or_else(|| format!("[{block_type}]"));
    Some(truncate_chars(
        &text.split_whitespace().collect::<Vec<_>>().join(" "),
        160,
    ))
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        let mut out = value.chars().take(max).collect::<String>();
        out.push_str("...");
        out
    }
}

fn canonical_id(id: &str) -> Result<String, AppError> {
    let compact = id.trim().trim_matches('"').replace('-', "");
    if compact.len() != 32 || !compact.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::new(
            "notion_prototype_invalid_id",
            "Notion ids must be UUIDs with or without hyphens.",
        ));
    }
    let uuid = Uuid::parse_str(&compact)
        .map_err(|_| AppError::new("notion_prototype_invalid_id", "Invalid Notion UUID."))?;
    Ok(uuid.hyphenated().to_string())
}

async fn ensure_account_loaded() -> Result<PrototypeAccount, AppError> {
    if let Some(account) = registry()
        .lock()
        .map_err(|_| registry_error())?
        .get(ACCOUNT_ID)
        .cloned()
    {
        return Ok(account);
    }
    let stored = load_token().await?.ok_or_else(missing_account_error)?;
    let account = PrototypeAccount {
        account_id: ACCOUNT_ID.to_string(),
        token_type: stored.token_type,
        selected_roots: Vec::new(),
    };
    registry()
        .lock()
        .map_err(|_| registry_error())?
        .insert(ACCOUNT_ID.to_string(), account.clone());
    Ok(account)
}

async fn load_token_string() -> Result<Zeroizing<String>, AppError> {
    refuse_plaintext_fallback()?;
    load_token()
        .await?
        .map(|stored| Zeroizing::new(stored.token.clone()))
        .ok_or_else(missing_account_error)
}

async fn store_token(tokens: &StoredNotionPrototypeToken) -> Result<(), AppError> {
    let json = serde_json::to_string(tokens).map_err(|_| {
        AppError::new(
            "notion_prototype_token_store_failed",
            "Could not encode Notion token.",
        )
    })?;
    store_platform_token(json).await
}

async fn load_token() -> Result<Option<StoredNotionPrototypeToken>, AppError> {
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
            "notion_prototype_keychain_write_failed",
            "Keychain write task failed.",
        )
    })?
    .map_err(|_| {
        AppError::new(
            "notion_prototype_keychain_write_failed",
            "Could not store the Notion prototype token in Keychain.",
        )
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn store_platform_token(_json: String) -> Result<(), AppError> {
    Err(secure_storage_unavailable())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn load_platform_token() -> Result<Option<StoredNotionPrototypeToken>, AppError> {
    let service = keychain_service().to_string();
    let raw = tokio::task::spawn_blocking(move || {
        match keyring::Entry::new(&service, ACCOUNT_ID).and_then(|entry| entry.get_password()) {
            Ok(raw) => Ok(Some(raw)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(AppError::new(
                "notion_prototype_keychain_read_failed",
                "Could not read the Notion prototype token from Keychain.",
            )),
        }
    })
    .await
    .map_err(|_| {
        AppError::new(
            "notion_prototype_keychain_read_failed",
            "Keychain read task failed.",
        )
    })??;
    let Some(raw) = raw else {
        return Ok(None);
    };
    serde_json::from_str::<StoredNotionPrototypeToken>(&raw)
        .map(Some)
        .map_err(|_| {
            AppError::new(
                "notion_prototype_keychain_read_failed",
                "Stored Notion prototype token could not be parsed.",
            )
        })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn load_platform_token() -> Result<Option<StoredNotionPrototypeToken>, AppError> {
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
                "notion_prototype_keychain_delete_failed",
                "Could not delete the Notion prototype token from Keychain.",
            )),
        }
    })
    .await
    .map_err(|_| {
        AppError::new(
            "notion_prototype_keychain_delete_failed",
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
        "notion_prototype_keychain_unavailable",
        "The Notion prototype requires Keychain-backed storage and refuses plaintext fallback.",
    )
}

fn keychain_service() -> &'static str {
    if cfg!(debug_assertions) {
        DEV_KEYCHAIN_SERVICE
    } else {
        KEYCHAIN_SERVICE
    }
}

fn plaintext_fallback_requested() -> bool {
    crate::os_accounts::load_local_env();
    super::env_truthy(DEV_PLAINTEXT_TOKEN_STORE_ENV)
}

fn refuse_plaintext_fallback() -> Result<(), AppError> {
    if plaintext_fallback_requested() {
        return Err(AppError::new(
            "notion_prototype_plaintext_store_refused",
            "The Notion prototype refuses OS_JUNE_DEV_PLAINTEXT_TOKEN_STORE; unset it so Keychain-only custody is tested.",
        ));
    }
    Ok(())
}

fn registry_error() -> AppError {
    AppError::new(
        "notion_prototype_registry_failed",
        "The Notion prototype registry is unavailable.",
    )
}

fn missing_account_error() -> AppError {
    AppError::new(
        "notion_prototype_not_connected",
        "Connect the Notion prototype before using this route.",
    )
}

fn connector_json<T: Serialize>(value: T) -> Result<Value, AppError> {
    serde_json::to_value(value).map_err(|_| {
        AppError::new(
            "notion_prototype_serialize_failed",
            "Could not serialize the Notion prototype response.",
        )
    })
}

pub async fn dispatch_proxy_route(
    path: &str,
    account_id: &str,
    body: &Value,
) -> Result<Value, AppError> {
    match path {
        "/v1/notion-prototype/get_page" => {
            let page_id = body
                .get("page_id")
                .or_else(|| body.get("pageId"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::new("notion_prototype_invalid_input", "page_id is required.")
                })?;
            connector_json(get_page_via_proxy(account_id, page_id).await?)
        }
        _ => Err(AppError::new(
            "connector_unknown_route",
            "Unknown Notion prototype connector tool.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_id_accepts_hyphenated_and_compact() {
        let hyphenated = "12345678-1234-1234-1234-123456789abc";
        let compact = "12345678123412341234123456789abc";
        assert_eq!(canonical_id(hyphenated).unwrap(), hyphenated);
        assert_eq!(canonical_id(compact).unwrap(), hyphenated);
    }

    #[test]
    fn parent_parser_handles_data_source_parent() {
        let value = serde_json::json!({
            "type": "data_source_id",
            "data_source_id": "12345678123412341234123456789abc"
        });
        match parent_from_value(Some(&value)) {
            Parent::Object(NotionObjectKind::DataSource, id) => {
                assert_eq!(id, "12345678-1234-1234-1234-123456789abc");
            }
            other => panic!("unexpected parent: {other:?}"),
        }
    }
}
