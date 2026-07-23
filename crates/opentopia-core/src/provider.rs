use crate::model::ModelContentPart;
use crate::model_context::{
    CompiledModelContext, ContextCacheScope, ContextItemKind, ContextRole, ModelContextItem,
};
use crate::settings::{PromptCachePolicy, ProviderHealthCheck, ProviderKind, ProviderSettings};
use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelConversationRole {
    System,
    User,
    Assistant,
}

/// Typed input content shared by user/history messages and tool results.
///
/// This alias leaves the model-layer representation as the single source of
/// truth while making the provider-facing API discoverable.
pub type ModelInputContent = ModelContentPart;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelConversationMessage {
    pub role: ModelConversationRole,
    /// Legacy text content. Non-empty `content_parts` are appended and sent as
    /// native content parts where the selected provider supports them.
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_parts: Vec<ModelInputContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequest {
    pub system_prompt: String,
    #[serde(default)]
    pub conversation: Vec<ModelConversationMessage>,
    pub user_message: String,
    /// Native user input carried alongside `user_message`. Keep the string for
    /// older callers and providers; adapters combine both fields in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_content: Vec<ModelInputContent>,
    #[serde(default)]
    pub tool_candidates: Vec<ProviderToolCandidate>,
    #[serde(default)]
    pub previous_tool_calls: Vec<ProviderToolCall>,
    #[serde(default)]
    pub tool_results: Vec<ProviderToolResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_items: Vec<ModelContextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_response_items: Vec<Value>,
    /// Continue a stored Responses API chain. The logical request still carries
    /// the complete replay context so the adapter can recover if this cursor is
    /// unknown or expired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    /// Branch-specific developer instructions are emitted after inherited
    /// conversation history so sibling agents retain an identical prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_developer_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_output_json_schema: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResponse {
    pub text: String,
    #[serde(default)]
    pub tool_calls: Vec<ProviderToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ModelUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_items: Vec<Value>,
    #[serde(default)]
    pub finish_reason: ModelFinishReason,
}

impl ModelResponse {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tool_calls: Vec::new(),
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        }
    }

    pub fn decision(&self) -> ModelDecision {
        if let Some(reason) = self.finish_reason.incomplete_reason() {
            return ModelDecision::Incomplete(reason);
        }
        if !self.tool_calls.is_empty() {
            return ModelDecision::Act(self.tool_calls.clone());
        }
        if self.finish_reason == ModelFinishReason::ToolCalls {
            return ModelDecision::Incomplete(IncompleteReason::ProviderProtocol(
                "provider reported tool_calls but returned no tool call".to_string(),
            ));
        }
        let text = self.text.trim();
        if text.is_empty() {
            ModelDecision::Incomplete(IncompleteReason::EmptyResponse)
        } else {
            ModelDecision::Final(text.to_string())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "reason", rename_all = "snake_case")]
pub enum ModelFinishReason {
    Stop,
    ToolCalls,
    Completed,
    Length,
    ContentFilter,
    Incomplete(String),
    StreamInterrupted,
}

impl Default for ModelFinishReason {
    fn default() -> Self {
        Self::StreamInterrupted
    }
}

impl ModelFinishReason {
    fn incomplete_reason(&self) -> Option<IncompleteReason> {
        match self {
            Self::Stop | Self::ToolCalls | Self::Completed => None,
            Self::Length => Some(IncompleteReason::OutputTokenLimit),
            Self::ContentFilter => Some(IncompleteReason::ContentFilter),
            Self::Incomplete(reason) => Some(IncompleteReason::Provider(reason.clone())),
            Self::StreamInterrupted => Some(IncompleteReason::StreamInterrupted),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum IncompleteReason {
    OutputTokenLimit,
    ContentFilter,
    EmptyResponse,
    StreamInterrupted,
    Provider(String),
    ProviderProtocol(String),
}

impl std::fmt::Display for IncompleteReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutputTokenLimit => formatter.write_str("output token limit reached"),
            Self::ContentFilter => formatter.write_str("response stopped by content filter"),
            Self::EmptyResponse => {
                formatter.write_str("provider returned an empty assistant response")
            }
            Self::StreamInterrupted => {
                formatter.write_str("provider stream ended before a terminal event")
            }
            Self::Provider(reason) => write!(
                formatter,
                "provider reported an incomplete response: {reason}"
            ),
            Self::ProviderProtocol(reason) => {
                write!(formatter, "provider completion protocol error: {reason}")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "decision", content = "value", rename_all = "snake_case")]
pub enum ModelDecision {
    Act(Vec<ProviderToolCall>),
    Final(String),
    Incomplete(IncompleteReason),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ModelStreamDelta {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    ToolCall {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    Usage {
        usage: ModelUsage,
    },
}

pub type ModelStreamCallback<'a> = dyn FnMut(ModelStreamDelta) -> anyhow::Result<()> + Send + 'a;

#[derive(Debug, Clone)]
pub struct PreparedProviderRequest {
    pub request_id: Uuid,
    pub adapter: String,
    pub method: String,
    pub endpoint: String,
    pub body: Value,
    pub observation_body: Value,
    pub logical_request: ModelRequest,
}

#[derive(Debug, Clone)]
pub enum ProviderTransportEvent {
    Retry {
        attempt: usize,
        reason: String,
        body: Value,
    },
    Response {
        attempt: usize,
        status: Option<u16>,
        response_id: Option<String>,
        body: Value,
    },
}

pub type ProviderTransportCallback<'a> =
    dyn FnMut(ProviderTransportEvent) -> anyhow::Result<()> + Send + 'a;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderToolCandidate {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderToolResult {
    pub call_id: String,
    pub name: String,
    /// Legacy text output. `content` preserves structured and multimodal tool
    /// output for provider adapters and persisted events.
    pub output: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ModelInputContent>,
    pub is_error: bool,
    pub metadata: Value,
}

impl ProviderToolResult {
    pub fn content_or_legacy_text(&self) -> Vec<ModelInputContent> {
        if self.content.is_empty() {
            vec![ModelInputContent::text(self.output.clone())]
        } else {
            self.content.clone()
        }
    }
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse>;

    fn prepare(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        let body = serde_json::to_value(&request)?;
        Ok(PreparedProviderRequest {
            request_id,
            adapter: "logical".to_string(),
            method: "MODEL".to_string(),
            endpoint: "provider://logical".to_string(),
            observation_body: redact_transport_value(&body),
            body,
            logical_request: request,
        })
    }

    async fn stream_prepared(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let response = self.stream(prepared.logical_request, on_delta).await?;
        on_transport(ProviderTransportEvent::Response {
            attempt: 1,
            status: None,
            response_id: response.response_id.clone(),
            body: model_response_observation(&response),
        })?;
        Ok(response)
    }

    async fn stream(
        &self,
        request: ModelRequest,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let response = self.complete(request).await?;
        if !response.text.is_empty() {
            on_delta(ModelStreamDelta::Text {
                text: response.text.clone(),
            })?;
        }
        for (index, call) in response.tool_calls.iter().enumerate() {
            on_delta(ModelStreamDelta::ToolCall {
                index,
                id: Some(call.id.clone()),
                name: Some(call.name.clone()),
                arguments_delta: call.arguments.to_string(),
            })?;
        }
        if let Some(usage) = &response.usage {
            on_delta(ModelStreamDelta::Usage {
                usage: usage.clone(),
            })?;
        }
        Ok(response)
    }

    async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck>;
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    temperature: f64,
    max_output_tokens: Option<u32>,
    reasoning_effort: Option<String>,
    parallel_tool_calls: bool,
    prompt_cache_key: Option<String>,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            temperature: 0.2,
            max_output_tokens: None,
            reasoning_effort: None,
            parallel_tool_calls: false,
            prompt_cache_key: None,
        }
    }

    pub fn from_env() -> Option<Self> {
        let env = ProviderEnv::load();
        let api_key = env.first([
            "OPENTOPIA_API_KEY",
            "AUDIT_COPILOT_LLM_API_KEY",
            "CREDIT_REVIEW_LLM_API_KEY",
            "OPENAI_API_KEY",
        ])?;
        let base_url = env
            .first([
                "OPENTOPIA_OPENAI_BASE_URL",
                "AUDIT_COPILOT_LLM_BASE_URL",
                "CREDIT_REVIEW_LLM_BASE_URL",
                "OPENAI_BASE_URL",
            ])
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let model = env
            .first([
                "OPENTOPIA_MODEL",
                "AUDIT_COPILOT_LLM_MODEL",
                "CREDIT_REVIEW_LLM_MODEL",
                "CREDIT_REVIEW_LLM_CHEAP_MODEL",
                "CREDIT_REVIEW_LLM_STRONG_MODEL",
            ])
            .unwrap_or_else(|| "gpt-4.1-mini".to_string());
        Some(Self::new(base_url, api_key, model))
    }

    pub fn from_settings(settings: &ProviderSettings) -> Option<Self> {
        if settings.kind != ProviderKind::OpenAiCompatible {
            return None;
        }
        let api_key = std::env::var(&settings.api_key_source)
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                if settings.api_key_source != "OPENTOPIA_API_KEY" {
                    return None;
                }
                ProviderEnv::load().first([
                    "OPENTOPIA_API_KEY",
                    "AUDIT_COPILOT_LLM_API_KEY",
                    "CREDIT_REVIEW_LLM_API_KEY",
                    "OPENAI_API_KEY",
                ])
            })?;
        Some(
            Self::new(settings.base_url.clone(), api_key, settings.model.clone())
                .with_generation_settings(settings),
        )
    }

    fn with_generation_settings(mut self, settings: &ProviderSettings) -> Self {
        self.temperature = settings.temperature;
        self.max_output_tokens = settings.max_output_tokens;
        self.reasoning_effort = settings.reasoning_effort.clone();
        self.parallel_tool_calls = settings.parallel_tool_calls;
        self.prompt_cache_key = settings.prompt_cache_key.clone();
        self
    }

    pub(crate) fn for_guardian(mut self) -> Self {
        self.temperature = 0.0;
        self.max_output_tokens = Some(self.max_output_tokens.unwrap_or(1_024).min(1_024));
        if self.reasoning_effort.is_some() {
            self.reasoning_effort = Some("low".to_string());
        }
        self.parallel_tool_calls = false;
        self
    }

    fn prepare_chat_request(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let mut payload = json!({
            "model": self.model,
            "temperature": self.temperature,
            "messages": openai_messages(&request),
            "stream": true,
            "stream_options": { "include_usage": true }
        });
        if let Some(max_output_tokens) = self.max_output_tokens {
            payload["max_tokens"] = json!(max_output_tokens);
        }
        if let Some(reasoning_effort) = self
            .reasoning_effort
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            payload["reasoning_effort"] = json!(reasoning_effort);
        }
        if !request.tool_candidates.is_empty() {
            payload["tools"] = json!(openai_tools(&request.tool_candidates));
            payload["tool_choice"] = json!("auto");
            payload["parallel_tool_calls"] = json!(self.parallel_tool_calls);
        }
        if let Some(prompt_cache_key) = request
            .prompt_cache_key
            .as_deref()
            .or(self.prompt_cache_key.as_deref())
            .filter(|value| !value.is_empty())
        {
            payload["prompt_cache_key"] = json!(prompt_cache_key);
        }
        if let Some(schema) = request.final_output_json_schema.as_ref() {
            payload["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "guardian_assessment",
                    "strict": true,
                    "schema": schema,
                }
            });
        }

        Ok(PreparedProviderRequest {
            request_id,
            adapter: "openai_chat_completions".to_string(),
            method: "POST".to_string(),
            endpoint: url,
            observation_body: redact_transport_value(&payload),
            body: payload,
            logical_request: request,
        })
    }

    async fn execute_chat_request(
        &self,
        mut prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let mut attempt = 1;
        let mut response = self
            .client
            .post(&prepared.endpoint)
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&prepared.body)
            .send()
            .await?;
        if response.status().as_u16() == 400
            && chat_request_has_compatibility_fallback(&prepared.logical_request)
        {
            let rejected_body = response.text().await?;
            attempt = 2;
            let mut changes = Vec::new();
            if prepared.logical_request.final_output_json_schema.is_some() {
                if let Some(body) = prepared.body.as_object_mut() {
                    body.remove("response_format");
                }
                changes.push("structured response format");
            }
            if chat_request_needs_message_compatibility_fallback(&prepared.logical_request) {
                prepared.body["messages"] =
                    json!(openai_compatibility_messages(&prepared.logical_request));
                changes.push("native developer messages or structured tool history");
            }
            on_transport(ProviderTransportEvent::Retry {
                attempt,
                reason: truncate_observation_text(&format!(
                    "provider rejected {} with HTTP 400: {rejected_body}",
                    changes.join(" and ")
                )),
                body: redact_transport_value(&prepared.body),
            })?;
            let retry = self
                .client
                .post(&prepared.endpoint)
                .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
                .header(CONTENT_TYPE, "application/json")
                .json(&prepared.body)
                .send()
                .await?;
            if !retry.status().is_success() {
                let retry_status = retry.status();
                let retry_body = retry.text().await?;
                on_transport(ProviderTransportEvent::Response {
                    attempt,
                    status: Some(retry_status.as_u16()),
                    response_id: None,
                    body: json!({ "error": truncate_observation_text(&retry_body) }),
                })?;
                anyhow::bail!(
                    "provider request failed (400): {rejected_body}; compatibility retry failed ({retry_status}): {retry_body}"
                );
            }
            response = retry;
        }
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await?;
            on_transport(ProviderTransportEvent::Response {
                attempt,
                status: Some(status.as_u16()),
                response_id: None,
                body: json!({ "error": truncate_observation_text(&body) }),
            })?;
            anyhow::bail!("provider request failed ({status}): {body}");
        }

        let mut decoder = SseDecoder::default();
        let mut accumulator = OpenAiStreamAccumulator::default();
        while let Some(chunk) = response.chunk().await? {
            for data in decoder.push(&chunk)? {
                if data == "[DONE]" {
                    continue;
                }
                let event: Value = serde_json::from_str(&data)
                    .map_err(|err| anyhow::anyhow!("invalid provider SSE data: {err}: {data}"))?;
                accumulator.apply(&event, on_delta)?;
            }
        }
        for data in decoder.finish()? {
            if data != "[DONE]" {
                let event: Value = serde_json::from_str(&data)
                    .map_err(|err| anyhow::anyhow!("invalid provider SSE data: {err}: {data}"))?;
                accumulator.apply(&event, on_delta)?;
            }
        }

        let response = accumulator.finish()?;
        on_transport(ProviderTransportEvent::Response {
            attempt,
            status: Some(status.as_u16()),
            response_id: response.response_id.clone(),
            body: model_response_observation(&response),
        })?;
        Ok(response)
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiResponsesProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    temperature: f64,
    max_output_tokens: Option<u32>,
    reasoning_effort: Option<String>,
    store_responses: bool,
    parallel_tool_calls: bool,
    prompt_cache_key: Option<String>,
    prompt_cache_policy: Option<PromptCachePolicy>,
    compaction_threshold_tokens: Option<u32>,
    native_web_search: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("provider request failed ({status}): {body}")]
