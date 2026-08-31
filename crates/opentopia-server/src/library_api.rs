use super::{ApiError, AppState};
use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use opentopia_core::{
    AgentKnowledgeBindingV1, KnowledgeLibraryProviderV1, ModelContentPart, Tool, ToolCall,
    ToolExecutionPolicy, ToolInvocationContext, ToolResult,
};
use reqwest::Url;
use schemars::JsonSchema;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_SAG_URL: &str = "http://127.0.0.1:8765";
const DEFAULT_GRAPH_RAG_URL: &str = "http://127.0.0.1:8000";
const DEFAULT_GRAPH_RAG_DEV_ROLES: &str =
    "engineering,operations,finance,knowledge_admin,security_auditor,restricted";
const MAX_LIBRARY_UPLOAD_BYTES: usize = 100 * 1024 * 1024;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/library/providers", get(list_library_providers))
        .route("/api/library/:provider/status", get(get_library_status))
        .route("/api/library/:provider/sources", get(list_library_sources))
        .route("/api/library/:provider/search", post(search_library))
        .route(
            "/api/library/:provider/ingestions/upload",
            post(upload_library_source).layer(DefaultBodyLimit::max(MAX_LIBRARY_UPLOAD_BYTES)),
        )
        .route(
            "/api/library/:provider/ingestions/text",
            post(ingest_library_text),
        )
}

#[derive(Clone)]
struct LibraryHttpTransport {
    base_url: Url,
    client: reqwest::Client,
}

impl LibraryHttpTransport {
    fn new(base_url: &str) -> anyhow::Result<Self> {
        let mut parsed = Url::parse(base_url.trim())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            anyhow::bail!("library provider URL must use http or https");
        }
        parsed.set_query(None);
        parsed.set_fragment(None);
        if !parsed.path().ends_with('/') {
            parsed.set_path(&format!("{}/", parsed.path()));
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .build()?;
        Ok(Self {
            base_url: parsed,
            client,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url, ApiError> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .map_err(|error| ApiError::internal(format!("invalid provider endpoint: {error}")))
    }

    async fn get<T: DeserializeOwned>(
        &self,
        provider: &str,
        path: &str,
        bearer: Option<&str>,
    ) -> Result<T, ApiError> {
        let mut request = self.client.get(self.endpoint(path)?);
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        let response = request
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|error| provider_transport_error(provider, error))?;
        parse_provider_response(provider, response).await
    }

    async fn get_query<T: DeserializeOwned, Q: Serialize + ?Sized>(
        &self,
        provider: &str,
        path: &str,
        query: &Q,
        bearer: Option<&str>,
    ) -> Result<T, ApiError> {
        let mut request = self.client.get(self.endpoint(path)?).query(query);
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        let response = request
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|error| provider_transport_error(provider, error))?;
        parse_provider_response(provider, response).await
    }

    async fn post_json<T: DeserializeOwned>(
        &self,
        provider: &str,
        path: &str,
        body: &Value,
        bearer: Option<&str>,
        request_timeout: Duration,
    ) -> Result<T, ApiError> {
        let mut request = self.client.post(self.endpoint(path)?).json(body);
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        let response = request
            .timeout(request_timeout)
            .send()
            .await
            .map_err(|error| provider_transport_error(provider, error))?;
        parse_provider_response(provider, response).await
    }

    async fn post_multipart<T: DeserializeOwned>(
        &self,
        provider: &str,
        path: &str,
        content_type: &str,
        body: Bytes,
        bearer: Option<&str>,
    ) -> Result<T, ApiError> {
        let mut request = self
            .client
            .post(self.endpoint(path)?)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body);
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        let response = request
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .map_err(|error| provider_transport_error(provider, error))?;
        parse_provider_response(provider, response).await
    }

    fn display_url(&self) -> String {
        self.base_url.as_str().trim_end_matches('/').to_string()
    }
}

#[derive(Clone)]
pub(crate) struct SagLibraryGateway {
    transport: LibraryHttpTransport,
}

impl SagLibraryGateway {
    fn from_env() -> anyhow::Result<Self> {
        let configured = configured_url("OPENTOPIA_SAG_URL", DEFAULT_SAG_URL);
        Ok(Self {
            transport: LibraryHttpTransport::new(&configured)?,
        })
    }
}

#[derive(Clone)]
struct GraphRagLibraryGateway {
    transport: LibraryHttpTransport,
    configured_token: Option<String>,
    roles: Vec<String>,
    tenant_id: String,
}

