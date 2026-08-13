use super::{ApiError, AppState};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use reqwest::Url;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

const DEFAULT_SAG_URL: &str = "http://127.0.0.1:8765";
const MAX_SAG_UPLOAD_BYTES: usize = 100 * 1024 * 1024;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/library/sag/status", get(get_sag_status))
        .route("/api/library/sag/sources", get(list_sag_sources))
        .route("/api/library/sag/search", post(search_sag))
        .route(
            "/api/library/sag/ingestions/upload",
            post(upload_sag_source).layer(DefaultBodyLimit::max(MAX_SAG_UPLOAD_BYTES)),
        )
        .route("/api/library/sag/ingestions/text", post(ingest_sag_text))
}

#[derive(Clone)]
pub(crate) struct SagLibraryGateway {
    base_url: Url,
    client: reqwest::Client,
}

impl SagLibraryGateway {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let configured = std::env::var("OPENTOPIA_SAG_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SAG_URL.to_string());
        Self::new(&configured)
    }

    fn new(base_url: &str) -> anyhow::Result<Self> {
        let mut parsed = Url::parse(base_url.trim())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            anyhow::bail!("OPENTOPIA_SAG_URL must use http or https");
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

    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self::new(DEFAULT_SAG_URL).expect("default SAG URL must be valid")
    }

    fn endpoint(&self, path: &str) -> Result<Url, ApiError> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .map_err(|error| ApiError::internal(format!("invalid SAG endpoint: {error}")))
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let response = self
            .client
            .get(self.endpoint(path)?)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(sag_transport_error)?;
        parse_sag_response(response).await
    }

    async fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
        request_timeout: Duration,
    ) -> Result<T, ApiError> {
        let response = self
            .client
            .post(self.endpoint(path)?)
            .json(body)
            .timeout(request_timeout)
            .send()
            .await
            .map_err(sag_transport_error)?;
        parse_sag_response(response).await
    }

    async fn post_multipart<T: DeserializeOwned>(
        &self,
        content_type: &str,
        body: Bytes,
    ) -> Result<T, ApiError> {
        let response = self
            .client
            .post(self.endpoint("api/ingestions/upload")?)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body)
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .map_err(sag_transport_error)?;
        parse_sag_response(response).await
    }

    fn display_url(&self) -> String {
        self.base_url.as_str().trim_end_matches('/').to_string()
    }
}

async fn get_sag_status(
    State(state): State<AppState>,
) -> Result<Json<SagConnectionView>, ApiError> {
    let status = state.sag_library.get::<SagStatus>("api/status").await?;
    Ok(Json(SagConnectionView {
        provider: "SAG",
        endpoint: state.sag_library.display_url(),
        status,
    }))
}

async fn list_sag_sources(
    State(state): State<AppState>,
) -> Result<Json<Vec<SagSourceView>>, ApiError> {
    Ok(Json(
        state
            .sag_library
            .get::<Vec<SagSourceView>>("api/sources")
            .await?,
    ))
}

async fn search_sag(
    State(state): State<AppState>,
    Json(request): Json<SagSearchRequest>,
) -> Result<Json<SagSearchResponse>, ApiError> {
    request.validate()?;
    let upstream = json!({
        "query": request.query.trim(),
        "purpose": request.purpose,
        "top_k": request.top_k,
        "maximum_tokens": request.maximum_tokens,
        "use_deepseek": request.use_deepseek,
        "subject_refs": request.subject_refs,
        "namespaces": request.namespaces,
    });
    Ok(Json(
        state
            .sag_library
            .post_json("api/search", &upstream, Duration::from_secs(180))
            .await?,
    ))
}

async fn upload_sag_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<SagIngestionResult>), ApiError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.starts_with("multipart/form-data;"))
        .ok_or_else(|| ApiError::bad_request("SAG upload requires multipart/form-data"))?;
    let result = state
        .sag_library
        .post_multipart::<SagIngestionResult>(content_type, body)
        .await?;
    Ok((StatusCode::CREATED, Json(result)))
}

async fn ingest_sag_text(
    State(state): State<AppState>,
    Json(request): Json<SagTextIngestionRequest>,
) -> Result<(StatusCode, Json<SagIngestionResult>), ApiError> {
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
        .sag_library
        .post_json("api/ingestions/text", &upstream, Duration::from_secs(300))
        .await?;
    Ok((StatusCode::CREATED, Json(result)))
}