struct ResponsesRequestError {
    status: reqwest::StatusCode,
    body: String,
}

impl ResponsesRequestError {
    fn invalid_previous_response(&self, response_id: &str) -> bool {
        if !matches!(self.status.as_u16(), 400 | 404) {
            return false;
        }
        let body = self.body.to_ascii_lowercase();
        body.contains("previous_response_id")
            || body.contains("previous response")
            || body.contains(&response_id.to_ascii_lowercase())
    }
}

impl OpenAiResponsesProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            temperature: 0.2,
            max_output_tokens: None,
            reasoning_effort: None,
            store_responses: false,
            parallel_tool_calls: false,
            prompt_cache_key: None,
            prompt_cache_policy: None,
            compaction_threshold_tokens: None,
            native_web_search: false,
        }
    }

    pub fn from_settings(settings: &ProviderSettings) -> Option<Self> {
        if settings.kind != ProviderKind::OpenAiResponses {
            return None;
        }
        let api_key = provider_api_key(settings)?;
        let mut provider = Self::new(settings.base_url.clone(), api_key, settings.model.clone());
        provider.temperature = settings.temperature;
        provider.max_output_tokens = settings.max_output_tokens;
        provider.reasoning_effort = settings.reasoning_effort.clone();
        provider.store_responses = settings.store_responses;
        provider.parallel_tool_calls = settings.parallel_tool_calls;
        provider.prompt_cache_key = settings.prompt_cache_key.clone();
        provider.prompt_cache_policy = settings.prompt_cache_policy;
        provider.compaction_threshold_tokens = settings.responses_compaction_threshold_tokens;
        Some(provider)
    }

    pub(crate) fn for_guardian(mut self) -> Self {
        self.temperature = 0.0;
        self.max_output_tokens = Some(self.max_output_tokens.unwrap_or(1_024).min(1_024));
        if self.reasoning_effort.is_some() {
            self.reasoning_effort = Some("low".to_string());
        }
        self.parallel_tool_calls = false;
        self.native_web_search = false;
        self
    }

    pub(crate) fn with_native_web_search(mut self, enabled: bool) -> Self {
        self.native_web_search = enabled;
        self
    }

    fn prepare_responses_request(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        let endpoint = format!("{}/responses", self.base_url.trim_end_matches('/'));
        let mut payload = json!({
            "model": self.model,
            "input": responses_input(&request),
            "stream": true,
            "store": self.store_responses,
            "parallel_tool_calls": self.parallel_tool_calls,
            "temperature": self.temperature,
        });
        let system_instructions = responses_system_instructions(&request);
        if !system_instructions.trim().is_empty() {
            payload["instructions"] = json!(system_instructions);
        }
        if let Some(previous_response_id) = request
            .previous_response_id
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            payload["previous_response_id"] = json!(previous_response_id);
        }
        if let Some(max_output_tokens) = self.max_output_tokens {
            payload["max_output_tokens"] = json!(max_output_tokens);
        }
        if let Some(reasoning_effort) = self
            .reasoning_effort
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            payload["reasoning"] = json!({ "effort": reasoning_effort });
            if !self.store_responses {
                payload["include"] = json!(["reasoning.encrypted_content"]);
            }
        }
        let mut tools = responses_tools(&request.tool_candidates);
        if self.native_web_search {
            tools.push(json!({ "type": "web_search" }));
        }
        if !tools.is_empty() {
            payload["tools"] = json!(tools);
            payload["tool_choice"] = json!("auto");
        }
        if let Some(prompt_cache_key) = request
            .prompt_cache_key
            .as_deref()
            .or(self.prompt_cache_key.as_deref())
            .filter(|value| !value.is_empty())
        {
            payload["prompt_cache_key"] = json!(prompt_cache_key);
        }
        match self.prompt_cache_policy {
            Some(PromptCachePolicy::Explicit30m) => {
                payload["prompt_cache_options"] = json!({
                    "mode": "explicit",
                    "ttl": "30m",
                });
                add_responses_prompt_cache_breakpoint(&mut payload["input"], &request);
            }
            Some(PromptCachePolicy::LegacyInMemory) => {
                payload["prompt_cache_retention"] = json!("in_memory");
            }
            Some(PromptCachePolicy::Legacy24h) => {
                payload["prompt_cache_retention"] = json!("24h");
            }
            None => {}
        }
        if let Some(threshold) = self.compaction_threshold_tokens.filter(|value| *value > 0) {
            payload["context_management"] = json!([{
                "type": "compaction",
                "compact_threshold": threshold,
            }]);
        }
        if let Some(schema) = request.final_output_json_schema.as_ref() {
            payload["text"] = json!({
                "format": {
                    "type": "json_schema",
                    "name": "guardian_assessment",
                    "strict": true,
                    "schema": schema,
                }
            });
        }

        Ok(PreparedProviderRequest {
            request_id,
            adapter: "openai_responses".to_string(),
            method: "POST".to_string(),
            endpoint,
            observation_body: redact_transport_value(&payload),
            body: payload,
            logical_request: request,
        })
    }

    async fn execute_responses_request(
        &self,
        prepared: PreparedProviderRequest,
        attempt: usize,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let response = self
            .client
            .post(&prepared.endpoint)
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&prepared.body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await?;
            on_transport(ProviderTransportEvent::Response {
                attempt,
                status: Some(status.as_u16()),
                response_id: None,
                body: json!({ "error": truncate_observation_text(&body) }),
            })?;
            return Err(ResponsesRequestError { status, body }.into());
        }

        let mut decoder = SseDecoder::default();
        let mut accumulator = ResponsesStreamAccumulator::default();
        let mut response = response;
        while let Some(chunk) = response.chunk().await? {
            for data in decoder.push(&chunk)? {
                if data == "[DONE]" {
                    continue;
                }
                let event: Value = serde_json::from_str(&data)
                    .map_err(|err| anyhow::anyhow!("invalid Responses SSE data: {err}: {data}"))?;
                accumulator.apply(&event, on_delta)?;
            }
        }
        for data in decoder.finish()? {
            if data != "[DONE]" {
                let event: Value = serde_json::from_str(&data)
                    .map_err(|err| anyhow::anyhow!("invalid Responses SSE data: {err}: {data}"))?;
                accumulator.apply(&event, on_delta)?;
            }
        }
        let response = accumulator.finish()?;
        on_transport(ProviderTransportEvent::Response {
            attempt,
            status: Some(status.as_u16()),
            response_id: response.response_id.clone(),
            body: model_response_observation(&response),
        })?;
        Ok(response)
    }
}

#[derive(Debug, Default)]
struct SseDecoder {
    buffer: Vec<u8>,
    data_lines: Vec<String>,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> anyhow::Result<Vec<String>> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line = self.buffer.drain(..=newline).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8(line)
                .map_err(|err| anyhow::anyhow!("provider SSE was not valid UTF-8: {err}"))?;
            self.process_line(&line, &mut events);
        }
        Ok(events)
    }

    fn finish(&mut self) -> anyhow::Result<Vec<String>> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let line = String::from_utf8(std::mem::take(&mut self.buffer))
                .map_err(|err| anyhow::anyhow!("provider SSE was not valid UTF-8: {err}"))?;
            self.process_line(line.trim_end_matches('\r'), &mut events);
        }
        self.dispatch(&mut events);
        Ok(events)
    }

    fn process_line(&mut self, line: &str, events: &mut Vec<String>) {
        if line.is_empty() {
            self.dispatch(events);
            return;
        }
        if line.starts_with(':') {
            return;
        }
        if let Some(data) = line.strip_prefix("data:") {
            self.data_lines
                .push(data.strip_prefix(' ').unwrap_or(data).to_string());
        }
    }

    fn dispatch(&mut self, events: &mut Vec<String>) {
        if !self.data_lines.is_empty() {
            events.push(std::mem::take(&mut self.data_lines).join("\n"));
        }
    }
}

#[derive(Debug, Default)]
struct StreamingToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Default)]
struct OpenAiStreamAccumulator {
    text: String,
    tool_calls: BTreeMap<usize, StreamingToolCall>,
    usage: Option<ModelUsage>,
    finish_reason: Option<ModelFinishReason>,
}

impl OpenAiStreamAccumulator {
    fn apply(
        &mut self,
        event: &Value,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<()> {
        if let Some(error) = event.get("error") {
            anyhow::bail!("provider stream returned an error: {error}");
        }

        if let Some(usage) = parse_model_usage(event.get("usage")) {
            self.usage = Some(usage.clone());
            on_delta(ModelStreamDelta::Usage { usage })?;
        }

        if let Some(reason) = event
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
        {
            self.finish_reason = Some(chat_finish_reason(reason));
        }

        let Some(delta) = event.pointer("/choices/0/delta") else {
            return Ok(());
        };
        let reasoning = extract_stream_reasoning(delta);
        if !reasoning.is_empty() {
            on_delta(ModelStreamDelta::Reasoning { text: reasoning })?;
        }
        let text = extract_stream_text(delta.get("content"));
        if !text.is_empty() {
            self.text.push_str(&text);
            on_delta(ModelStreamDelta::Text { text })?;
        }

        let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) else {
            return Ok(());
        };
        for (fallback_index, value) in tool_calls.iter().enumerate() {
            let index = value
                .get("index")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(fallback_index);
            let id_delta = value.get("id").and_then(Value::as_str);
            let name_delta = value.pointer("/function/name").and_then(Value::as_str);
            let arguments_delta = value
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("");
            let call = self.tool_calls.entry(index).or_default();
            if let Some(id) = id_delta {
                call.id.push_str(id);
            }
            if let Some(name) = name_delta {
                call.name.push_str(name);
            }
            call.arguments.push_str(arguments_delta);
            on_delta(ModelStreamDelta::ToolCall {
                index,
                id: id_delta.map(str::to_string),
                name: name_delta.map(str::to_string),
                arguments_delta: arguments_delta.to_string(),
            })?;
        }
        Ok(())
    }