impl GraphRagLibraryGateway {
    fn from_env() -> anyhow::Result<Self> {
        let configured = configured_url("OPENTOPIA_GRAPH_RAG_URL", DEFAULT_GRAPH_RAG_URL);
        let configured_token = std::env::var("OPENTOPIA_GRAPH_RAG_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let roles = std::env::var("OPENTOPIA_GRAPH_RAG_ROLES")
            .unwrap_or_else(|_| DEFAULT_GRAPH_RAG_DEV_ROLES.to_string())
            .split(',')
            .map(str::trim)
            .filter(|role| !role.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let tenant_id = std::env::var("OPENTOPIA_GRAPH_RAG_TENANT")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "demo".to_string());
        if roles.is_empty() {
            anyhow::bail!("OPENTOPIA_GRAPH_RAG_ROLES must contain at least one role");
        }
        Ok(Self {
            transport: LibraryHttpTransport::new(&configured)?,
            configured_token,
            roles,
            tenant_id,
        })
    }

    async fn access_token(&self) -> Result<String, ApiError> {
        if let Some(token) = &self.configured_token {
            return Ok(token.clone());
        }
        let response = self
            .transport
            .post_json::<GraphTokenResponse>(
                "Graph RAG",
                "dev/token",
                &json!({
                    "subject": "opentopia-library-review",
                    "roles": self.roles,
                    "tenant_id": self.tenant_id,
                }),
                None,
                Duration::from_secs(15),
            )
            .await
            .map_err(|error| {
                ApiError::bad_gateway(format!(
                    "Graph RAG 身份认证失败。生产环境请配置 OPENTOPIA_GRAPH_RAG_TOKEN；详情：{}",
                    error.message
                ))
            })?;
        Ok(response.access_token)
    }
}

#[derive(Clone)]
pub(crate) struct LibraryProviderRegistry {
    sag: SagLibraryGateway,
    graph_rag: GraphRagLibraryGateway,
}

impl LibraryProviderRegistry {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            sag: SagLibraryGateway::from_env()?,
            graph_rag: GraphRagLibraryGateway::from_env()?,
        })
    }

    async fn search_context_pack(
        &self,
        provider: LibraryProviderId,
        request: LibrarySearchRequest,
    ) -> Result<Value, ApiError> {
        request.validate()?;
        let value = match provider {
            LibraryProviderId::Sag => {
                let upstream = json!({
                    "query": request.query.trim(),
                    "purpose": request.purpose,
                    "top_k": request.top_k,
                    "maximum_tokens": request.maximum_tokens,
                    "use_deepseek": request.use_deepseek,
                    "subject_refs": request.subject_refs,
                    "namespaces": request.namespaces,
                });
                self.sag
                    .transport
                    .post_json::<Value>(
                        "SAG",
                        "api/search",
                        &upstream,
                        None,
                        Duration::from_secs(180),
                    )
                    .await?
            }
            LibraryProviderId::GraphRag => {
                let token = self.graph_rag.access_token().await?;
                let upstream = json!({
                    "query": request.query.trim(),
                    "top_k": request.top_k,
                    "maximum_tokens": request.maximum_tokens,
                    "retrieval_mode": request.retrieval_mode,
                });
                self.graph_rag
                    .transport
                    .post_json::<Value>(
                        "Graph RAG",
                        "v1/context-packs",
                        &upstream,
                        Some(&token),
                        Duration::from_secs(180),
                    )
                    .await?
            }
        };
        Ok(camelize_value(value))
    }

    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self {
            sag: SagLibraryGateway {
                transport: LibraryHttpTransport::new(DEFAULT_SAG_URL).unwrap(),
            },
            graph_rag: GraphRagLibraryGateway {
                transport: LibraryHttpTransport::new(DEFAULT_GRAPH_RAG_URL).unwrap(),
                configured_token: Some("test-token".to_string()),
                roles: vec!["engineering".to_string()],
                tenant_id: "demo".to_string(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum LibraryProviderId {
    Sag,
    GraphRag,
}

impl LibraryProviderId {
    pub(crate) fn parse(value: &str) -> Result<Self, ApiError> {
        match value {
            "sag" => Ok(Self::Sag),
            "graph-rag" | "graph_rag" => Ok(Self::GraphRag),
            _ => Err(ApiError::not_found(format!("未知的资料库后端：{value}"))),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Sag => "sag",
            Self::GraphRag => "graph-rag",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Sag => "SAG",
            Self::GraphRag => "Graph RAG",
        }
    }
}

impl From<KnowledgeLibraryProviderV1> for LibraryProviderId {
    fn from(value: KnowledgeLibraryProviderV1) -> Self {
        match value {
            KnowledgeLibraryProviderV1::Sag => Self::Sag,
            KnowledgeLibraryProviderV1::GraphRag => Self::GraphRag,
        }
    }
}

#[derive(Clone)]
pub(crate) struct LibrarySearchTool {
    providers: Arc<LibraryProviderRegistry>,
    binding_scope: LibraryBindingScope,
}

#[derive(Clone)]
enum LibraryBindingScope {
    Unrestricted(LibraryProviderId),
    Fixed(AgentKnowledgeBindingV1),
    RuntimeBound,
}

impl LibrarySearchTool {
    pub(crate) fn new(
        providers: Arc<LibraryProviderRegistry>,
        provider: LibraryProviderId,
    ) -> Self {
        Self {
            providers,
            binding_scope: LibraryBindingScope::Unrestricted(provider),
        }
    }

    pub(crate) fn bound(
        providers: Arc<LibraryProviderRegistry>,
        binding: AgentKnowledgeBindingV1,
    ) -> Self {
        Self {
            providers,
            binding_scope: LibraryBindingScope::Fixed(binding),
        }
    }

    pub(crate) fn runtime_bound(providers: Arc<LibraryProviderRegistry>) -> Self {
        Self {
            providers,
            binding_scope: LibraryBindingScope::RuntimeBound,
        }
    }

    fn resolved_binding(
        &self,
        context: &ToolInvocationContext,
    ) -> anyhow::Result<(LibraryProviderId, Vec<String>)> {
        let binding = match &self.binding_scope {
            LibraryBindingScope::Unrestricted(provider) => return Ok((*provider, Vec::new())),
            LibraryBindingScope::Fixed(binding) => binding,
            LibraryBindingScope::RuntimeBound => {
                context.knowledge_binding.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("library_search requires an Agent knowledge binding")
                })?
            }
        };
        let provider = LibraryProviderId::from(binding.provider);
        let namespaces = normalize_namespaces(binding.namespaces.iter().cloned());
        match provider {
            LibraryProviderId::Sag if namespaces.is_empty() => {
                anyhow::bail!("library_search requires at least one Agent-bound SAG namespace")
            }
            LibraryProviderId::GraphRag if !namespaces.is_empty() => {
                anyhow::bail!("Graph RAG Agent bindings cannot include SAG namespaces")
            }
            _ => Ok((provider, namespaces)),
        }
    }

    fn described_provider(&self) -> Option<LibraryProviderId> {
        match &self.binding_scope {
            LibraryBindingScope::Unrestricted(provider) => Some(*provider),
            LibraryBindingScope::Fixed(binding) => Some(binding.provider.into()),
            LibraryBindingScope::RuntimeBound => None,
        }
    }
}

fn normalize_namespaces(namespaces: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut namespaces = namespaces
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    namespaces.sort();
    namespaces.dedup();
    namespaces
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryToolInput {
    query: String,
    #[serde(default)]
    top_k: Option<usize>,
    #[serde(default)]
    maximum_tokens: Option<usize>,
    #[serde(default)]
    retrieval_mode: Option<String>,
}

#[async_trait]
impl Tool for LibrarySearchTool {
    fn name(&self) -> &str {
        "library_search"
    }

    fn description(&self) -> &str {
        match self.described_provider() {
            Some(LibraryProviderId::Sag) => {
                "Search the selected SAG knowledge and memory library when the user's request would benefit from stored personal, enterprise, event, entity, or temporal evidence. The tool returns evidence only; use its source titles and paths when explaining the answer. Do not call it for questions answerable from the current conversation alone."
            }
            Some(LibraryProviderId::GraphRag) => {
                "Search the selected Graph RAG knowledge library when the user's request would benefit from enterprise documents, entities, or relationship evidence. The tool returns evidence and graph paths only; use its source titles and anchors when explaining the answer. Do not call it for questions answerable from the current conversation alone."
            }
            None => "Search the knowledge library selected by the current Agent when the request would benefit from stored evidence. The Agent's immutable binding chooses the provider and scope; tool arguments cannot change either. Use returned source anchors when explaining the answer.",
        }
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 4000,
                    "description": "A self-contained search query for the selected library."
                },
                "topK": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 30,
                    "default": 12
                },
                "maximumTokens": {
                    "type": "integer",
                    "minimum": 256,
                    "maximum": 16000,
                    "default": 5000
                },
                "retrievalMode": {
                    "type": "string",
                    "enum": ["auto", "hybrid", "graph"],
                    "default": "auto",
                    "description": "Graph RAG routing preference; SAG safely ignores this preference."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn execution_policy(&self, _call: &ToolCall) -> ToolExecutionPolicy {
        let provider = self
            .described_provider()
            .map(LibraryProviderId::as_str)
            .unwrap_or("agent-bound");
        ToolExecutionPolicy::read_only(vec![format!("library:{provider}")])
    }

    async fn execute(
        &self,
        call: ToolCall,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let input: LibraryToolInput = serde_json::from_value(call.input)
            .map_err(|error| anyhow::anyhow!("invalid library_search arguments: {error}"))?;
        let (provider, namespaces) = self.resolved_binding(&ctx)?;
        let request = LibrarySearchRequest {
            query: input.query,
            purpose: default_search_purpose(),
            top_k: input.top_k.unwrap_or_else(default_top_k),
            maximum_tokens: input.maximum_tokens.unwrap_or_else(default_maximum_tokens),
            use_deepseek: true,
            subject_refs: Vec::new(),
            namespaces: namespaces.clone(),
            retrieval_mode: input.retrieval_mode.unwrap_or_else(default_retrieval_mode),
        };
        let value = self
            .providers
            .search_context_pack(provider, request)
            .await
            .map_err(|error| anyhow::anyhow!(error.message))?;
        let output = serde_json::to_string_pretty(&value)
            .map_err(|error| anyhow::anyhow!("failed to serialize library result: {error}"))?;
        Ok(ToolResult {
            call_id: call.id,
            output,
            content: vec![ModelContentPart::json(value.clone())],
            metadata: json!({
                "toolName": "library_search",
                "provider": provider.as_str(),
                "providerName": provider.display_name(),
                "success": true,
                "reviewOnly": true,
                "namespaces": namespaces,
                "promptInjection": false,
            }),
        })
    }
}

async fn list_library_providers() -> Json<Vec<LibraryProviderDescriptor>> {
    Json(vec![
        LibraryProviderDescriptor::sag(),
        LibraryProviderDescriptor::graph_rag(),
    ])
}

async fn get_library_status(
    Path(provider): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<LibraryProviderStatus>, ApiError> {
    let value = match LibraryProviderId::parse(&provider)? {
        LibraryProviderId::Sag => {
            let status = state
                .library_providers
                .sag
                .transport
                .get::<SagStatus>("SAG", "api/status", None)
                .await?;
            LibraryProviderStatus::Sag(SagConnectionView {
                provider: "SAG",
                endpoint: state.library_providers.sag.transport.display_url(),
                status,
            })
        }
        LibraryProviderId::GraphRag => {
            let status = state
                .library_providers
                .graph_rag
                .transport
                .get::<GraphRagStatus>("Graph RAG", "health", None)
                .await?;
            LibraryProviderStatus::GraphRag(GraphRagConnectionView {
                provider: "Graph RAG",
                endpoint: state.library_providers.graph_rag.transport.display_url(),
                status,
            })
        }
    };
    Ok(Json(value))
}

async fn list_library_sources(
    Path(provider): Path<String>,
    Query(query): Query<LibrarySourcesQuery>,
    State(state): State<AppState>,
) -> Result<Json<LibrarySourcePageView>, ApiError> {
    query.validate()?;
    let value = match LibraryProviderId::parse(&provider)? {
        LibraryProviderId::Sag => {
            let sources = state
                .library_providers
                .sag
                .transport
                .get::<Vec<SagSourceView>>("SAG", "api/sources", None)
                .await?;
            serde_json::to_value(paginate_sag_sources(sources, &query))
        }
        LibraryProviderId::GraphRag => {
            let token = state.library_providers.graph_rag.access_token().await?;
            let sources = state
                .library_providers
                .graph_rag
                .transport
                .get_query::<Value, _>("Graph RAG", "v1/knowledge/sources", &query, Some(&token))
                .await?;
            Ok(camelize_value(sources))
        }
    }
    .map_err(|error| ApiError::internal(format!("序列化资料来源失败：{error}")))?;
    let response = serde_json::from_value(camelize_value(value))
        .map_err(|error| ApiError::bad_gateway(format!("资料来源响应格式无效：{error}")))?;
    Ok(Json(response))
}

async fn search_library(
    Path(provider): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<LibrarySearchRequest>,
) -> Result<Json<LibrarySearchResponseView>, ApiError> {
    let provider = LibraryProviderId::parse(&provider)?;
    let value = state
        .library_providers
        .search_context_pack(provider, request)
        .await?;
    let response = serde_json::from_value(value)
        .map_err(|error| ApiError::bad_gateway(format!("资料检索响应格式无效：{error}")))?;
    Ok(Json(response))
}

async fn upload_library_source(
    Path(provider): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<LibraryIngestionResponseView>), ApiError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.starts_with("multipart/form-data;"))
        .ok_or_else(|| ApiError::bad_request("资料上传必须使用 multipart/form-data"))?;
    let value = match LibraryProviderId::parse(&provider)? {
        LibraryProviderId::Sag => {
            state
                .library_providers
                .sag
                .transport
                .post_multipart::<Value>("SAG", "api/ingestions/upload", content_type, body, None)
                .await?
        }
        LibraryProviderId::GraphRag => {
            let token = state.library_providers.graph_rag.access_token().await?;
            state
                .library_providers
                .graph_rag
                .transport
                .post_multipart::<Value>(
                    "Graph RAG",
                    "v1/documents/upload",
                    content_type,
                    body,
                    Some(&token),
                )
                .await?
        }
    };
    let response = serde_json::from_value(camelize_value(value))
        .map_err(|error| ApiError::bad_gateway(format!("资料导入响应格式无效：{error}")))?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn ingest_library_text(
    Path(provider): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<SagTextIngestionRequest>,
) -> Result<(StatusCode, Json<LibraryIngestionResponseView>), ApiError> {
    if !matches!(LibraryProviderId::parse(&provider)?, LibraryProviderId::Sag) {
        return Err(ApiError::bad_request(
            "Graph RAG 的文本导入请使用结构化文档接口或文件上传。",
        ));
    }
    request.validate()?;
    let upstream = json!({
        "content": request.content,
        "filename": request.filename,
        "asset_id": request.asset_id,
        "source_key": request.source_key,
        "namespace": request.namespace,
        "title": request.title,
        "metadata": request.metadata,
    });
    let result = state
        .library_providers
        .sag
        .transport
        .post_json::<Value>(
            "SAG",
            "api/ingestions/text",
            &upstream,
            None,
            Duration::from_secs(300),
        )
        .await?;
    let response = serde_json::from_value(camelize_value(result))
        .map_err(|error| ApiError::bad_gateway(format!("资料导入响应格式无效：{error}")))?;
    Ok((StatusCode::CREATED, Json(response)))
}

fn configured_url(name: &str, fallback: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn camelize_value(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (snake_to_camel(&key), camelize_value(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(camelize_value).collect()),
        other => other,
    }
}

fn snake_to_camel(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut uppercase_next = false;
    for character in value.chars() {
        if character == '_' {
            uppercase_next = true;
        } else if uppercase_next {
            result.extend(character.to_uppercase());
            uppercase_next = false;
        } else {
            result.push(character);
        }
    }
    result
}

fn provider_transport_error(provider: &str, error: reqwest::Error) -> ApiError {
    if error.is_timeout() {
        return ApiError::gateway_timeout(format!(
            "{provider} 服务响应超时，请检查服务状态或稍后重试。"
        ));
    }
    tracing::warn!(provider, error = %error, "library provider transport request failed");
    ApiError::bad_gateway(format!(
        "{provider} 服务尚未就绪。桌面端会尝试自动启动，也可以配置对应的外部服务地址。"
    ))
}

async fn parse_provider_response<T: DeserializeOwned>(
    provider: &str,
    response: reqwest::Response,
) -> Result<T, ApiError> {
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|error| ApiError::bad_gateway(format!("读取 {provider} 响应失败：{error}")))?;
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("detail")
                    .or_else(|| value.get("error"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| String::from_utf8_lossy(&body).trim().to_string());
        let mapped = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        return Err(ApiError {
            status: mapped,
            message: if message.is_empty() {
                format!("{provider} 服务返回 {mapped}")
            } else {
                message
            },
        });
    }
    serde_json::from_slice(&body)
        .map_err(|error| ApiError::bad_gateway(format!("{provider} 响应格式无效：{error}")))
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct LibraryProviderDescriptor {
    id: &'static str,
    name: &'static str,
    title: &'static str,
    description: &'static str,
    capabilities: LibraryProviderCapabilities,
}

impl LibraryProviderDescriptor {
    fn sag() -> Self {
        Self {
            id: "sag",
            name: "SAG",
            title: "SAG 记忆检索",
            description: "面向事件、实体与时序记忆的多路检索。",
            capabilities: LibraryProviderCapabilities {
                graph_paths: false,
                temporal_memory: true,
                incremental_upload: true,
                llm_planning: true,
            },
        }
    }

    fn graph_rag() -> Self {
        Self {
            id: "graph-rag",
            name: "Graph RAG",
            title: "Graph RAG 图谱检索",
            description: "从混合检索种子沿知识关系扩展，并展示可解释路径。",
            capabilities: LibraryProviderCapabilities {
                graph_paths: true,
                temporal_memory: false,
                incremental_upload: true,
                llm_planning: false,
            },
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct LibraryProviderCapabilities {
    graph_paths: bool,
    temporal_memory: bool,
    incremental_upload: bool,
    llm_planning: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SagConnectionView {
    provider: &'static str,
    endpoint: String,
    status: SagStatus,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct SagStatus {
    status: String,
    #[serde(default)]
    database: Option<String>,
    #[serde(default)]
    index_version: Option<String>,
    #[serde(default)]
    embedding_backend: Option<String>,
    #[serde(default)]
    embedding_dimensions: Option<usize>,
    #[serde(default)]
    stats: BTreeMap<String, usize>,
    #[serde(default)]
    integrity_check: Option<String>,
    #[serde(default)]
    model_loaded: bool,
    #[serde(default)]
    deepseek_configured: bool,
    #[serde(default)]
    agent_loop_integration: bool,
    #[serde(default)]
    prompt_injection: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct GraphRagConnectionView {
    provider: &'static str,
    endpoint: String,
    status: GraphRagStatus,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct GraphRagStatus {
    status: String,
    #[serde(default)]
    embedding_backend: Option<String>,
    #[serde(default)]
    embedding_dimensions: Option<usize>,
    #[serde(default)]
    reranker_backend: Option<String>,
    #[serde(default)]
    vector_backend: Option<String>,
    #[serde(default)]
    documents: usize,
    #[serde(default)]
    chunks: usize,
    #[serde(default)]
    relations: usize,
    #[serde(default)]
    index_version: Option<String>,
    #[serde(default)]
    graph_enabled: bool,
    #[serde(default)]
    stats: BTreeMap<String, usize>,
    #[serde(default)]
    agent_loop_integration: bool,
    #[serde(default)]
    prompt_injection: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub(super) enum LibraryProviderStatus {
    Sag(SagConnectionView),
    GraphRag(GraphRagConnectionView),
}

#[derive(Debug, Deserialize)]
struct GraphTokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct LibrarySourcesQuery {
    #[serde(default)]
    query: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_source_page_limit")]
    limit: usize,
}

impl LibrarySourcesQuery {
    fn validate(&self) -> Result<(), ApiError> {
        if self.query.chars().count() > 300 {
            return Err(ApiError::bad_request("资料筛选条件不能超过 300 个字符"));
        }
        if !(1..=200).contains(&self.limit) {
            return Err(ApiError::bad_request("limit 必须在 1 到 200 之间"));
        }
        Ok(())
    }
}

fn default_source_page_limit() -> usize {
    100
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LibrarySourcePage<T> {
    items: Vec<T>,
    total: usize,
    authorized_total: usize,
    index_total: usize,
    offset: usize,
    limit: usize,
    has_more: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct LibrarySourcePageView {
    items: Vec<Value>,
    total: usize,
    authorized_total: usize,
    index_total: usize,
    offset: usize,
    limit: usize,
    has_more: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct LibrarySearchResponseView {
    pack: Value,
    diagnostics: Value,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct LibraryIngestionResponseView {
    status: String,
    #[serde(flatten)]
    fields: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibrarySearchRequest {
    query: String,
    #[serde(default = "default_search_purpose")]
    purpose: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
    #[serde(default = "default_maximum_tokens")]
    maximum_tokens: usize,
    #[serde(default = "default_true")]
    use_deepseek: bool,
    #[serde(default)]
    subject_refs: Vec<String>,
    #[serde(default)]
    namespaces: Vec<String>,
    #[serde(default = "default_retrieval_mode")]
    retrieval_mode: String,
}

impl LibrarySearchRequest {
    fn validate(&self) -> Result<(), ApiError> {
        if self.query.trim().is_empty() || self.query.chars().count() > 4000 {
            return Err(ApiError::bad_request("检索问题必须为 1 到 4000 个字符"));
        }
        if !(1..=30).contains(&self.top_k) {
            return Err(ApiError::bad_request("topK 必须在 1 到 30 之间"));
        }
        if !(256..=16_000).contains(&self.maximum_tokens) {
            return Err(ApiError::bad_request(
                "maximumTokens 必须在 256 到 16000 之间",
            ));
        }
        if !matches!(self.retrieval_mode.as_str(), "auto" | "hybrid" | "graph") {
            return Err(ApiError::bad_request(
                "retrievalMode 必须是 auto、hybrid 或 graph",
            ));
        }
        Ok(())
    }
}

fn default_search_purpose() -> String {
    "evidence_review".to_string()
}

fn default_top_k() -> usize {
    12
}

fn default_maximum_tokens() -> usize {
    5000
}

fn default_true() -> bool {
    true
}

fn default_retrieval_mode() -> String {
    "auto".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SagTextIngestionRequest {
    content: String,
    #[serde(default = "default_import_filename")]
    filename: String,
    #[serde(default)]
    asset_id: Option<String>,
    #[serde(default)]
    source_key: Option<String>,
    #[serde(default = "default_namespace")]
    namespace: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, Value>,
}

impl SagTextIngestionRequest {
    fn validate(&self) -> Result<(), ApiError> {
        if self.content.trim().is_empty() {
            return Err(ApiError::bad_request("导入内容不能为空"));
        }
        if self.filename.trim().is_empty() || self.namespace.trim().is_empty() {
            return Err(ApiError::bad_request("filename 和 namespace 不能为空"));
        }
        Ok(())
    }
}

fn default_import_filename() -> String {
    "imported.md".to_string()
}

fn default_namespace() -> String {
    "enterprise_knowledge".to_string()
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct SagSourceView {
    asset_id: String,
    source_key: String,
    namespace: String,
    origin: String,
    version_id: String,
    version_number: usize,
    source_id: String,
    title: String,
    original_filename: String,
    content_hash: String,
    stored_path: String,
    #[serde(default)]
    metadata: BTreeMap<String, Value>,
    #[serde(default)]
    evidence_units: usize,
    #[serde(default)]
    events: usize,
    created_at: String,
}

fn paginate_sag_sources(
    mut sources: Vec<SagSourceView>,
    query: &LibrarySourcesQuery,
) -> LibrarySourcePage<SagSourceView> {
    let authorized_total = sources.len();
    let normalized = query.query.trim().to_lowercase();
    if !normalized.is_empty() {
        sources.retain(|source| {
            [
                source.title.as_str(),
                source.original_filename.as_str(),
                source.namespace.as_str(),
                source.source_key.as_str(),
                source.asset_id.as_str(),
            ]
            .iter()
            .any(|value| value.to_lowercase().contains(&normalized))
        });
    }
    sources.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.asset_id.cmp(&right.asset_id))
    });
    let total = sources.len();
    let items = sources
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect::<Vec<_>>();
    LibrarySourcePage {
        has_more: query.offset.saturating_add(items.len()) < total,
        items,
        total,
        authorized_total,
        index_total: authorized_total,
        offset: query.offset,
        limit: query.limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_provider_endpoint_without_losing_a_path_prefix() {
        let gateway = LibraryHttpTransport::new("https://memory.example.test/sag").unwrap();
        assert_eq!(
            gateway.endpoint("api/status").unwrap().as_str(),
            "https://memory.example.test/sag/api/status"
        );
    }

    #[test]
    fn maps_upstream_snake_case_to_desktop_camel_case() {
        let status: SagStatus = serde_json::from_value(json!({
            "status": "ready",
            "index_version": "sag-v1",
            "deepseek_configured": true,
            "agent_loop_integration": false,
            "prompt_injection": false,
            "stats": {"evidence_units": 18}
        }))
        .unwrap();
        let desktop = serde_json::to_value(status).unwrap();
        assert_eq!(desktop["indexVersion"], "sag-v1");
        assert_eq!(desktop["deepseekConfigured"], true);
        assert_eq!(desktop["agentLoopIntegration"], false);
    }

    #[test]
    fn rejects_search_requests_outside_the_provider_contract() {
        let request = LibrarySearchRequest {
            query: "   ".to_string(),
            purpose: default_search_purpose(),
            top_k: 12,
            maximum_tokens: 5000,
            use_deepseek: true,
            subject_refs: vec![],
            namespaces: vec![],
            retrieval_mode: default_retrieval_mode(),
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn exposes_both_library_provider_descriptors() {
        let providers = [
            LibraryProviderDescriptor::sag(),
            LibraryProviderDescriptor::graph_rag(),
        ];
        assert_eq!(providers[0].id, "sag");
        assert_eq!(providers[1].id, "graph-rag");
        assert!(providers[1].capabilities.graph_paths);
        assert!(providers[0].capabilities.temporal_memory);
    }

    #[test]
    fn runtime_bound_library_tool_uses_the_agent_provider_and_fails_closed() {
        let tool = LibrarySearchTool::runtime_bound(Arc::new(LibraryProviderRegistry::for_tests()));
        let authority = opentopia_core::ExecutionAuthority::new(
            std::env::current_dir().unwrap(),
            opentopia_core::PermissionMode::ReadOnly,
            opentopia_core::LocalSandboxConfig::default(),
            opentopia_core::CapabilityProjection::unrestricted(),
        )
        .unwrap();
        let mut context = authority.local_tool_context();
        assert!(tool.resolved_binding(&context).is_err());
        context.knowledge_binding = Some(AgentKnowledgeBindingV1 {
            provider: KnowledgeLibraryProviderV1::Sag,
            namespaces: BTreeSet::from(["opentopia.audit.credit-review.v1".to_string()]),
        });
        assert_eq!(
            tool.resolved_binding(&context).unwrap(),
            (
                LibraryProviderId::Sag,
                vec!["opentopia.audit.credit-review.v1".to_string()]
            )
        );
        context.knowledge_binding = Some(AgentKnowledgeBindingV1 {
            provider: KnowledgeLibraryProviderV1::GraphRag,
            namespaces: BTreeSet::new(),
        });
        assert_eq!(
            tool.resolved_binding(&context).unwrap(),
            (LibraryProviderId::GraphRag, Vec::new())
        );
    }

    #[test]
    fn registers_the_selected_library_as_a_standard_agent_tool() {
        let registry = Arc::new(LibraryProviderRegistry::for_tests());
        let mut agent = opentopia_core::AgentCore::default();
        agent.register_runtime_tool(Arc::new(LibrarySearchTool::new(
            registry,
            LibraryProviderId::GraphRag,
        )));

        let tool = agent
            .provider_tool_catalog()
            .into_iter()
            .find(|candidate| candidate.name == "library_search")
            .expect("library_search should be exposed to the model");
        assert!(tool.description.contains("Graph RAG"));
        assert_eq!(tool.input_schema["required"], json!(["query"]));
    }

    #[test]
    fn local_graph_rag_review_identity_can_inspect_restricted_test_documents() {
        let roles = DEFAULT_GRAPH_RAG_DEV_ROLES.split(',').collect::<Vec<_>>();

        assert!(roles.contains(&"restricted"));
        assert!(roles.contains(&"knowledge_admin"));
        assert!(roles.contains(&"security_auditor"));
    }

    #[test]
    fn recursively_normalizes_provider_payload_keys() {
        let normalized = camelize_value(json!({
            "index_version": "v1",
            "items": [{"graph_path": ["a", "b"]}]
        }));
        assert_eq!(normalized["indexVersion"], "v1");
        assert_eq!(normalized["items"][0]["graphPath"][1], "b");
    }

    #[test]
    fn paginates_and_filters_sag_sources_without_unbounded_payloads() {
        let source = |asset_id: &str, title: &str| SagSourceView {
            asset_id: asset_id.to_string(),
            source_key: format!("source-{asset_id}"),
            namespace: "knowledge".to_string(),
            origin: "upload".to_string(),
            version_id: format!("version-{asset_id}"),
            version_number: 1,
            source_id: format!("document-{asset_id}"),
            title: title.to_string(),
            original_filename: format!("{title}.docx"),
            content_hash: "hash".to_string(),
            stored_path: "stored.docx".to_string(),
            metadata: BTreeMap::new(),
            evidence_units: 1,
            events: 1,
            created_at: "2026-08-15T00:00:00Z".to_string(),
        };
        let query = LibrarySourcesQuery {
            query: "发布".to_string(),
            offset: 1,
            limit: 1,
        };

        let page = paginate_sag_sources(
            vec![
                source("3", "发布规范 C"),
                source("1", "发布规范 A"),
                source("2", "其他资料"),
                source("4", "发布规范 B"),
            ],
            &query,
        );

        assert_eq!(page.index_total, 4);
        assert_eq!(page.total, 3);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].title, "发布规范 B");
        assert!(page.has_more);
    }
}
