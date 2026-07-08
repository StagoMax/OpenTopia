use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequest {
    pub system_prompt: String,
    pub user_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResponse {
    pub text: String,
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse>;
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
        let api_key = std::env::var("OPENTOPIA_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .ok()?;
        let base_url = std::env::var("OPENTOPIA_OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
        let model = std::env::var("OPENTOPIA_MODEL").unwrap_or_else(|_| "gpt-4.1-mini".to_string());
        Some(Self::new(base_url, api_key, model))
    }
}

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({
                "model": self.model,
                "temperature": 0.2,
                "messages": [
                    { "role": "system", "content": request.system_prompt },
                    { "role": "user", "content": request.user_message }
                ]
            }))
            .send()
            .await?;

        let status = response.status();
        let body: serde_json::Value = response.json().await?;
        if !status.is_success() {
            anyhow::bail!("provider request failed ({status}): {body}");
        }

        let text = body
            .pointer("/choices/0/message/content")
            .and_then(serde_json::Value::as_str)
            .or_else(|| body.pointer("/output_text").and_then(serde_json::Value::as_str))
            .unwrap_or("")
            .to_string();

        if text.is_empty() {
            anyhow::bail!("provider returned an empty response: {body}");
        }

        Ok(ModelResponse { text })
    }
}

#[derive(Debug, Default)]
pub struct MockProvider;

#[async_trait]
impl ModelProvider for MockProvider {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
        Ok(ModelResponse {
            text: format!(
                "OpenTopia MVP mock provider received: {}",
                request.user_message
            ),
        })
    }
}