    fn finish(self) -> anyhow::Result<ModelResponse> {
        let tool_calls = self
            .tool_calls
            .into_iter()
            .map(|(index, call)| {
                if call.name.is_empty() {
                    anyhow::bail!("streamed tool call {index} was missing a function name");
                }
                let id = if call.id.is_empty() {
                    format!("call_{index}")
                } else {
                    call.id
                };
                Ok(ProviderToolCall {
                    id,
                    name: call.name,
                    arguments: parse_tool_arguments(Some(&Value::String(call.arguments)))?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(ModelResponse {
            text: self.text,
            tool_calls,
            usage: self.usage,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: self
                .finish_reason
                .unwrap_or(ModelFinishReason::StreamInterrupted),
        })
    }
}

#[derive(Debug, Default)]
struct ResponsesStreamAccumulator {
    text: String,
    tool_calls: BTreeMap<usize, StreamingToolCall>,
    provider_items: BTreeMap<usize, Value>,
    usage: Option<ModelUsage>,
    response_id: Option<String>,
    completed_response: Option<Value>,
    finish_reason: Option<ModelFinishReason>,
}

impl ResponsesStreamAccumulator {
    fn apply(
        &mut self,
        event: &Value,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<()> {
        let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(event_type, "error" | "response.failed") {
            anyhow::bail!("Responses stream returned an error: {event}");
        }
        if let Some(response_id) = event
            .get("response_id")
            .or_else(|| event.pointer("/response/id"))
            .and_then(Value::as_str)
        {
            self.response_id = Some(response_id.to_string());
        }
        if let Some(usage) = parse_model_usage(
            event
                .get("usage")
                .or_else(|| event.pointer("/response/usage")),
        ) {
            self.usage = Some(usage.clone());
            on_delta(ModelStreamDelta::Usage { usage })?;
        }

        match event_type {
            "response.output_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    self.text.push_str(delta);
                    on_delta(ModelStreamDelta::Text {
                        text: delta.to_string(),
                    })?;
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    on_delta(ModelStreamDelta::Reasoning {
                        text: delta.to_string(),
                    })?;
                }
            }
            "response.output_item.added" | "response.output_item.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(self.provider_items.len() as u64)
                    as usize;
                if let Some(item) = event.get("item") {
                    self.provider_items.insert(index, item.clone());
                    if item.get("type").and_then(Value::as_str) == Some("function_call") {
                        let call = self.tool_calls.entry(index).or_default();
                        if let Some(id) = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(Value::as_str)
                        {
                            call.id = id.to_string();
                        }
                        if let Some(name) = item.get("name").and_then(Value::as_str) {
                            call.name = name.to_string();
                        }
                        if event_type == "response.output_item.done" {
                            if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
                                call.arguments = arguments.to_string();
                            }
                        }
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let delta = event.get("delta").and_then(Value::as_str).unwrap_or("");
                let call = self.tool_calls.entry(index).or_default();
                call.arguments.push_str(delta);
                on_delta(ModelStreamDelta::ToolCall {
                    index,
                    id: (!call.id.is_empty()).then(|| call.id.clone()),
                    name: (!call.name.is_empty()).then(|| call.name.clone()),
                    arguments_delta: delta.to_string(),
                })?;
            }
            "response.function_call_arguments.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(arguments) = event.get("arguments").and_then(Value::as_str) {
                    self.tool_calls.entry(index).or_default().arguments = arguments.to_string();
                }
            }
            "response.completed" | "response.incomplete" => {
                if let Some(response) = event.get("response") {
                    self.completed_response = Some(response.clone());
                    self.finish_reason = Some(if event_type == "response.completed" {
                        responses_finish_reason(response, ModelFinishReason::Completed)
                    } else {
                        responses_finish_reason(
                            response,
                            ModelFinishReason::Incomplete("response.incomplete".to_string()),
                        )
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(mut self) -> anyhow::Result<ModelResponse> {
        if let Some(completed) = self.completed_response.as_ref() {
            let completed_text = extract_response_text(completed);
            if !completed_text.is_empty() {
                self.text = completed_text;
            }
            let completed_calls = extract_provider_tool_calls(completed)?;
            if !completed_calls.is_empty() {
                self.tool_calls = completed_calls
                    .into_iter()
                    .enumerate()
                    .map(|(index, call)| {
                        (
                            index,
                            StreamingToolCall {
                                id: call.id,
                                name: call.name,
                                arguments: call.arguments.to_string(),
                            },
                        )
                    })
                    .collect();
            }
            if let Some(output) = completed.get("output").and_then(Value::as_array) {
                self.provider_items = output
                    .iter()
                    .cloned()
                    .enumerate()
                    .collect::<BTreeMap<_, _>>();
            }
            self.usage = parse_model_usage(completed.get("usage")).or(self.usage);
            self.response_id = completed
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(self.response_id);
        }
        let tool_calls = self
            .tool_calls
            .into_iter()
            .map(|(index, call)| {
                if call.name.is_empty() {
                    anyhow::bail!("Responses tool call {index} was missing a function name");
                }
                Ok(ProviderToolCall {
                    id: if call.id.is_empty() {
                        format!("call_{index}")
                    } else {
                        call.id
                    },
                    name: call.name,
                    arguments: parse_tool_arguments(Some(&Value::String(call.arguments)))?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(ModelResponse {
            text: self.text,
            tool_calls,
            usage: self.usage,
            response_id: self.response_id,
            provider_items: self.provider_items.into_values().collect(),
            finish_reason: self
                .finish_reason
                .unwrap_or(ModelFinishReason::StreamInterrupted),
        })
    }
}

fn chat_finish_reason(reason: &str) -> ModelFinishReason {
    match reason {
        "stop" | "end_turn" => ModelFinishReason::Stop,
        "tool_calls" | "function_call" => ModelFinishReason::ToolCalls,
        "length" | "max_tokens" | "max_output_tokens" => ModelFinishReason::Length,
        "content_filter" => ModelFinishReason::ContentFilter,
        other => ModelFinishReason::Incomplete(other.to_string()),
    }
}

fn responses_finish_reason(response: &Value, fallback: ModelFinishReason) -> ModelFinishReason {
    match response.get("status").and_then(Value::as_str) {
        Some("completed") => ModelFinishReason::Completed,
        Some("incomplete") => response
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
            .map(chat_finish_reason)
            .unwrap_or_else(|| ModelFinishReason::Incomplete("response incomplete".to_string())),
        Some(status) => ModelFinishReason::Incomplete(status.to_string()),
        None => fallback,
    }
}

fn extract_stream_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.get("content").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn extract_stream_reasoning(delta: &Value) -> String {
    delta
        .get("reasoning_content")
        .or_else(|| delta.get("reasoning"))
        .map(extract_reasoning_value)
        .unwrap_or_default()
}

fn extract_reasoning_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(extract_reasoning_value)
            .collect::<Vec<_>>()
            .join(""),
        Value::Object(fields) => ["text", "content", "summary", "output_text"]
            .into_iter()
            .find_map(|key| fields.get(key))
            .map(extract_reasoning_value)
            .unwrap_or_default(),
        _ => String::new(),
    }
}

#[derive(Debug, Default)]
struct ProviderEnv {
    values: HashMap<String, String>,
}

fn provider_api_key(settings: &ProviderSettings) -> Option<String> {
    std::env::var(&settings.api_key_source)
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            if settings.api_key_source != "OPENTOPIA_API_KEY" {
                return None;
            }
            ProviderEnv::load().first([
                "OPENTOPIA_API_KEY",
                "AUDIT_COPILOT_LLM_API_KEY",
                "CREDIT_REVIEW_LLM_API_KEY",
                "OPENAI_API_KEY",
            ])
        })
}

impl ProviderEnv {
    fn load() -> Self {
        let mut values = std::env::vars().collect::<HashMap<_, _>>();
        for path in candidate_env_files() {
            merge_dotenv_file(&mut values, &path);
        }
        Self { values }
    }

    fn first<const N: usize>(&self, keys: [&str; N]) -> Option<String> {
        keys.into_iter().find_map(|key| {
            self.values
                .get(key)
                .filter(|value| !value.is_empty())
                .cloned()
        })
    }
}

fn candidate_env_files() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(path) = std::env::var("OPENTOPIA_ENV_FILE") {
        paths.push(PathBuf::from(path));
    }
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join(".env"));
        if let Some(parent) = cwd.parent() {
            paths.push(parent.join(credit_review_project_name()).join(".env"));
            paths.extend(find_sibling_credit_review_env_files(parent));
        }
    }
    paths
}

fn credit_review_project_name() -> String {
    [0x4FE1, 0x8D37, 0x5BA1, 0x6838, 0x52A9, 0x624B]
        .into_iter()
        .filter_map(char::from_u32)
        .collect()
}

fn find_sibling_credit_review_env_files(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(".env"))
        .filter(|path| {
            std::fs::read_to_string(path).is_ok_and(|content| {
                content.contains("CREDIT_REVIEW_LLM_API_KEY")
                    || content.contains("AUDIT_COPILOT_LLM_API_KEY")
            })
        })
        .collect()
}

fn merge_dotenv_file(values: &mut HashMap<String, String>, path: &Path) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };

    for line in content.lines() {
        let Some((key, value)) = parse_dotenv_line(line) else {
            continue;
        };
        values.entry(key).or_insert(value);
    }
}

fn parse_dotenv_line(line: &str) -> Option<(String, String)> {
    let mut line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    if let Some(rest) = line.strip_prefix("export ") {
        line = rest.trim();
    }

    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }

    let value = strip_env_quotes(value.trim());
    Some((key.to_string(), value.to_string()))
}

fn strip_env_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn instruction_messages(request: &ModelRequest) -> Vec<(ContextRole, String)> {
    if request.context_items.is_empty() {
        return vec![(ContextRole::System, request.system_prompt.clone())];
    }
    CompiledModelContext {
        items: request.context_items.clone(),
        prompt_cache_key: request.prompt_cache_key.clone(),
    }
    .instruction_messages()
}

fn openai_instruction_messages(request: &ModelRequest) -> Vec<Value> {
    instruction_messages(request)
        .into_iter()
        .map(|(role, content)| {
            json!({
                "role": match role {
                    ContextRole::System => "system",
                    ContextRole::Developer => "developer",
                    _ => unreachable!("instruction messages contain only system/developer roles"),
                },
                "content": content,
            })
        })
        .collect()
}

fn responses_system_instructions(request: &ModelRequest) -> String {
    instruction_messages(request)
        .into_iter()
        .filter_map(|(role, content)| (role == ContextRole::System).then_some(content))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn chat_request_needs_message_compatibility_fallback(request: &ModelRequest) -> bool {
    !request.tool_results.is_empty()
        || instruction_messages(request)
            .iter()
            .any(|(role, _)| *role == ContextRole::Developer)
}

fn chat_request_has_compatibility_fallback(request: &ModelRequest) -> bool {
    request.final_output_json_schema.is_some()
        || chat_request_needs_message_compatibility_fallback(request)
}

fn openai_messages(request: &ModelRequest) -> Vec<Value> {
    let mut messages = openai_instruction_messages(request);

    messages.extend(request.conversation.iter().map(|message| {
        json!({
            "role": openai_conversation_role(message.role),
            "content": openai_message_content(&message.content, &message.content_parts)
        })
    }));
    if let Some(instructions) = request
        .branch_developer_instructions
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        messages.push(json!({
            "role": "developer",
            "content": instructions,
        }));
    }
    messages.push(json!({
        "role": "user",
        "content": openai_message_content(&request.user_message, &request.user_content)
    }));

    // AgentCore keeps completed calls as a flat durable history. Rebuild a valid
    // Chat Completions sequence instead of collapsing calls from several model
    // rounds into one synthetic assistant message.
    let mut emitted_results = vec![false; request.tool_results.len()];
    for call in &request.previous_tool_calls {
        messages.push(json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [openai_tool_call_message(call)]
        }));
        for (index, result) in request.tool_results.iter().enumerate() {
            if result.call_id == call.id {
                messages.push(openai_tool_result_message(result));
                emitted_results[index] = true;
            }
        }
    }
    for (index, result) in request.tool_results.iter().enumerate() {
        if !emitted_results[index] {
            messages.push(openai_tool_result_message(result));
        }
    }
    if let Some(companion) = openai_tool_image_companion(&request.tool_results) {
        messages.push(companion);
    }

    messages
}

fn openai_compatibility_messages(request: &ModelRequest) -> Vec<Value> {
    let mut messages = vec![json!({
        "role": "system",
        "content": &request.system_prompt
    })];
    messages.extend(request.conversation.iter().map(|message| {
        json!({
            "role": openai_conversation_role(message.role),
            "content": openai_message_content(&message.content, &message.content_parts)
        })
    }));
    if let Some(instructions) = request
        .branch_developer_instructions
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        messages.push(json!({
            "role": "system",
            "content": instructions,
        }));
    }
    messages.push(json!({
        "role": "user",
        "content": openai_message_content(&request.user_message, &request.user_content)
    }));

    let history = request
        .previous_tool_calls
        .iter()
        .map(|call| {
            let results = request
                .tool_results
                .iter()
                .filter(|result| result.call_id == call.id)
                .map(|result| {
                    json!({
                        "output": &result.output,
                        "isError": result.is_error,
                        "metadata": &result.metadata
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "callId": &call.id,
                "name": &call.name,
                "arguments": &call.arguments,
                "results": results
            })
        })
        .collect::<Vec<_>>();
    if !history.is_empty() {
        messages.push(json!({
            "role": "user",
            "content": format!(
                "Continue the original task using this authoritative completed tool history. Do not repeat completed calls unless needed:\n{}",
                Value::Array(history)
            )
        }));
    }
    messages
}

fn openai_conversation_role(role: ModelConversationRole) -> &'static str {
    match role {
        ModelConversationRole::System => "system",
        ModelConversationRole::User => "user",
        ModelConversationRole::Assistant => "assistant",
    }
}

