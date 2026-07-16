use crate::model::ModelContentPart;
use crate::settings::{ProviderHealthCheck, ProviderKind, ProviderSettings};
use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResponse {
    pub text: String,
    #[serde(default)]
    pub tool_calls: Vec<ProviderToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ModelUsage>,
}

impl ModelResponse {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tool_calls: Vec::new(),
            usage: None,
        }
    }
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
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ModelStreamDelta {
    Text {
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
        }
    }

    pub fn from_env() -> Option<Self> {
        let env = ProviderEnv::load();
        let api_key = env.first([
            "OPENTOPIA_API_KEY",
            "CREDIT_REVIEW_LLM_API_KEY",
            "AUDIT_COPILOT_LLM_API_KEY",
            "OPENAI_API_KEY",
        ])?;
        let base_url = env
            .first([
                "OPENTOPIA_OPENAI_BASE_URL",
                "CREDIT_REVIEW_LLM_BASE_URL",
                "AUDIT_COPILOT_LLM_BASE_URL",
                "OPENAI_BASE_URL",
            ])
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let model = env
            .first([
                "OPENTOPIA_MODEL",
                "CREDIT_REVIEW_LLM_MODEL",
                "AUDIT_COPILOT_LLM_MODEL",
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
                ProviderEnv::load().first([
                    "OPENTOPIA_API_KEY",
                    "CREDIT_REVIEW_LLM_API_KEY",
                    "AUDIT_COPILOT_LLM_API_KEY",
                    "OPENAI_API_KEY",
                ])
            })?;
        Some(Self::new(
            settings.base_url.clone(),
            api_key,
            settings.model.clone(),
        ))
    }

    async fn stream_completion(
        &self,
        request: ModelRequest,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let mut payload = json!({
            "model": self.model,
            "temperature": 0.2,
            "messages": openai_messages(&request),
            "stream": true,
            "stream_options": { "include_usage": true }
        });
        if !request.tool_candidates.is_empty() {
            payload["tools"] = json!(openai_tools(&request.tool_candidates));
            payload["tool_choice"] = json!("auto");
            payload["parallel_tool_calls"] = json!(false);
        }

        let mut response = self
            .client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await?;
        if response.status().as_u16() == 400 && !request.tool_results.is_empty() {
            let rejected_body = response.text().await?;
            payload["messages"] = json!(openai_compatibility_messages(&request));
            let retry = self
                .client
                .post(&url)
                .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send()
                .await?;
            if !retry.status().is_success() {
                let retry_status = retry.status();
                let retry_body = retry.text().await?;
                anyhow::bail!(
                    "provider request failed (400): {rejected_body}; compatibility retry failed ({retry_status}): {retry_body}"
                );
            }
            response = retry;
        }
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await?;
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
        if response.text.is_empty() && response.tool_calls.is_empty() {
            anyhow::bail!("provider returned an empty streaming response");
        }
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

        let Some(delta) = event.pointer("/choices/0/delta") else {
            return Ok(());
        };
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
        })
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

#[derive(Debug, Default)]
struct ProviderEnv {
    values: HashMap<String, String>,
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

fn openai_messages(request: &ModelRequest) -> Vec<Value> {
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
    messages.push(json!({
        "role": "user",
        "content": format!(
            "Continue the original task using this authoritative completed tool history. Do not repeat completed calls unless needed:\n{}",
            Value::Array(history)
        )
    }));
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

#[cfg(test)]
fn parse_model_response_body(body: &Value) -> anyhow::Result<ModelResponse> {
    Ok(ModelResponse {
        text: extract_response_text(body),
        tool_calls: extract_provider_tool_calls(body)?,
        usage: parse_model_usage(body.get("usage")),
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
        reasoning_tokens,
    })
}

#[cfg(test)]
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

    body.pointer("/output_text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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
        self.stream_completion(request, &mut |_| Ok(())).await
    }

    async fn stream(
        &self,
        request: ModelRequest,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        self.stream_completion(request, on_delta).await
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
        }
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
                "prompt_tokens_details": { "cached_tokens": 12 },
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
                        reasoning_tokens: None,
                    }
                }
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
        let provider =
            OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
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