fn sag_transport_error(error: reqwest::Error) -> ApiError {
    if error.is_timeout() {
        return ApiError::gateway_timeout("SAG 服务响应超时，请检查服务状态或稍后重试。");
    }
    tracing::warn!(error = %error, "SAG transport request failed");
    ApiError::bad_gateway(
        "SAG 服务尚未就绪。桌面端会尝试自动启动；也可以配置 OPENTOPIA_SAG_URL 连接外部服务。",
    )
}

async fn parse_sag_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, ApiError> {
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|error| ApiError::bad_gateway(format!("读取 SAG 响应失败：{error}")))?;
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
                format!("SAG 服务返回 {mapped}")
            } else {
                message
            },
        });
    }
    serde_json::from_slice(&body)
        .map_err(|error| ApiError::bad_gateway(format!("SAG 响应格式无效：{error}")))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SagConnectionView {
    provider: &'static str,
    endpoint: String,
    status: SagStatus,
}

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SagSearchRequest {
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
}

impl SagSearchRequest {
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct SagIngestionResult {
    job_id: String,
    status: String,
    asset_id: String,
    version_id: String,
    #[serde(default)]
    previous_version_id: Option<String>,
    version_number: usize,
    source_id: String,
    content_hash: String,
    namespace: String,
    title: String,
    stored_path: String,
    index_version: String,
    pipeline_signature: String,
    #[serde(default)]
    reused_projection: bool,
    #[serde(default)]
    evidence_units: usize,
    #[serde(default)]
    events: usize,
    #[serde(default)]
    entities: usize,
    #[serde(default)]
    llm_requests: usize,
    created_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct SagSearchResponse {
    pack: SagContextPack,
    diagnostics: SagSearchDiagnostics,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct SagContextPack {
    #[serde(default)]
    pack_id: Option<String>,
    status: String,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    query: Option<String>,
    plan: SagRetrievalPlan,
    #[serde(default)]
    coverage: Vec<SagNeedCoverage>,
    #[serde(default)]
    index_version: Option<String>,
    #[serde(default)]
    retrieval_engine: Option<String>,
    #[serde(default)]
    items: Vec<SagContextPackItem>,
    #[serde(default)]
    excluded_items: Vec<Value>,
    #[serde(default)]
    estimated_tokens: usize,
    maximum_tokens: usize,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct SagRetrievalPlan {
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    original_query: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    planner: String,
    #[serde(default)]
    needs: Vec<SagEvidenceNeed>,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct SagEvidenceNeed {
    need_id: String,
    description: String,
    query: String,
    #[serde(default)]
    facets: Vec<String>,
    #[serde(default)]
    subject_refs: Vec<String>,
    #[serde(default)]
    time_mode: Option<String>,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    weight: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct SagNeedCoverage {
    need_id: String,
    #[serde(default)]
    required: bool,
    status: String,
    #[serde(default)]
    selected_event_ids: Vec<String>,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct SagContextPackItem {
    event_id: String,
    evidence_id: String,
    content: String,
    event_summary: String,
    source_path: String,
    title: String,
    #[serde(default)]
    section_path: Vec<String>,
    #[serde(default)]
    anchors: Vec<String>,
    score: f64,
    selection_reason: String,
    #[serde(default)]
    matched_need_ids: Vec<String>,
    #[serde(default)]
    estimated_tokens: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct SagSearchDiagnostics {
    #[serde(default)]
    elapsed_seconds: f64,
    #[serde(default)]
    route_candidates: BTreeMap<String, usize>,
    #[serde(default)]
    llm_requests: usize,
    #[serde(default)]
    embedding_backend: Option<String>,
    #[serde(default)]
    deepseek_enabled: bool,
    #[serde(default)]
    agent_loop_integration: bool,
    #[serde(default)]
    prompt_injection: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_sag_endpoint_without_losing_a_path_prefix() {
        let gateway = SagLibraryGateway::new("https://memory.example.test/sag").unwrap();
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
    fn rejects_search_requests_outside_the_sag_contract() {
        let request = SagSearchRequest {
            query: "   ".to_string(),
            purpose: default_search_purpose(),
            top_k: 12,
            maximum_tokens: 5000,
            use_deepseek: true,
            subject_refs: vec![],
            namespaces: vec![],
        };
        assert!(request.validate().is_err());
    }
}