fn openai_tools(candidates: &[ProviderToolCandidate]) -> Vec<Value> {
    candidates
        .iter()
        .map(|candidate| {
            json!({
            "type": "function",
            "function": {
                    "name": &candidate.name,
                    "description": &candidate.description,
                    "parameters": &candidate.input_schema
                }
            })
        })
        .collect()
}

fn responses_tools(candidates: &[ProviderToolCandidate]) -> Vec<Value> {
    candidates
        .iter()
        .map(|candidate| {
            json!({
                "type": "function",
                "name": &candidate.name,
                "description": &candidate.description,
                "parameters": &candidate.input_schema,
                "strict": false,
            })
        })
        .collect()
}

fn responses_input(request: &ModelRequest) -> Vec<Value> {
    let replay_full_prefix = request.previous_response_id.is_none();
    let mut input = Vec::new();
    if replay_full_prefix {
        input.extend(
            instruction_messages(request)
                .into_iter()
                .filter_map(|(role, content)| {
                    (role == ContextRole::Developer).then(|| {
                        json!({
                            "role": "developer",
                            "content": content,
                        })
                    })
                }),
        );
        input.extend(request.conversation.iter().map(|message| {
            json!({
                "role": openai_conversation_role(message.role),
                "content": responses_message_content(
                    message.role,
                    &message.content,
                    &message.content_parts,
                ),
            })
        }));
        if let Some(instructions) = request
            .branch_developer_instructions
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            input.push(json!({
                "role": "developer",
                "content": instructions,
            }));
        }
    }
    input.push(json!({
        "role": "user",
        "content": responses_message_content(
            ModelConversationRole::User,
            &request.user_message,
            &request.user_content,
        ),
    }));

    if request.previous_response_items.is_empty() {
        input.extend(request.previous_tool_calls.iter().map(|call| {
            json!({
                "type": "function_call",
                "call_id": &call.id,
                "name": &call.name,
                "arguments": call.arguments.to_string(),
            })
        }));
    } else {
        input.extend(request.previous_response_items.iter().cloned());
    }
    input.extend(request.tool_results.iter().map(|result| {
        json!({
            "type": "function_call_output",
            "call_id": &result.call_id,
            "output": provider_tool_result_content(result),
        })
    }));
    if let Some(companion) = responses_tool_image_companion(&request.tool_results) {
        input.push(companion);
    }
    input
}

fn add_responses_prompt_cache_breakpoint(input: &mut Value, request: &ModelRequest) {
    if request.context_items.is_empty() {
        return;
    }

    let compiled = CompiledModelContext {
        items: request.context_items.clone(),
        prompt_cache_key: request.prompt_cache_key.clone(),
    };
    let breakpoint_index = compiled
        .ordered_items()
        .into_iter()
        .filter(|item| {
            item.role == ContextRole::Developer
                && item.kind != ContextItemKind::Summary
                && !item.text_content().trim().is_empty()
        })
        .enumerate()
        .filter_map(|(index, item)| {
            matches!(
                item.cache_scope,
                ContextCacheScope::Stable | ContextCacheScope::Thread
            )
            .then_some(index)
        })
        .last();
    let Some(items) = input.as_array_mut() else {
        return;
    };
    if request.previous_response_id.is_none() {
        if let Some(index) = breakpoint_index {
            if let Some(message) = items.get_mut(index) {
                mark_responses_message_cache_breakpoint(message);
            }
        }
    }

    let replay_full_prefix = request.previous_response_id.is_none();
    let developer_count = if replay_full_prefix {
        instruction_messages(request)
            .into_iter()
            .filter(|(role, _)| *role == ContextRole::Developer)
            .count()
    } else {
        0
    };
    let replayed_conversation_count = if replay_full_prefix {
        request.conversation.len()
    } else {
        0
    };
    let has_branch_instructions = replay_full_prefix
        && request
            .branch_developer_instructions
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());

    if has_branch_instructions && replayed_conversation_count > 0 {
        let inherited_prefix_end = developer_count + replayed_conversation_count - 1;
        if let Some(message) = items.get_mut(inherited_prefix_end) {
            mark_responses_message_cache_breakpoint(message);
        }
    }

    let current_user_index =
        developer_count + replayed_conversation_count + usize::from(has_branch_instructions);
    if let Some(message) = items.get_mut(current_user_index) {
        mark_responses_message_cache_breakpoint(message);
    }
}

fn mark_responses_message_cache_breakpoint(message: &mut Value) {
    let content_type = if message.get("role").and_then(Value::as_str) == Some("assistant") {
        "output_text"
    } else {
        "input_text"
    };
    let Some(content) = message.get_mut("content") else {
        return;
    };
    if let Some(text) = content.as_str().map(str::to_string) {
        *content = json!([{
            "type": content_type,
            "text": text,
            "prompt_cache_breakpoint": { "mode": "explicit" },
        }]);
        return;
    }
    let Some(parts) = content.as_array_mut() else {
        return;
    };
    if let Some(part) = parts.iter_mut().rev().find(|part| {
        matches!(
            part.get("type").and_then(Value::as_str),
            Some("input_text") | Some("output_text")
        )
    }) {
        part["prompt_cache_breakpoint"] = json!({ "mode": "explicit" });
    }
}

fn responses_message_content(
    role: ModelConversationRole,
    legacy_text: &str,
    parts: &[ModelInputContent],
) -> Value {
    if parts.is_empty() {
        return Value::String(legacy_text.to_string());
    }
    let text_type = if role == ModelConversationRole::Assistant {
        "output_text"
    } else {
        "input_text"
    };
    let mut content = Vec::new();
    if !legacy_text.is_empty() {
        content.push(json!({ "type": text_type, "text": legacy_text }));
    }
    content.extend(parts.iter().map(|part| match part {
        ModelInputContent::Text { text } => json!({ "type": text_type, "text": text }),
        ModelInputContent::Json { value } => {
            json!({ "type": text_type, "text": value.to_string() })
        }
        ModelInputContent::Image { content_type, data } => json!({
            "type": "input_image",
            "image_url": format!("data:{content_type};base64,{}", encode_base64(data)),
        }),
        ModelInputContent::Resource {
            uri,
            content_type,
            name,
        } => json!({
            "type": text_type,
            "text": resource_fallback_text(uri, content_type.as_deref(), name.as_deref()),
        }),
    }));
    Value::Array(content)
}

fn responses_tool_image_companion(results: &[ProviderToolResult]) -> Option<Value> {
    let mut content = Vec::new();
    for result in results {
        for part in &result.content {
            if let ModelInputContent::Image { content_type, data } = part {
                content.push(json!({
                    "type": "input_text",
                    "text": format!("Tool image: {} (call {})", result.name, result.call_id),
                }));
                content.push(json!({
                    "type": "input_image",
                    "image_url": format!("data:{content_type};base64,{}", encode_base64(data)),
                }));
            }
        }
    }
    (!content.is_empty()).then(|| json!({ "role": "user", "content": content }))
}

fn openai_tool_call_message(call: &ProviderToolCall) -> Value {
    json!({
        "id": &call.id,
        "type": "function",
        "function": {
            "name": &call.name,
            "arguments": call.arguments.to_string()
        }
    })
}

fn provider_tool_result_content(result: &ProviderToolResult) -> String {
    let mut payload = json!({
        "output": &result.output,
        "isError": result.is_error,
        "metadata": &result.metadata
    });
    if !result.content.is_empty() {
        payload["content"] = json!(result
            .content
            .iter()
            .map(openai_tool_result_part)
            .collect::<Vec<_>>());
    }
    payload.to_string()
}

/// Chat Completions accepts native image content on user/assistant messages.
/// Resources and JSON have no portable Chat Completions content-part analogue,
/// so they remain explicit text/JSON representations instead of being dropped.
fn openai_message_content(legacy_text: &str, parts: &[ModelInputContent]) -> Value {
    if parts.is_empty() {
        return Value::String(legacy_text.to_string());
    }

    let mut content = Vec::new();
    if !legacy_text.is_empty() {
        content.push(json!({ "type": "text", "text": legacy_text }));
    }
    content.extend(parts.iter().map(openai_input_part));
    Value::Array(content)
}

fn openai_input_part(part: &ModelInputContent) -> Value {
    match part {
        ModelInputContent::Text { text } => json!({ "type": "text", "text": text }),
        ModelInputContent::Json { value } => json!({
            "type": "text",
            "text": value.to_string()
        }),
        ModelInputContent::Image { content_type, data } => json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:{content_type};base64,{}", encode_base64(data))
            }
        }),
        ModelInputContent::Resource {
            uri,
            content_type,
            name,
        } => json!({
            "type": "text",
            "text": resource_fallback_text(uri, content_type.as_deref(), name.as_deref())
        }),
    }
}

fn openai_tool_result_message(result: &ProviderToolResult) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": &result.call_id,
        "content": provider_tool_result_content(result)
    })
}

// A Chat Completions tool message is text-only across OpenAI-compatible APIs.
// Keep image metadata in its JSON envelope while the bytes travel in a native
// multimodal companion message after every tool result has been acknowledged.
fn openai_tool_result_part(part: &ModelInputContent) -> Value {
    match part {
        ModelInputContent::Text { text } => json!({ "type": "text", "text": text }),
        ModelInputContent::Json { value } => json!({ "type": "json", "value": value }),
        ModelInputContent::Image { content_type, data } => json!({
            "type": "image",
            "contentType": content_type,
            "bytes": data.len(),
            "delivery": "native_companion"
        }),
        ModelInputContent::Resource {
            uri,
            content_type,
            name,
        } => json!({
            "type": "resource",
            "uri": uri,
            "contentType": content_type,
            "name": name
        }),
    }
}

fn openai_tool_image_companion(results: &[ProviderToolResult]) -> Option<Value> {
    let mut content = Vec::new();
    for result in results {
        for part in &result.content {
            if matches!(part, ModelInputContent::Image { .. }) {
                content.push(json!({
                    "type": "text",
                    "text": format!(
                        "Tool image: {} (call {})",
                        result.name, result.call_id
                    )
                }));
                content.push(openai_input_part(part));
            }
        }
    }

    (!content.is_empty()).then(|| json!({ "role": "user", "content": content }))
}

fn resource_fallback_text(uri: &str, content_type: Option<&str>, name: Option<&str>) -> String {
    let mut fields = vec![format!("uri={uri}")];
    if let Some(name) = name {
        fields.push(format!("name={name}"));
    }
    if let Some(content_type) = content_type {
        fields.push(format!("contentType={content_type}"));
    }
    format!("[Attached resource: {}]", fields.join(", "))
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[((first & 0b0000_0011) << 4 | second >> 4) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((second & 0b0000_1111) << 2 | third >> 6) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(third & 0b0011_1111) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

fn redact_transport_value(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| {
                    let normalized = key.to_ascii_lowercase();
                    let is_image_bytes = normalized == "data"
                        && fields.get("type").and_then(Value::as_str) == Some("image")
                        && value.is_array();
                    let value = if is_image_bytes {
                        Value::String(format!(
                            "[binary image omitted: {} bytes]",
                            value.as_array().map(Vec::len).unwrap_or_default()
                        ))
                    } else if matches!(
                        normalized.as_str(),
                        "authorization"
                            | "api_key"
                            | "apikey"
                            | "password"
                            | "secret"
                            | "access_token"
                            | "refresh_token"
                    ) {
                        Value::String("[REDACTED]".to_string())
                    } else {
                        redact_transport_value(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_transport_value).collect()),
        Value::String(text) if text.starts_with("data:") && text.contains(";base64,") => {
            Value::String(format!("[data URL omitted: {} chars]", text.len()))
        }
        Value::String(text) if text.len() > 256_000 => Value::String(format!(
            "{}\n[observation truncated: {} chars total]",
            text.chars().take(256_000).collect::<String>(),
            text.len()
        )),
        value => value.clone(),
    }
}

pub fn redact_model_observation(value: &Value) -> Value {
    redact_transport_value(value)
}

fn truncate_observation_text(text: &str) -> String {
    const LIMIT: usize = 16_000;
    if text.len() <= LIMIT {
        return text.to_string();
    }
    format!(
        "{}\n[observation truncated: {} chars total]",
        text.chars().take(LIMIT).collect::<String>(),
        text.len()
    )
}

fn model_response_observation(response: &ModelResponse) -> Value {
    json!({
        "responseId": response.response_id,
        "textChars": response.text.len(),
        "toolCalls": response.tool_calls,
        "finishReason": response.finish_reason,
        "usage": response.usage,
        "providerItems": redact_transport_value(&Value::Array(response.provider_items.clone())),
    })
}

#[cfg(test)]
fn parse_model_response_body(body: &Value) -> anyhow::Result<ModelResponse> {
    Ok(ModelResponse {
        text: extract_response_text(body),
        tool_calls: extract_provider_tool_calls(body)?,
        usage: parse_model_usage(body.get("usage")),
        response_id: body.get("id").and_then(Value::as_str).map(str::to_string),
        provider_items: body
            .get("output")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        finish_reason: body
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .map(chat_finish_reason)
            .unwrap_or_else(|| responses_finish_reason(body, ModelFinishReason::StreamInterrupted)),
    })
}

fn parse_model_usage(value: Option<&Value>) -> Option<ModelUsage> {
    let usage = value?.as_object()?;
    let input_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(input_tokens.saturating_add(output_tokens));
    let cached_input_tokens = usage
        .get("prompt_tokens_details")
        .or_else(|| usage.get("input_tokens_details"))
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64);
    let cache_write_tokens = usage
        .get("prompt_tokens_details")
        .or_else(|| usage.get("input_tokens_details"))
        .and_then(|details| details.get("cache_write_tokens"))
        .and_then(Value::as_u64)
        .or_else(|| usage.get("cache_write_tokens").and_then(Value::as_u64));
    let reasoning_tokens = usage
        .get("completion_tokens_details")
        .or_else(|| usage.get("output_tokens_details"))
        .and_then(|details| details.get("reasoning_tokens"))
        .and_then(Value::as_u64);

    Some(ModelUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        cached_input_tokens,
        cache_write_tokens,
        reasoning_tokens,
    })
}

fn extract_response_text(body: &Value) -> String {
    if let Some(text) = body
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
    {
        return text.to_string();
    }

    if let Some(parts) = body
        .pointer("/choices/0/message/content")
        .and_then(Value::as_array)
    {
        let text = parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.get("content").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join("");
        if !text.is_empty() {
            return text;
        }
    }

    let responses_text = body
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("output_text"))
        .filter_map(render_responses_output_text_part)
        .collect::<Vec<_>>()
        .join("");
    if !responses_text.is_empty() {
        return responses_text;
    }

    body.get("output_text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn render_responses_output_text_part(part: &Value) -> Option<String> {
    let text = part.get("text").and_then(Value::as_str)?;
    let annotations = part
        .get("annotations")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Some(apply_url_citations(text, annotations))
}

fn apply_url_citations(text: &str, annotations: &[Value]) -> String {
    let mut ranges = Vec::new();
    let mut fallback_sources = Vec::new();
    for annotation in annotations {
        if annotation.get("type").and_then(Value::as_str) != Some("url_citation") {
            continue;
        }
        let citation = annotation.get("url_citation").unwrap_or(annotation);
        let Some(url) = citation.get("url").and_then(Value::as_str) else {
            continue;
        };
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            continue;
        }
        let title = citation
            .get("title")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("Source")
            .to_string();
        match (
            citation.get("start_index").and_then(Value::as_u64),
            citation.get("end_index").and_then(Value::as_u64),
        ) {
            (Some(start), Some(end)) if start < end => {
                ranges.push((start as usize, end as usize, url.to_string(), title));
            }
            _ => fallback_sources.push((url.to_string(), title)),
        }
    }
    if ranges.is_empty() && fallback_sources.is_empty() {
        return text.to_string();
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut char_boundaries = text
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    char_boundaries.push(text.len());
    ranges.sort_by_key(|(start, _, _, _)| std::cmp::Reverse(*start));
    let mut rendered = text.to_string();
    let mut upper_bound = char_boundaries.len().saturating_sub(1);
    for (mut start, mut end, url, title) in ranges {
        while start < end && chars.get(start).is_some_and(|value| value.is_whitespace()) {
            start += 1;
        }
        while end > start
            && chars
                .get(end.saturating_sub(1))
                .is_some_and(|value| value.is_whitespace())
        {
            end -= 1;
        }
        while end < chars.len()
            && chars[end].is_alphanumeric()
            && chars
                .get(end.saturating_sub(1))
                .is_some_and(|value| value.is_alphanumeric())
        {
            end += 1;
        }
        if end > upper_bound || start >= end {
            fallback_sources.push((url, title));
            continue;
        }
        let byte_start = char_boundaries[start];
        let byte_end = char_boundaries[end];
        let label = text[byte_start..byte_end].trim();
        let label = if label.is_empty() {
            title.as_str()
        } else {
            label
        };
        rendered.replace_range(
            byte_start..byte_end,
            &format!(
                "[{}]({})",
                escape_markdown_link_label(label),
                escape_markdown_link_url(&url)
            ),
        );
        upper_bound = start;
    }

    let mut seen = std::collections::HashSet::new();
    let fallback_sources = fallback_sources
        .into_iter()
        .filter(|(url, _)| seen.insert(url.clone()))
        .collect::<Vec<_>>();
    if !fallback_sources.is_empty() {
        rendered.push_str("\n\nSources:\n");
        for (url, title) in fallback_sources {
            rendered.push_str(&format!(
                "- [{}]({})\n",
                escape_markdown_link_label(&title),
                escape_markdown_link_url(&url)
            ));
        }
        rendered.pop();
    }
    rendered
}

fn escape_markdown_link_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

fn escape_markdown_link_url(value: &str) -> String {
    value.replace(' ', "%20").replace(')', "%29")
}

fn extract_provider_tool_calls(body: &Value) -> anyhow::Result<Vec<ProviderToolCall>> {
    let mut calls = Vec::new();

    if let Some(tool_calls) = body
        .pointer("/choices/0/message/tool_calls")
        .and_then(Value::as_array)
    {
        for (index, call) in tool_calls.iter().enumerate() {
            calls.push(parse_chat_tool_call(call, index)?);
        }
    }

    if let Some(function_call) = body
        .pointer("/choices/0/message/function_call")
        .filter(|value| value.is_object())
    {
        calls.push(parse_legacy_function_call(function_call, calls.len())?);
    }

    if let Some(output) = body.get("output").and_then(Value::as_array) {
        for item in output {
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                calls.push(parse_responses_function_call(item, calls.len())?);
            }
        }
    }

    Ok(calls)
}

fn parse_chat_tool_call(value: &Value, index: usize) -> anyhow::Result<ProviderToolCall> {
    let function = value
        .get("function")
        .ok_or_else(|| anyhow::anyhow!("tool call missing function payload: {value}"))?;
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("tool call missing function name: {value}"))?;
    let arguments = parse_tool_arguments(function.get("arguments"))?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("call_{index}"));

    Ok(ProviderToolCall {
        id,
        name: name.to_string(),
        arguments,
    })
}

fn parse_legacy_function_call(value: &Value, index: usize) -> anyhow::Result<ProviderToolCall> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("function_call missing name: {value}"))?;
    Ok(ProviderToolCall {
        id: format!("call_{index}"),
        name: name.to_string(),
        arguments: parse_tool_arguments(value.get("arguments"))?,
    })
}

fn parse_responses_function_call(value: &Value, index: usize) -> anyhow::Result<ProviderToolCall> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("function_call missing name: {value}"))?;
    let id = value
        .get("call_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("call_{index}"));

    Ok(ProviderToolCall {
        id,
        name: name.to_string(),
        arguments: parse_tool_arguments(value.get("arguments"))?,
    })
}

fn parse_tool_arguments(value: Option<&Value>) -> anyhow::Result<Value> {
    match value {
        None | Some(Value::Null) => Ok(json!({})),
        Some(Value::String(arguments)) if arguments.trim().is_empty() => Ok(json!({})),
        Some(Value::String(arguments)) => serde_json::from_str(arguments)
            .map_err(|err| anyhow::anyhow!("failed to parse tool arguments as JSON: {err}")),
        Some(value) => Ok(value.clone()),
    }
}

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
        let prepared = self.prepare(Uuid::new_v4(), request)?;
        self.stream_prepared(prepared, &mut |_| Ok(()), &mut |_| Ok(()))
            .await
    }

    fn prepare(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        self.prepare_chat_request(request_id, request)
    }

    async fn stream(
        &self,
        request: ModelRequest,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let prepared = self.prepare(Uuid::new_v4(), request)?;
        self.stream_prepared(prepared, on_delta, &mut |_| Ok(()))
            .await
    }

    async fn stream_prepared(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        self.execute_chat_request(prepared, on_delta, on_transport)
            .await
    }

    async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
        let start = std::time::Instant::now();

        let models_url = format!("{}/models", self.base_url.trim_end_matches('/'));
        match tokio::time::timeout(
            Duration::from_secs(5),
            self.client
                .get(&models_url)
                .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
                .send(),
        )
        .await
        {
            Ok(Ok(response)) => {
                let latency = start.elapsed().as_millis() as u64;
                let reachable = response.status().is_success();
                Ok(ProviderHealthCheck {
                    reachable,
                    latency_ms: Some(latency),
                    model_available: reachable,
                    error: if reachable {
                        None
                    } else {
                        Some(format!("HTTP {}", response.status()))
                    },
                })
            }
            Ok(Err(_)) => {
                let chat_url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
                match tokio::time::timeout(
                    Duration::from_secs(5),
                    self.client
                        .post(&chat_url)
                        .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
                        .header(CONTENT_TYPE, "application/json")
                        .json(&json!({
                            "model": self.model,
                            "messages": [{"role": "user", "content": "hi"}],
                            "max_tokens": 1
                        }))
                        .send(),
                )
                .await
                {
                    Ok(Ok(resp)) => {
                        let latency = start.elapsed().as_millis() as u64;
                        let reachable = resp.status().is_success();
                        Ok(ProviderHealthCheck {
                            reachable,
                            latency_ms: Some(latency),
                            model_available: reachable,
                            error: if reachable {
                                None
                            } else {
                                Some(format!("HTTP {}", resp.status()))
                            },
                        })
                    }
                    Ok(Err(err)) => {
                        let latency = start.elapsed().as_millis() as u64;
                        Ok(ProviderHealthCheck {
                            reachable: false,
                            latency_ms: Some(latency),
                            model_available: false,
                            error: Some(err.to_string()),
                        })
                    }
                    Err(_) => Ok(ProviderHealthCheck {
                        reachable: false,
                        latency_ms: None,
                        model_available: false,
                        error: Some("timeout".to_string()),
                    }),
                }
            }
            Err(_) => Ok(ProviderHealthCheck {
                reachable: false,
                latency_ms: None,
                model_available: false,
                error: Some("timeout".to_string()),
            }),
        }
    }
}

#[async_trait]
impl ModelProvider for OpenAiResponsesProvider {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
        let prepared = self.prepare(Uuid::new_v4(), request)?;
        self.stream_prepared(prepared, &mut |_| Ok(()), &mut |_| Ok(()))
            .await
    }

    fn prepare(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        self.prepare_responses_request(request_id, request)
    }

    async fn stream(
        &self,
        request: ModelRequest,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let prepared = self.prepare(Uuid::new_v4(), request)?;
        self.stream_prepared(prepared, on_delta, &mut |_| Ok(()))
            .await
    }

    async fn stream_prepared(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let previous_response_id = prepared.logical_request.previous_response_id.clone();
        match self
            .execute_responses_request(prepared.clone(), 1, on_delta, on_transport)
            .await
        {
            Err(error)
                if previous_response_id.as_deref().is_some_and(|response_id| {
                    error
                        .downcast_ref::<ResponsesRequestError>()
                        .is_some_and(|error| error.invalid_previous_response(response_id))
                }) =>
            {
                let mut replay = prepared.logical_request;
                replay.previous_response_id = None;
                let replay = self.prepare_responses_request(prepared.request_id, replay)?;
                on_transport(ProviderTransportEvent::Retry {
                    attempt: 2,
                    reason: "stored response cursor unavailable; replaying canonical local context"
                        .to_string(),
                    body: replay.observation_body.clone(),
                })?;
                self.execute_responses_request(replay, 2, on_delta, on_transport)
                    .await
            }
            result => result,
        }
    }

    async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
        let start = std::time::Instant::now();
        let models_url = format!("{}/models", self.base_url.trim_end_matches('/'));
        match tokio::time::timeout(
            Duration::from_secs(5),
            self.client
                .get(&models_url)
                .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
                .send(),
        )
        .await
        {
            Ok(Ok(response)) => {
                let reachable = response.status().is_success();
                Ok(ProviderHealthCheck {
                    reachable,
                    latency_ms: Some(start.elapsed().as_millis() as u64),
                    model_available: reachable,
                    error: (!reachable).then(|| format!("HTTP {}", response.status())),
                })
            }
            Ok(Err(error)) => Ok(ProviderHealthCheck {
                reachable: false,
                latency_ms: Some(start.elapsed().as_millis() as u64),
                model_available: false,
                error: Some(error.to_string()),
            }),
            Err(_) => Ok(ProviderHealthCheck {
                reachable: false,
                latency_ms: None,
                model_available: false,
                error: Some("timeout".to_string()),
            }),
        }
    }
}

#[derive(Debug, Default)]
pub struct MockProvider;

#[async_trait]
impl ModelProvider for MockProvider {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
        Ok(ModelResponse::text(format!(
            "OpenTopia MVP mock provider received: {}",
            request.user_message
        )))
    }

    async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
        Ok(ProviderHealthCheck {
            reachable: true,
            latency_ms: None,
            model_available: false,
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    fn model_request() -> ModelRequest {
        ModelRequest {
            system_prompt: "system".to_string(),
            conversation: Vec::new(),
            user_message: "current".to_string(),
            user_content: Vec::new(),
            tool_candidates: Vec::new(),
            previous_tool_calls: Vec::new(),
            tool_results: Vec::new(),
            context_items: Vec::new(),
            previous_response_items: Vec::new(),
            previous_response_id: None,
            branch_developer_instructions: None,
            prompt_cache_key: None,
            final_output_json_schema: None,
        }
    }

    #[test]
    fn model_decision_requires_normal_completion_non_empty_text_and_no_tools() {
        assert_eq!(
            ModelResponse::text("final response").decision(),
            ModelDecision::Final("final response".to_string())
        );

        let empty = ModelResponse::text("   ");
        assert_eq!(
            empty.decision(),
            ModelDecision::Incomplete(IncompleteReason::EmptyResponse)
        );

        let truncated = ModelResponse {
            text: "partial response".to_string(),
            finish_reason: ModelFinishReason::Length,
            ..ModelResponse::text("")
        };
        assert_eq!(
            truncated.decision(),
            ModelDecision::Incomplete(IncompleteReason::OutputTokenLimit)
        );
    }

    #[test]
    fn chat_stream_retains_length_finish_reason() {
        let mut accumulator = OpenAiStreamAccumulator::default();
        accumulator
            .apply(
                &json!({ "choices": [{ "delta": { "content": "partial" } }] }),
                &mut |_| Ok(()),
            )
            .unwrap();
        accumulator
            .apply(
                &json!({ "choices": [{ "delta": {}, "finish_reason": "length" }] }),
                &mut |_| Ok(()),
            )
            .unwrap();

        let response = accumulator.finish().unwrap();
        assert_eq!(response.finish_reason, ModelFinishReason::Length);
        assert_eq!(
            response.decision(),
            ModelDecision::Incomplete(IncompleteReason::OutputTokenLimit)
        );
    }

    #[test]
    fn responses_stream_retains_incomplete_and_interrupted_states() {
        let mut incomplete = ResponsesStreamAccumulator::default();
        incomplete
            .apply(
                &json!({
                    "type": "response.incomplete",
                    "response": {
                        "id": "resp_incomplete",
                        "status": "incomplete",
                        "incomplete_details": { "reason": "max_output_tokens" },
                        "output_text": "partial"
                    }
                }),
                &mut |_| Ok(()),
            )
            .unwrap();
        let incomplete = incomplete.finish().unwrap();
        assert_eq!(incomplete.finish_reason, ModelFinishReason::Length);
        assert_eq!(
            incomplete.decision(),
            ModelDecision::Incomplete(IncompleteReason::OutputTokenLimit)
        );

        let interrupted = ResponsesStreamAccumulator::default().finish().unwrap();
        assert_eq!(
            interrupted.decision(),
            ModelDecision::Incomplete(IncompleteReason::StreamInterrupted)
        );
    }

    fn layered_model_request() -> ModelRequest {
        let mut request = model_request();
        request.system_prompt = "legacy combined system and developer text".to_string();
        request.context_items = vec![
            ModelContextItem::text(
                crate::model_context::ContextItemKind::BaseInstructions,
                ContextRole::System,
                "opentopia:base",
                "base instructions",
                crate::model_context::ContextCacheScope::Stable,
                crate::model_context::ContextSensitivity::Public,
            ),
            ModelContextItem::text(
                crate::model_context::ContextItemKind::Environment,
                ContextRole::Developer,
                "opentopia:environment",
                "developer environment",
                crate::model_context::ContextCacheScope::Turn,
                crate::model_context::ContextSensitivity::Workspace,
            ),
            ModelContextItem::text(
                crate::model_context::ContextItemKind::User,
                ContextRole::User,
                "current_user_message",
                "must not be duplicated",
                crate::model_context::ContextCacheScope::Turn,
                crate::model_context::ContextSensitivity::Workspace,
            ),
        ];
        request
    }

    #[test]
    fn chat_provider_maps_final_output_schema_to_strict_json_schema() {
        let provider =
            OpenAiCompatibleProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
        let mut request = model_request();
        request.final_output_json_schema = Some(json!({
            "type": "object",
            "properties": { "outcome": { "type": "string" } },
            "required": ["outcome"]
        }));
        let prepared = provider.prepare(Uuid::nil(), request).unwrap();
        assert_eq!(prepared.body["response_format"]["type"], "json_schema");
        assert_eq!(
            prepared.body["response_format"]["json_schema"]["strict"],
            true
        );
    }

    #[test]
    fn responses_provider_maps_final_output_schema_to_text_format() {
        let provider =
            OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
        let mut request = model_request();
        request.final_output_json_schema = Some(json!({
            "type": "object",
            "properties": { "outcome": { "type": "string" } },
            "required": ["outcome"]
        }));
        let prepared = provider.prepare(Uuid::nil(), request).unwrap();
        assert_eq!(prepared.body["text"]["format"]["type"], "json_schema");
        assert_eq!(prepared.body["text"]["format"]["strict"], true);
    }

    #[test]
    fn responses_provider_adds_native_web_search_alongside_function_tools() {
        let provider =
            OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test")
                .with_native_web_search(true);
        let mut request = model_request();
        request.tool_candidates.push(ProviderToolCandidate {
            name: "read_file".to_string(),
            description: "Read a workspace file".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        });

        let prepared = provider.prepare(Uuid::nil(), request).unwrap();
        let tools = prepared.body["tools"].as_array().expect("tools array");
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[1], json!({ "type": "web_search" }));
        assert_eq!(prepared.body["tool_choice"], "auto");
    }

    #[test]
    fn parses_openai_chat_tool_calls() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_read",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"Cargo.toml\"}"
                        }
                    }]
                }
            }],
            "usage": {
                "prompt_tokens": 41,
                "completion_tokens": 7,
                "total_tokens": 48,
                "prompt_tokens_details": {
                    "cached_tokens": 12,
                    "cache_write_tokens": 29
                },
                "completion_tokens_details": { "reasoning_tokens": 3 }
            }
        });

        let response = parse_model_response_body(&body).expect("response parses");

        assert_eq!(response.text, "");
        assert_eq!(
            response.tool_calls,
            vec![ProviderToolCall {
                id: "call_read".to_string(),
                name: "read_file".to_string(),
                arguments: json!({ "path": "Cargo.toml" }),
            }]
        );
        assert_eq!(
            response.usage,
            Some(ModelUsage {
                input_tokens: 41,
                output_tokens: 7,
                total_tokens: 48,
                cached_input_tokens: Some(12),
                cache_write_tokens: Some(29),
                reasoning_tokens: Some(3),
            })
        );
    }

    #[test]
    fn parses_responses_function_calls() {
        let body = json!({
            "output_text": "",
            "output": [{
                "type": "function_call",
                "call_id": "call_search",
                "name": "search",
                "arguments": "{\"query\":\"AgentCore\",\"path\":\"crates\"}"
            }]
        });

        let response = parse_model_response_body(&body).expect("response parses");

        assert_eq!(
            response.tool_calls,
            vec![ProviderToolCall {
                id: "call_search".to_string(),
                name: "search".to_string(),
                arguments: json!({ "query": "AgentCore", "path": "crates" }),
            }]
        );
    }

    #[test]
    fn responses_web_search_citations_become_clickable_markdown_links() {
        let body = json!({
            "output": [{
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "OpenTopia source",
                    "annotations": [{
                        "type": "url_citation",
                        "start_index": 9,
                        "end_index": 15,
                        "url": "https://example.test/source",
                        "title": "Example source"
                    }]
                }]
            }]
        });

        let response = parse_model_response_body(&body).expect("response parses");
        assert_eq!(
            response.text,
            "OpenTopia [source](https://example.test/source)"
        );
    }

    #[test]
    fn orders_system_history_current_user_and_current_tool_messages() {
        let mut request = model_request();
        request.conversation = vec![
            ModelConversationMessage {
                role: ModelConversationRole::User,
                content: "earlier user".to_string(),
                content_parts: Vec::new(),
            },
            ModelConversationMessage {
                role: ModelConversationRole::Assistant,
                content: "earlier assistant".to_string(),
                content_parts: Vec::new(),
            },
        ];
        request.previous_tool_calls = vec![ProviderToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: json!({ "path": "Cargo.toml" }),
        }];
        request.tool_results = vec![ProviderToolResult {
            call_id: "call_1".to_string(),
            name: "read_file".to_string(),
            output: "workspace".to_string(),
            content: Vec::new(),
            is_error: false,
            metadata: json!({}),
        }];

        let messages = openai_messages(&request);

        assert_eq!(messages.len(), 6);
        assert_eq!(
            messages[0],
            json!({ "role": "system", "content": "system" })
        );
        assert_eq!(
            messages[1],
            json!({ "role": "user", "content": "earlier user" })
        );
        assert_eq!(
            messages[2],
            json!({ "role": "assistant", "content": "earlier assistant" })
        );
        assert_eq!(messages[3], json!({ "role": "user", "content": "current" }));
        assert_eq!(messages[4]["role"], "assistant");
        assert_eq!(messages[4]["content"], "");
        assert_eq!(messages[4]["tool_calls"][0]["id"], "call_1");
        assert_eq!(messages[5]["role"], "tool");
        assert_eq!(messages[5]["tool_call_id"], "call_1");
    }

    #[test]
    fn chat_messages_preserve_native_context_roles() {
        let request = layered_model_request();

        let messages = openai_messages(&request);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "base instructions");
        assert_eq!(messages[1]["role"], "developer");
        assert!(messages[1]["content"]
            .as_str()
            .unwrap()
            .contains("developer environment"));
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"], "current");
        assert!(!messages
            .iter()
            .any(|message| message.to_string().contains("must not be duplicated")));
    }

    #[test]
    fn responses_split_system_instructions_from_developer_input() {
        let provider =
            OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
        let prepared = provider
            .prepare(Uuid::nil(), layered_model_request())
            .unwrap();

        assert_eq!(prepared.body["instructions"], "base instructions");
        assert_eq!(prepared.body["input"][0]["role"], "developer");
        assert!(prepared.body["input"][0]["content"]
            .as_str()
            .unwrap()
            .contains("developer environment"));
        assert_eq!(prepared.body["input"][1]["role"], "user");
        assert_eq!(prepared.body["input"][1]["content"], "current");
    }

    #[test]
    fn responses_explicit_cache_marks_last_reusable_developer_prefix() {
        let mut provider =
            OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
        provider.prompt_cache_policy = Some(PromptCachePolicy::Explicit30m);
        let mut request = layered_model_request();
        request.context_items.push(ModelContextItem::text(
            crate::model_context::ContextItemKind::RepositoryInstructions,
            ContextRole::Developer,
            "AGENTS.md",
            "stable repository instructions",
            ContextCacheScope::Thread,
            crate::model_context::ContextSensitivity::Workspace,
        ));

        let prepared = provider.prepare(Uuid::nil(), request).unwrap();

        assert_eq!(prepared.body["prompt_cache_options"]["mode"], "explicit");
        assert_eq!(prepared.body["prompt_cache_options"]["ttl"], "30m");
        assert_eq!(
            prepared.body["input"][0]["content"][0]["prompt_cache_breakpoint"]["mode"],
            "explicit"
        );
        assert!(prepared.body["input"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("stable repository instructions"));
        assert_eq!(
            prepared.body["input"][2]["content"][0]["prompt_cache_breakpoint"]["mode"],
            "explicit"
        );
    }

    #[test]
    fn responses_stateful_request_sends_only_incremental_input() {
        let provider =
            OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
        let mut request = layered_model_request();
        request.conversation = vec![ModelConversationMessage {
            role: ModelConversationRole::User,
            content: "already stored".to_string(),
            content_parts: Vec::new(),
        }];
        request.previous_response_id = Some("resp_parent".to_string());

        let prepared = provider.prepare(Uuid::nil(), request).unwrap();

        assert_eq!(prepared.body["previous_response_id"], "resp_parent");
        assert_eq!(prepared.body["input"].as_array().unwrap().len(), 1);
        assert!(!prepared.body["input"]
            .to_string()
            .contains("already stored"));
        assert!(!prepared.body["input"]
            .to_string()
            .contains("developer environment"));
        assert_eq!(prepared.body["input"][0]["content"], "current");
    }

    #[test]
    fn responses_branch_marks_inherited_history_as_a_shared_prefix() {
        let mut provider =
            OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
        provider.prompt_cache_policy = Some(PromptCachePolicy::Explicit30m);
        let mut request = layered_model_request();
        request.conversation = vec![ModelConversationMessage {
            role: ModelConversationRole::User,
            content: "parent fork point".to_string(),
            content_parts: Vec::new(),
        }];
        request.branch_developer_instructions = Some("review this branch".to_string());

        let prepared = provider.prepare(Uuid::nil(), request).unwrap();

        assert_eq!(prepared.body["input"][1]["role"], "user");
        assert_eq!(
            prepared.body["input"][1]["content"][0]["prompt_cache_breakpoint"]["mode"],
            "explicit"
        );
        assert_eq!(prepared.body["input"][2]["role"], "developer");
        assert_eq!(prepared.body["input"][3]["role"], "user");
    }

    #[test]
    fn responses_maps_legacy_cache_retention_and_native_compaction() {
        let mut provider =
            OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
        provider.prompt_cache_policy = Some(PromptCachePolicy::Legacy24h);
        provider.compaction_threshold_tokens = Some(96_000);

        let prepared = provider.prepare(Uuid::nil(), model_request()).unwrap();

        assert_eq!(prepared.body["prompt_cache_retention"], "24h");
        assert!(prepared.body.get("prompt_cache_options").is_none());
        assert_eq!(
            prepared.body["context_management"],
            json!([{"type": "compaction", "compact_threshold": 96_000}])
        );
    }

    #[test]
    fn compatibility_fallback_flattens_developer_context_without_empty_history() {
        let request = layered_model_request();

        assert!(chat_request_has_compatibility_fallback(&request));
        let messages = openai_compatibility_messages(&request);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(
            messages[0]["content"],
            "legacy combined system and developer text"
        );
        assert_eq!(messages[1]["role"], "user");
    }

    #[test]
    fn serializes_native_user_images_and_structured_tool_content() {
        let mut request = model_request();
        request.user_content = vec![
            ModelInputContent::image("image/png", vec![0x89, b'P', b'N', b'G']),
            ModelInputContent::json(json!({ "selection": 4 })),
            ModelInputContent::resource(
                "file:///workspace/spec.pdf",
                Some("application/pdf".to_string()),
                Some("spec.pdf".to_string()),
            ),
        ];
        request.previous_tool_calls = vec![ProviderToolCall {
            id: "call_1".to_string(),
            name: "inspect".to_string(),
            arguments: json!({}),
        }];
        request.tool_results = vec![ProviderToolResult {
            call_id: "call_1".to_string(),
            name: "inspect".to_string(),
            output: "legacy".to_string(),
            content: vec![ModelInputContent::json(json!({ "ready": true }))],
            is_error: false,
            metadata: json!({}),
        }];

        let messages = openai_messages(&request);
        let user = &messages[1];
        assert_eq!(user["role"], "user");
        assert_eq!(
            user["content"][0],
            json!({ "type": "text", "text": "current" })
        );
        assert_eq!(user["content"][1]["type"], "image_url");
        assert_eq!(
            user["content"][1]["image_url"]["url"],
            "data:image/png;base64,iVBORw=="
        );
        assert_eq!(user["content"][2]["text"], "{\"selection\":4}");
        assert!(user["content"][3]["text"]
            .as_str()
            .unwrap()
            .contains("file:///workspace/spec.pdf"));

        let tool_content: Value =
            serde_json::from_str(messages[3]["content"].as_str().unwrap()).unwrap();
        assert_eq!(tool_content["content"][0]["type"], "json");
        assert_eq!(
            tool_content["content"][0]["value"],
            json!({ "ready": true })
        );
    }

    #[test]
    fn compacts_completed_tool_history_for_strict_compatible_providers() {
        let mut request = model_request();
        request.previous_tool_calls = vec![ProviderToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: json!({ "path": "SPEC.md" }),
        }];
        request.tool_results = vec![ProviderToolResult {
            call_id: "call_1".to_string(),
            name: "read_file".to_string(),
            output: "contract".to_string(),
            content: vec![ModelInputContent::text("contract")],
            is_error: false,
            metadata: json!({ "bytes": 8 }),
        }];

        let messages = openai_compatibility_messages(&request);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2]["role"], "user");
        let history = messages[2]["content"].as_str().unwrap();
        assert!(history.contains("read_file"));
        assert!(history.contains("SPEC.md"));
        assert!(history.contains("contract"));
        assert!(!messages.iter().any(|message| message["role"] == "tool"));
    }

    #[test]
    fn appends_native_tool_images_after_all_tool_messages() {
        let mut request = model_request();
        request.previous_tool_calls = vec![
            ProviderToolCall {
                id: "call_first".to_string(),
                name: "browser_screenshot".to_string(),
                arguments: json!({}),
            },
            ProviderToolCall {
                id: "call_second".to_string(),
                name: "inspect_page".to_string(),
                arguments: json!({}),
            },
        ];
        request.tool_results = vec![
            ProviderToolResult {
                call_id: "call_first".to_string(),
                name: "browser_screenshot".to_string(),
                output: "first screenshot".to_string(),
                content: vec![ModelInputContent::image(
                    "image/png",
                    vec![0x89, b'P', b'N', b'G'],
                )],
                is_error: false,
                metadata: json!({}),
            },
            ProviderToolResult {
                call_id: "call_second".to_string(),
                name: "inspect_page".to_string(),
                output: "page inspected".to_string(),
                content: vec![
                    ModelInputContent::json(json!({ "ready": true })),
                    ModelInputContent::image("image/jpeg", vec![0xff, 0xd8, 0xff]),
                ],
                is_error: false,
                metadata: json!({}),
            },
        ];

        let messages = openai_messages(&request);

        assert_eq!(messages.len(), 7);
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[2]["tool_calls"][0]["id"], "call_first");
        assert_eq!(messages[3]["tool_call_id"], "call_first");
        assert_eq!(messages[4]["role"], "assistant");
        assert_eq!(messages[4]["tool_calls"][0]["id"], "call_second");
        assert_eq!(messages[5]["tool_call_id"], "call_second");
        assert_eq!(messages[6]["role"], "user");

        let first_tool_content = messages[3]["content"].as_str().unwrap();
        let second_tool_content = messages[5]["content"].as_str().unwrap();
        assert!(!first_tool_content.contains("data:"));
        assert!(!second_tool_content.contains("data:"));
        let first_tool_content: Value = serde_json::from_str(first_tool_content).unwrap();
        let second_tool_content: Value = serde_json::from_str(second_tool_content).unwrap();
        assert_eq!(
            first_tool_content["content"][0],
            json!({
                "type": "image",
                "contentType": "image/png",
                "bytes": 4,
                "delivery": "native_companion"
            })
        );
        assert_eq!(second_tool_content["content"][0]["type"], "json");
        assert_eq!(
            second_tool_content["content"][1]["delivery"],
            "native_companion"
        );

        let companion = messages[6]["content"].as_array().unwrap();
        assert_eq!(companion.len(), 4);
        assert_eq!(
            companion[0],
            json!({
                "type": "text",
                "text": "Tool image: browser_screenshot (call call_first)"
            })
        );
        assert_eq!(companion[1]["type"], "image_url");
        assert_eq!(
            companion[1]["image_url"]["url"],
            "data:image/png;base64,iVBORw=="
        );
        assert_eq!(
            companion[2]["text"],
            "Tool image: inspect_page (call call_second)"
        );
        assert_eq!(
            companion[3]["image_url"]["url"],
            "data:image/jpeg;base64,/9j/"
        );
    }

    #[test]
    fn base64_encoding_handles_all_padding_cases() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
    }

    #[test]
    fn decodes_sse_across_arbitrary_chunks() {
        let mut decoder = SseDecoder::default();

        assert!(decoder
            .push(b"data: {\"choices\":[{\"del")
            .unwrap()
            .is_empty());
        let events = decoder
            .push(b"ta\":{\"content\":\"hello\"}}]}\r\n\r\ndata: [DO")
            .unwrap();
        assert_eq!(
            events,
            vec![r#"{"choices":[{"delta":{"content":"hello"}}]}"#]
        );
        assert_eq!(decoder.push(b"NE]\n\n").unwrap(), vec!["[DONE]"]);
        assert!(decoder.finish().unwrap().is_empty());
    }

    #[test]
    fn accumulates_streamed_text_tool_arguments_and_usage() {
        let mut accumulator = OpenAiStreamAccumulator::default();
        let mut deltas = Vec::new();
        let mut collect = |delta| {
            deltas.push(delta);
            Ok(())
        };
        accumulator
            .apply(
                &json!({
                    "choices": [{"delta": {
                        "content": "Inspecting ",
                        "tool_calls": [{
                            "index": 0,
                            "id": "call_read",
                            "function": {"name": "read_file", "arguments": "{\"path\":"}
                        }]
                    }}]
                }),
                &mut collect,
            )
            .unwrap();
        accumulator
            .apply(
                &json!({
                    "choices": [{"delta": {
                        "content": "now",
                        "tool_calls": [{
                            "index": 0,
                            "function": {"arguments": "\"src/lib.rs\"}"}
                        }]
                    }}]
                }),
                &mut collect,
            )
            .unwrap();
        accumulator
            .apply(
                &json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 20,
                        "completion_tokens": 5,
                        "total_tokens": 25
                    }
                }),
                &mut collect,
            )
            .unwrap();

        let response = accumulator.finish().unwrap();

        assert_eq!(response.text, "Inspecting now");
        assert_eq!(
            response.tool_calls,
            vec![ProviderToolCall {
                id: "call_read".to_string(),
                name: "read_file".to_string(),
                arguments: json!({ "path": "src/lib.rs" }),
            }]
        );
        assert_eq!(
            response.usage,
            Some(ModelUsage {
                input_tokens: 20,
                output_tokens: 5,
                total_tokens: 25,
                cached_input_tokens: None,
                cache_write_tokens: None,
                reasoning_tokens: None,
            })
        );
        assert_eq!(
            deltas,
            vec![
                ModelStreamDelta::Text {
                    text: "Inspecting ".to_string()
                },
                ModelStreamDelta::ToolCall {
                    index: 0,
                    id: Some("call_read".to_string()),
                    name: Some("read_file".to_string()),
                    arguments_delta: "{\"path\":".to_string(),
                },
                ModelStreamDelta::Text {
                    text: "now".to_string()
                },
                ModelStreamDelta::ToolCall {
                    index: 0,
                    id: None,
                    name: None,
                    arguments_delta: "\"src/lib.rs\"}".to_string(),
                },
                ModelStreamDelta::Usage {
                    usage: ModelUsage {
                        input_tokens: 20,
                        output_tokens: 5,
                        total_tokens: 25,
                        cached_input_tokens: None,
                        cache_write_tokens: None,
                        reasoning_tokens: None,
                    }
                }
            ]
        );
    }

    #[test]
    fn emits_provider_supplied_reasoning_deltas_without_synthesizing_text() {
        let mut accumulator = OpenAiStreamAccumulator::default();
        let mut deltas = Vec::new();
        let mut collect = |delta| {
            deltas.push(delta);
            Ok(())
        };

        accumulator
            .apply(
                &json!({
                    "choices": [{"delta": {
                        "reasoning_content": "检查工作区"
                    }}]
                }),
                &mut collect,
            )
            .unwrap();
        accumulator
            .apply(
                &json!({
                    "choices": [{"delta": {
                        "reasoning": {
                            "summary": [{"type": "summary_text", "text": "并制定计划"}]
                        }
                    }}]
                }),
                &mut collect,
            )
            .unwrap();
        accumulator
            .apply(
                &json!({
                    "choices": [{"delta": {"content": "开始执行"}}]
                }),
                &mut collect,
            )
            .unwrap();

        let response = accumulator.finish().unwrap();
        assert_eq!(response.text, "开始执行");
        assert_eq!(
            deltas,
            vec![
                ModelStreamDelta::Reasoning {
                    text: "检查工作区".to_string(),
                },
                ModelStreamDelta::Reasoning {
                    text: "并制定计划".to_string(),
                },
                ModelStreamDelta::Text {
                    text: "开始执行".to_string(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn default_stream_emits_one_complete_text_delta() {
        let provider = MockProvider;
        let mut deltas = Vec::new();
        let response = provider
            .stream(model_request(), &mut |delta| {
                deltas.push(delta);
                Ok(())
            })
            .await
            .unwrap();

        assert_eq!(deltas.len(), 1);
        assert_eq!(
            deltas[0],
            ModelStreamDelta::Text {
                text: response.text
            }
        );
    }

    #[tokio::test]
    async fn openai_provider_requests_and_collects_real_sse_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            request_tx.send(request).unwrap();
            socket
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/event-stream\r\n",
                        "Connection: close\r\n\r\n",
                        "data: {\"choices\":[{\"delta\":{\"content\":\"hello \"}}]}\n\n",
                        "data: {\"choices\":[{\"delta\":{\"content\":\"world\"}}]}\n\n",
                        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":2,\"total_tokens\":11}}\n\n",
                        "data: [DONE]\n\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.shutdown().await.unwrap();
        });
        let mut provider =
            OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
        provider.temperature = 0.7;
        provider.max_output_tokens = Some(2048);
        provider.reasoning_effort = Some("high".to_string());
        let mut request = model_request();
        request.conversation.push(ModelConversationMessage {
            role: ModelConversationRole::Assistant,
            content: "history".to_string(),
            content_parts: Vec::new(),
        });
        request.tool_candidates.push(ProviderToolCandidate {
            name: "read_file".to_string(),
            description: "Read a workspace file".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        });
        let mut deltas = Vec::new();

        let response = provider
            .stream(request, &mut |delta| {
                deltas.push(delta);
                Ok(())
            })
            .await
            .unwrap();
        server.await.unwrap();
        let raw_request = request_rx.await.unwrap();
        let (_, body) = raw_request.split_once("\r\n\r\n").unwrap();
        let payload: Value = serde_json::from_str(body).unwrap();

        assert_eq!(payload["stream"], true);
        let temperature = payload["temperature"].as_f64().unwrap();
        assert!((temperature - 0.7).abs() < 0.000_001);
        assert_eq!(payload["max_tokens"], 2048);
        assert_eq!(payload["reasoning_effort"], "high");
        assert_eq!(payload["stream_options"]["include_usage"], true);
        assert_eq!(payload["tool_choice"], "auto");
        assert_eq!(payload["parallel_tool_calls"], false);
        assert_eq!(payload["messages"][0]["role"], "system");
        assert_eq!(payload["messages"][1]["content"], "history");
        assert_eq!(payload["messages"][2]["content"], "current");
        assert_eq!(response.text, "hello world");
        assert_eq!(response.usage.unwrap().total_tokens, 11);
        assert_eq!(
            deltas
                .iter()
                .filter(|delta| matches!(delta, ModelStreamDelta::Text { .. }))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn chat_provider_retries_without_developer_role_after_http_400() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut first_socket, _) = listener.accept().await.unwrap();
            let first_request = read_http_request(&mut first_socket).await;
            let rejected = r#"{"error":"unsupported developer role"}"#;
            first_socket
                .write_all(
                    format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        rejected.len(),
                        rejected
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            first_socket.shutdown().await.unwrap();

            let (mut second_socket, _) = listener.accept().await.unwrap();
            let second_request = read_http_request(&mut second_socket).await;
            second_socket
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/event-stream\r\n",
                        "Connection: close\r\n\r\n",
                        "data: {\"choices\":[{\"delta\":{\"content\":\"compatible\"}}]}\n\n",
                        "data: [DONE]\n\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            second_socket.shutdown().await.unwrap();
            request_tx.send((first_request, second_request)).unwrap();
        });
        let provider =
            OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
        let prepared = provider
            .prepare(Uuid::nil(), layered_model_request())
            .unwrap();
        let mut transport = Vec::new();

        let response = provider
            .stream_prepared(prepared, &mut |_| Ok(()), &mut |event| {
                transport.push(event);
                Ok(())
            })
            .await
            .unwrap();
        server.await.unwrap();
        let (first_request, second_request) = request_rx.await.unwrap();
        let first: Value =
            serde_json::from_str(first_request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let second: Value =
            serde_json::from_str(second_request.split_once("\r\n\r\n").unwrap().1).unwrap();

        assert_eq!(response.text, "compatible");
        assert!(first["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "developer"));
        assert!(!second["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "developer"));
        assert_eq!(
            second["messages"][0]["content"],
            "legacy combined system and developer text"
        );
        assert!(transport
            .iter()
            .any(|event| matches!(event, ProviderTransportEvent::Retry { attempt: 2, .. })));
    }

    #[tokio::test]
    async fn chat_provider_retries_without_unsupported_response_format_after_http_400() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut first_socket, _) = listener.accept().await.unwrap();
            let first_request = read_http_request(&mut first_socket).await;
            let rejected = r#"{"error":"unsupported response_format"}"#;
            first_socket
                .write_all(
                    format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        rejected.len(),
                        rejected
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            first_socket.shutdown().await.unwrap();

            let (mut second_socket, _) = listener.accept().await.unwrap();
            let second_request = read_http_request(&mut second_socket).await;
            second_socket
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/event-stream\r\n",
                        "Connection: close\r\n\r\n",
                        "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"outcome\\\":\\\"allow\\\"}\"}}]}\n\n",
                        "data: [DONE]\n\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            second_socket.shutdown().await.unwrap();
            request_tx.send((first_request, second_request)).unwrap();
        });
        let provider =
            OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
        let mut request = model_request();
        request.final_output_json_schema = Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "outcome": { "type": "string" } },
            "required": ["outcome"]
        }));
        let prepared = provider.prepare(Uuid::nil(), request).unwrap();
        let mut transport = Vec::new();

        let response = provider
            .stream_prepared(prepared, &mut |_| Ok(()), &mut |event| {
                transport.push(event);
                Ok(())
            })
            .await
            .unwrap();
        server.await.unwrap();
        let (first_request, second_request) = request_rx.await.unwrap();
        let first: Value =
            serde_json::from_str(first_request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let second: Value =
            serde_json::from_str(second_request.split_once("\r\n\r\n").unwrap().1).unwrap();

        assert_eq!(response.text, r#"{"outcome":"allow"}"#);
        assert!(first.get("response_format").is_some());
        assert!(second.get("response_format").is_none());
        assert_eq!(first["messages"], second["messages"]);
        assert!(transport
            .iter()
            .any(|event| matches!(event, ProviderTransportEvent::Retry { attempt: 2, .. })));
    }

    #[test]
    fn responses_input_replays_typed_items_and_correlates_tool_outputs() {
        let mut request = model_request();
        request.previous_response_items = vec![
            json!({
                "type": "reasoning",
                "encrypted_content": "opaque",
                "summary": []
            }),
            json!({
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "read_file",
                "arguments": "{\"path\":\"Cargo.toml\"}"
            }),
        ];
        request.previous_tool_calls = vec![ProviderToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: json!({ "path": "Cargo.toml" }),
        }];
        request.tool_results = vec![ProviderToolResult {
            call_id: "call_1".to_string(),
            name: "read_file".to_string(),
            output: "workspace".to_string(),
            content: Vec::new(),
            is_error: false,
            metadata: json!({}),
        }];

        let input = responses_input(&request);

        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_1");
        assert_eq!(
            input
                .iter()
                .filter(|item| item.get("type") == Some(&json!("function_call")))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn responses_provider_prepares_redacted_body_and_collects_typed_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            request_tx.send(request).unwrap();
            socket
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/event-stream\r\n",
                        "Connection: close\r\n\r\n",
                        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_123\"}}\n\n",
                        "data: {\"type\":\"response.output_item.added\",\"response_id\":\"resp_123\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"read_file\",\"arguments\":\"\"}}\n\n",
                        "data: {\"type\":\"response.function_call_arguments.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"delta\":\"{\\\"path\\\":\\\"Cargo.toml\\\"}\"}\n\n",
                        "data: {\"type\":\"response.output_item.done\",\"response_id\":\"resp_123\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"Cargo.toml\\\"}\"}}\n\n",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_123\",\"output\":[{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"Cargo.toml\\\"}\"}],\"usage\":{\"input_tokens\":20,\"output_tokens\":5,\"total_tokens\":25,\"input_tokens_details\":{\"cached_tokens\":12,\"cache_write_tokens\":8}}}}\n\n",
                        "data: [DONE]\n\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.shutdown().await.unwrap();
        });
        let provider =
            OpenAiResponsesProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
        let mut request = model_request();
        request.user_content = vec![ModelInputContent::image("image/png", vec![1, 2, 3])];
        request.prompt_cache_key = Some("workspace-cache".to_string());
        request.tool_candidates = vec![ProviderToolCandidate {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        }];
        let prepared = provider.prepare(Uuid::nil(), request).unwrap();
        assert_eq!(prepared.adapter, "openai_responses");
        assert_eq!(
            prepared.observation_body["prompt_cache_key"],
            "workspace-cache"
        );
        assert!(prepared
            .observation_body
            .to_string()
            .contains("data URL omitted"));
        assert!(!prepared.observation_body.to_string().contains("AQID"));
        let mut transport = Vec::new();
        let response = provider
            .stream_prepared(prepared, &mut |_| Ok(()), &mut |event| {
                transport.push(event);
                Ok(())
            })
            .await
            .unwrap();
        server.await.unwrap();
        let raw_request = request_rx.await.unwrap();
        let (_, body) = raw_request.split_once("\r\n\r\n").unwrap();
        let payload: Value = serde_json::from_str(body).unwrap();

        assert_eq!(payload["stream"], true);
        assert_eq!(payload["store"], false);
        assert_eq!(payload["tools"][0]["name"], "read_file");
        assert!(payload["tools"][0].get("function").is_none());
        assert_eq!(payload["input"][0]["content"][1]["type"], "input_image");
        assert_eq!(response.response_id.as_deref(), Some("resp_123"));
        assert_eq!(response.tool_calls[0].id, "call_1");
        assert_eq!(
            response.tool_calls[0].arguments,
            json!({ "path": "Cargo.toml" })
        );
        assert_eq!(response.provider_items.len(), 1);
        let usage = response.usage.unwrap();
        assert_eq!(usage.total_tokens, 25);
        assert_eq!(usage.cached_input_tokens, Some(12));
        assert_eq!(usage.cache_write_tokens, Some(8));
        assert!(matches!(
            transport.as_slice(),
            [ProviderTransportEvent::Response {
                status: Some(200),
                response_id: Some(response_id),
                ..
            }] if response_id == "resp_123"
        ));
    }

    #[tokio::test]
    async fn responses_provider_replays_local_context_when_state_cursor_is_missing() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut first_socket, _) = listener.accept().await.unwrap();
            let first_request = read_http_request(&mut first_socket).await;
            let rejected = r#"{"error":{"message":"previous_response_id resp_missing was not found","param":"previous_response_id"}}"#;
            first_socket
                .write_all(
                    format!(
                        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        rejected.len(),
                        rejected
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            first_socket.shutdown().await.unwrap();

            let (mut second_socket, _) = listener.accept().await.unwrap();
            let second_request = read_http_request(&mut second_socket).await;
            second_socket
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/event-stream\r\n",
                        "Connection: close\r\n\r\n",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_replayed\",\"output_text\":\"replayed locally\",\"output\":[]}}\n\n",
                        "data: [DONE]\n\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            second_socket.shutdown().await.unwrap();
            request_tx.send((first_request, second_request)).unwrap();
        });
        let mut provider =
            OpenAiResponsesProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
        provider.store_responses = true;
        let mut request = layered_model_request();
        request.conversation = vec![ModelConversationMessage {
            role: ModelConversationRole::User,
            content: "canonical local history".to_string(),
            content_parts: Vec::new(),
        }];
        request.previous_response_id = Some("resp_missing".to_string());
        let prepared = provider.prepare(Uuid::nil(), request).unwrap();
        let mut transport = Vec::new();

        let response = provider
            .stream_prepared(prepared, &mut |_| Ok(()), &mut |event| {
                transport.push(event);
                Ok(())
            })
            .await
            .unwrap();
        server.await.unwrap();
        let (first_request, second_request) = request_rx.await.unwrap();
        let first: Value =
            serde_json::from_str(first_request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let second: Value =
            serde_json::from_str(second_request.split_once("\r\n\r\n").unwrap().1).unwrap();

        assert_eq!(response.text, "replayed locally");
        assert_eq!(response.response_id.as_deref(), Some("resp_replayed"));
        assert_eq!(first["previous_response_id"], "resp_missing");
        assert_eq!(first["input"].as_array().unwrap().len(), 1);
        assert!(second.get("previous_response_id").is_none());
        assert!(second["input"]
            .to_string()
            .contains("canonical local history"));
        assert!(transport
            .iter()
            .any(|event| matches!(event, ProviderTransportEvent::Retry { attempt: 2, .. })));
    }

    async fn read_http_request(socket: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = socket.read(&mut buffer).await.unwrap();
            assert!(read > 0, "client closed before sending a complete request");
            bytes.extend_from_slice(&buffer[..read]);
            let Some(headers_end) = find_bytes(&bytes, b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&bytes[..headers_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= headers_end + 4 + content_length {
                return String::from_utf8(bytes).unwrap();
            }
        }
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }
}
