//! HTTP-backed browser runtime for an Electron-owned, user-visible browser session.
//!
//! The broker is deliberately restricted to a loopback HTTP endpoint. This keeps browser
//! credentials and automation commands off the network while allowing the Rust agent runtime and
//! Electron's `WebContentsView` to operate on the same session.

use crate::{
    BrowserAction, BrowserActionReceipt, BrowserBackendKind, BrowserDownloadRequest, BrowserError,
    BrowserNavigateRequest, BrowserNetworkGrant, BrowserNode, BrowserNodeRef, BrowserObservation,
    BrowserObservationId, BrowserObserveOptions, BrowserOutput, BrowserRuntime,
    BrowserRuntimeCapabilities, BrowserSessionId, BrowserSessionInfo, BrowserSessionSpec,
    BrowserSurfaceKind, BrowserTargetRef, BrowserWaitCondition, BrowserWaitRequest,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_LENGTH};
use reqwest::{redirect, Client, Response, Url};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(35);
const DEFAULT_MAX_RESPONSE_BYTES: usize = 12 * 1024 * 1024;
const MAX_ERROR_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_ERROR_MESSAGE_CHARS: usize = 512;

#[derive(Debug, Clone)]
pub struct DesktopBrowserRuntimeConfig {
    pub connect_timeout: Duration,
    pub health_timeout: Duration,
    pub request_timeout: Duration,
    pub max_response_bytes: usize,
}

impl Default for DesktopBrowserRuntimeConfig {
    fn default() -> Self {
        Self {
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            health_timeout: DEFAULT_HEALTH_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }
}

#[derive(Clone)]
pub struct DesktopBrowserRuntime {
    base_url: Url,
    authorization: HeaderValue,
    token: Arc<str>,
    client: Client,
    config: Arc<DesktopBrowserRuntimeConfig>,
}

impl DesktopBrowserRuntime {
    pub fn new(base_url: &str, token: &str) -> Result<Self, BrowserError> {
        Self::with_config(base_url, token, DesktopBrowserRuntimeConfig::default())
    }

    pub fn with_config(
        base_url: &str,
        token: &str,
        config: DesktopBrowserRuntimeConfig,
    ) -> Result<Self, BrowserError> {
        if config.connect_timeout.is_zero()
            || config.health_timeout.is_zero()
            || config.request_timeout.is_zero()
            || config.max_response_bytes == 0
        {
            return Err(BrowserError::BrokerConfiguration(
                "timeouts and response limit must be greater than zero".to_string(),
            ));
        }

        let base_url = validate_and_normalize_base_url(base_url)?;
        let token = token.trim();
        if token.is_empty() {
            return Err(BrowserError::BrokerConfiguration(
                "broker token is missing".to_string(),
            ));
        }

        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
                BrowserError::BrokerConfiguration("broker token is invalid".to_string())
            })?;
        authorization.set_sensitive(true);

        let client = Client::builder()
            .no_proxy()
            .redirect(redirect::Policy::none())
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .build()
            .map_err(|_| {
                BrowserError::BrokerConfiguration(
                    "failed to initialize the broker HTTP client".to_string(),
                )
            })?;

        Ok(Self {
            base_url,
            authorization,
            token: Arc::from(token),
            client,
            config: Arc::new(config),
        })
    }

    pub async fn health_check(&self) -> Result<(), BrowserError> {
        let endpoint = self.endpoint("health")?;
        let request = self
            .client
            .get(endpoint)
            .header(AUTHORIZATION, self.authorization.clone());
        let response = tokio::time::timeout(self.config.health_timeout, request.send())
            .await
            .map_err(|_| BrowserError::Timeout("desktop browser broker health check".to_string()))?
            .map_err(map_transport_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(self.rejected_error(response).await);
        }

        read_response_limited(response, 4 * 1024).await?;
        Ok(())
    }

    async fn execute<T: DeserializeOwned>(
        &self,
        request: impl Serialize,
    ) -> Result<T, BrowserError> {
        let endpoint = self.endpoint("v1/browser")?;
        let request = self
            .client
            .post(endpoint)
            .header(AUTHORIZATION, self.authorization.clone())
            .json(&request);
        let response = tokio::time::timeout(self.config.request_timeout, request.send())
            .await
            .map_err(|_| BrowserError::Timeout("desktop browser broker response".to_string()))?
            .map_err(map_transport_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(self.rejected_error(response).await);
        }

        let bytes = read_response_limited(response, self.config.max_response_bytes).await?;
        serde_json::from_slice(&bytes).map_err(|_| {
            BrowserError::Protocol(
                "Desktop browser broker returned an invalid BrowserOutput response".to_string(),
            )
        })
    }

    async fn rejected_error(&self, response: Response) -> BrowserError {
        let status = response.status().as_u16();
        let limit = self.config.max_response_bytes.min(MAX_ERROR_RESPONSE_BYTES);
        let body = match read_response_limited(response, limit).await {
            Ok(body) => body,
            Err(BrowserError::BrokerResponseTooLarge { .. }) => {
                return BrowserError::BrokerRejected {
                    status,
                    message: "broker error response exceeded the configured limit".to_string(),
                };
            }
            Err(_) => {
                return BrowserError::BrokerRejected {
                    status,
                    message: "broker returned an unreadable error response".to_string(),
                };
            }
        };
        if let Some(reason) = stale_observation_reason(&body, &self.token) {
            return BrowserError::StaleObservation { reason };
        }
        if let Some(host) = network_blocked_host(&body) {
            return BrowserError::NetworkBlocked { host };
        }
        let message = sanitize_error_message(&body, &self.token);
        BrowserError::BrokerRejected { status, message }
    }

    fn endpoint(&self, path: &str) -> Result<Url, BrowserError> {
        self.base_url.join(path).map_err(|_| {
            BrowserError::BrokerConfiguration("broker endpoint path is invalid".to_string())
        })
    }
}

#[async_trait]
impl BrowserRuntime for DesktopBrowserRuntime {
    fn capabilities(&self) -> BrowserRuntimeCapabilities {
        BrowserRuntimeCapabilities::managed(
            BrowserBackendKind::Electron,
            BrowserSurfaceKind::Embedded,
            true,
        )
    }

    async fn create_session(
        &self,
        spec: BrowserSessionSpec,
    ) -> Result<BrowserSessionInfo, BrowserError> {
        self.execute(BrokerCreateSessionRequest {
            session_id: spec.session_id,
            action: "create_session",
            profile_id: spec.profile_id,
            profile_persistence: spec.profile_persistence,
        })
        .await
    }

    async fn grant_network_access(
        &self,
        session: BrowserSessionId,
        grant: BrowserNetworkGrant,
    ) -> Result<(), BrowserError> {
        self.execute::<BrowserOutput>(BrokerNetworkGrantRequest {
            session_id: session,
            action: "grant_network_access",
            allowed_hosts: grant.allowed_hosts,
        })
        .await?;
        Ok(())
    }

    async fn navigate(
        &self,
        session: BrowserSessionId,
        request: BrowserNavigateRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        let (selector, text, wait) = split_wait(request.wait);
        self.execute(BrokerRequest {
            session_id: session,
            action: BrokerAction::Navigate,
            url: Some(request.url),
            selector,
            text,
            value: None,
            wait,
            observation_id: None,
            node_ref: None,
            operation: None,
            clear_first: None,
            delta_x: None,
            delta_y: None,
            include_screenshot: None,
            target_ref: None,
        })
        .await
    }

    async fn observe(
        &self,
        session: BrowserSessionId,
        options: BrowserObserveOptions,
    ) -> Result<BrowserObservation, BrowserError> {
        self.execute(BrokerRequest {
            session_id: session,
            action: BrokerAction::Observe,
            url: None,
            selector: None,
            text: None,
            value: None,
            wait: None,
            observation_id: None,
            node_ref: None,
            operation: None,
            clear_first: None,
            delta_x: None,
            delta_y: None,
            include_screenshot: Some(options.include_screenshot),
            target_ref: None,
        })
        .await
    }

    async fn switch_target(
        &self,
        session: BrowserSessionId,
        target: BrowserTargetRef,
    ) -> Result<BrowserOutput, BrowserError> {
        let mut request = BrokerRequest::new(session, BrokerAction::SwitchTarget);
        request.target_ref = Some(target);
        self.execute(request).await
    }

    async fn observation_node(
        &self,
        session: BrowserSessionId,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
    ) -> Result<BrowserNode, BrowserError> {
        self.execute(BrokerRequest {
            session_id: session,
            action: BrokerAction::ObservationNode,
            url: None,
            selector: None,
            text: None,
            value: None,
            wait: None,
            observation_id: Some(observation_id),
            node_ref: Some(node_ref),
            operation: None,
            clear_first: None,
            delta_x: None,
            delta_y: None,
            include_screenshot: None,
            target_ref: None,
        })
        .await
    }

    async fn perform(
        &self,
        session: BrowserSessionId,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
        action: BrowserAction,
    ) -> Result<BrowserActionReceipt, BrowserError> {
        let (operation, text, value, clear_first, delta_x, delta_y) = match action {
            BrowserAction::Click => (BrokerOperation::Click, None, None, None, None, None),
            BrowserAction::Type { text, clear_first } => (
                BrokerOperation::Type,
                Some(text),
                None,
                Some(clear_first),
                None,
                None,
            ),
            BrowserAction::Select { value } => {
                (BrokerOperation::Select, None, Some(value), None, None, None)
            }
            BrowserAction::Hover => (BrokerOperation::Hover, None, None, None, None, None),
            BrowserAction::Scroll { delta_x, delta_y } => (
                BrokerOperation::Scroll,
                None,
                None,
                None,
                Some(delta_x),
                Some(delta_y),
            ),
        };
        self.execute(BrokerRequest {
            session_id: session,
            action: BrokerAction::Perform,
            url: None,
            selector: None,
            text,
            value,
            wait: None,
            observation_id: Some(observation_id),
            node_ref: Some(node_ref),
            operation: Some(operation),
            clear_first,
            delta_x,
            delta_y,
            include_screenshot: None,
            target_ref: None,
        })
        .await
    }

    async fn screenshot(&self, session: BrowserSessionId) -> Result<BrowserOutput, BrowserError> {
        self.execute(BrokerRequest::new(session, BrokerAction::Screenshot))
            .await
    }

    async fn wait(
        &self,
        session: BrowserSessionId,
        request: BrowserWaitRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        let (selector, text, wait) = split_wait(Some(request));
        self.execute(BrokerRequest {
            session_id: session,
            action: BrokerAction::Wait,
            url: None,
            selector,
            text,
            value: None,
            wait,
            observation_id: None,
            node_ref: None,
            operation: None,
            clear_first: None,
            delta_x: None,
            delta_y: None,
            include_screenshot: None,
            target_ref: None,
        })
        .await
    }

    async fn download(
        &self,
        session: BrowserSessionId,
        request: BrowserDownloadRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        self.execute(BrokerRequest {
            session_id: session,
            action: BrokerAction::Download,
            url: Some(request.url),
            selector: None,
            text: None,
            value: None,
            wait: request.timeout.map(|timeout| BrokerWait {
                condition: None,
                timeout_ms: Some(duration_millis(timeout)),
                poll_interval_ms: None,
            }),
            observation_id: None,
            node_ref: None,
            operation: None,
            clear_first: None,
            delta_x: None,
            delta_y: None,
            include_screenshot: None,
            target_ref: None,
        })
        .await
    }

    async fn close_session(&self, session: BrowserSessionId) -> Result<(), BrowserError> {
        self.execute::<BrowserOutput>(BrokerRequest::new(session, BrokerAction::Close))
            .await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum BrokerAction {
    Navigate,
    Observe,
    ObservationNode,
    SwitchTarget,
    Screenshot,
    Perform,
    Wait,
    Download,
    Close,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum BrokerOperation {
    Click,
    Type,
    Select,
    Hover,
    Scroll,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrokerCreateSessionRequest {
    session_id: BrowserSessionId,
    action: &'static str,
    profile_id: crate::BrowserProfileId,
    profile_persistence: crate::BrowserProfilePersistence,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrokerRequest {
    session_id: BrowserSessionId,
    action: BrokerAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait: Option<BrokerWait>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observation_id: Option<BrowserObservationId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_ref: Option<BrowserNodeRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation: Option<BrokerOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clear_first: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delta_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delta_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include_screenshot: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_ref: Option<BrowserTargetRef>,
}

impl BrokerRequest {
    fn new(session_id: BrowserSessionId, action: BrokerAction) -> Self {
        Self {
            session_id,
            action,
            url: None,
            selector: None,
            text: None,
            value: None,
            wait: None,
            observation_id: None,
            node_ref: None,
            operation: None,
            clear_first: None,
            delta_x: None,
            delta_y: None,
            include_screenshot: None,
            target_ref: None,
        }
    }
}

#[derive(Serialize)]
struct BrokerWait {
    #[serde(skip_serializing_if = "Option::is_none")]
    condition: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    poll_interval_ms: Option<u64>,
}

fn split_wait(
    request: Option<BrowserWaitRequest>,
) -> (Option<String>, Option<String>, Option<BrokerWait>) {
    let Some(request) = request else {
        return (None, None, None);
    };
    let (condition, selector, text) = match request.condition {
        BrowserWaitCondition::DocumentComplete => ("document_complete", None, None),
        BrowserWaitCondition::Selector(selector) => {
            ("selector", Some(selector.as_str().to_string()), None)
        }
        BrowserWaitCondition::Text(text) => ("text", None, Some(text)),
    };
    (
        selector,
        text,
        Some(BrokerWait {
            condition: Some(condition),
            timeout_ms: request.timeout.map(duration_millis),
            poll_interval_ms: Some(duration_millis(request.poll_interval)),
        }),
    )
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn validate_and_normalize_base_url(raw: &str) -> Result<Url, BrowserError> {
    let mut url = Url::parse(raw.trim()).map_err(|_| {
        BrowserError::BrokerConfiguration("broker URL is not a valid URL".to_string())
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(BrowserError::BrokerConfiguration(
            "broker URL must use HTTP or HTTPS".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(BrowserError::BrokerConfiguration(
            "broker URL must not contain credentials".to_string(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(BrowserError::BrokerConfiguration(
            "broker URL must not contain a query or fragment".to_string(),
        ));
    }
    let host = url.host_str().ok_or_else(|| {
        BrowserError::BrokerConfiguration("broker URL is missing a host".to_string())
    })?;
    let host_without_ipv6_brackets = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    let loopback = host_without_ipv6_brackets
        .parse::<IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false);
    if !loopback {
        return Err(BrowserError::BrokerConfiguration(
            "broker URL host must be a numeric loopback address".to_string(),
        ));
    }

    let normalized_path = format!("{}/", url.path().trim_end_matches('/'));
    url.set_path(&normalized_path);
    Ok(url)
}

fn map_transport_error(error: reqwest::Error) -> BrowserError {
    if error.is_timeout() {
        BrowserError::Timeout("desktop browser broker response".to_string())
    } else {
        BrowserError::BrokerUnavailable
    }
}

async fn read_response_limited(
    response: Response,
    maximum: usize,
) -> Result<Vec<u8>, BrowserError> {
    if let Some(length) = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        if length > maximum as u64 {
            return Err(BrowserError::BrokerResponseTooLarge {
                actual: usize::try_from(length).unwrap_or(usize::MAX),
                maximum,
            });
        }
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(map_transport_error)?;
        let actual = body.len().saturating_add(chunk.len());
        if actual > maximum {
            return Err(BrowserError::BrokerResponseTooLarge { actual, maximum });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn sanitize_error_message(body: &[u8], token: &str) -> String {
    let extracted = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .or_else(|| value.pointer("/error/message").and_then(Value::as_str))
                .or_else(|| value.get("message").and_then(Value::as_str))
                .map(str::to_string)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());
    let redacted = if token.is_empty() {
        extracted
    } else {
        extracted.replace(token, "[redacted]")
    };
    let message = redacted
        .chars()
        .filter(|character| !character.is_control() || character.is_whitespace())
        .take(MAX_ERROR_MESSAGE_CHARS)
        .collect::<String>();
    let message = message.trim();
    if message.is_empty() {
        "broker rejected the request".to_string()
    } else {
        message.to_string()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrokerNetworkGrantRequest {
    session_id: BrowserSessionId,
    action: &'static str,
    allowed_hosts: Vec<String>,
}

fn stale_observation_reason(body: &[u8], token: &str) -> Option<String> {
    let value = serde_json::from_slice::<Value>(body).ok()?;
    let code = value.pointer("/error/code").and_then(Value::as_str)?;
    if code != "stale_observation" {
        return None;
    }
    Some(sanitize_error_message(body, token))
}

fn network_blocked_host(body: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(body).ok()?;
    if value.pointer("/error/code").and_then(Value::as_str)? != "network_host_blocked" {
        return None;
    }
    let host = value.pointer("/error/host").and_then(Value::as_str)?;
    BrowserNetworkGrant::new([host])
        .ok()?
        .allowed_hosts
        .into_iter()
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrowserProfileId, BrowserProfilePersistence, BrowserSelector};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;

    #[derive(Debug)]
    struct CapturedRequest {
        request_line: String,
        headers: String,
        body: Vec<u8>,
    }

    async fn spawn_broker(
        status: u16,
        body: Vec<u8>,
        delay: Duration,
    ) -> (String, oneshot::Receiver<CapturedRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let captured = read_request(&mut socket).await;
            let _ = request_tx.send(captured);
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let reason = if status == 200 { "OK" } else { "Error" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
            let _ = socket.shutdown().await;
        });
        (format!("http://{address}"), request_rx)
    }

    async fn read_request(socket: &mut TcpStream) -> CapturedRequest {
        let mut bytes = Vec::new();
        let header_end = loop {
            let mut buffer = [0_u8; 1024];
            let read = socket.read(&mut buffer).await.unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let mut buffer = [0_u8; 1024];
            let read = socket.read(&mut buffer).await.unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&buffer[..read]);
        }
        CapturedRequest {
            request_line: headers.lines().next().unwrap().to_string(),
            headers,
            body: bytes[header_end..header_end + content_length].to_vec(),
        }
    }

    fn output_body() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "url": "https://example.com/",
            "contents": [{ "type": "text", "text": "ready", "truncated": false }],
            "metadata": { "visible": true }
        }))
        .unwrap()
    }

    #[test]
    fn broker_url_must_be_loopback_and_free_of_credentials() {
        assert!(DesktopBrowserRuntime::new("http://127.0.0.1:3100", "token").is_ok());
        assert!(DesktopBrowserRuntime::new("http://[::1]:3100/base", "token").is_ok());
        assert!(matches!(
            DesktopBrowserRuntime::new("http://localhost:3100", "token"),
            Err(BrowserError::BrokerConfiguration(_))
        ));
        assert!(matches!(
            DesktopBrowserRuntime::new("https://example.com", "token"),
            Err(BrowserError::BrokerConfiguration(_))
        ));
        assert!(matches!(
            DesktopBrowserRuntime::new("http://user:secret@127.0.0.1:3100", "token"),
            Err(BrowserError::BrokerConfiguration(_))
        ));
    }

    #[tokio::test]
    async fn explicit_session_creation_binds_a_validated_profile() {
        let session = BrowserSessionId::new();
        let profile = BrowserProfileId::new("release-smoke").unwrap();
        let body = serde_json::to_vec(&json!({
            "sessionId": session,
            "profileId": profile,
            "profilePersistence": "ephemeral",
            "backend": "electron"
        }))
        .unwrap();
        let (base_url, captured) = spawn_broker(200, body, Duration::ZERO).await;
        let runtime = DesktopBrowserRuntime::new(&base_url, "test-token").unwrap();
        let spec = BrowserSessionSpec {
            session_id: session,
            profile_id: BrowserProfileId::new("release-smoke").unwrap(),
            profile_persistence: BrowserProfilePersistence::Ephemeral,
        };

        let info = runtime.create_session(spec).await.unwrap();
        assert_eq!(info.backend, BrowserBackendKind::Electron);
        assert_eq!(info.profile_id.as_str(), "release-smoke");
        let payload: Value = serde_json::from_slice(&captured.await.unwrap().body).unwrap();
        assert_eq!(payload["action"], "create_session");
        assert_eq!(payload["profileId"], "release-smoke");
        assert_eq!(payload["profilePersistence"], "ephemeral");

        let capabilities = runtime.capabilities();
        assert_eq!(capabilities.surface, BrowserSurfaceKind::Embedded);
        assert!(capabilities.supports_user_handoff);
        assert!(capabilities.hard_network_isolation);
    }

    #[tokio::test]
    async fn navigate_sends_bearer_auth_and_serializes_the_broker_contract() {
        let (base_url, captured) = spawn_broker(200, output_body(), Duration::ZERO).await;
        let runtime = DesktopBrowserRuntime::new(&base_url, "test-broker-token").unwrap();
        let session = BrowserSessionId::new();

        let output = runtime
            .navigate(
                session,
                BrowserNavigateRequest {
                    url: "https://example.com/".to_string(),
                    wait: Some(BrowserWaitRequest {
                        condition: BrowserWaitCondition::Selector(
                            BrowserSelector::new("main").unwrap(),
                        ),
                        timeout: Some(Duration::from_millis(1_500)),
                        poll_interval: Duration::from_millis(50),
                    }),
                },
            )
            .await
            .unwrap();
        assert_eq!(output.url.as_deref(), Some("https://example.com/"));

        let captured = captured.await.unwrap();
        assert_eq!(captured.request_line, "POST /v1/browser HTTP/1.1");
        assert!(captured
            .headers
            .to_ascii_lowercase()
            .contains("authorization: bearer test-broker-token"));
        let payload: Value = serde_json::from_slice(&captured.body).unwrap();
        assert_eq!(payload["sessionId"], session.to_string());
        assert_eq!(payload["action"], "navigate");
        assert_eq!(payload["url"], "https://example.com/");
        assert_eq!(payload["selector"], "main");
        assert_eq!(payload["wait"]["condition"], "selector");
        assert_eq!(payload["wait"]["timeout_ms"], 1_500);
        assert_eq!(payload["wait"]["poll_interval_ms"], 50);
    }

    #[tokio::test]
    async fn semantic_actions_serialize_without_exposing_internal_locators() {
        let observation_id = BrowserObservationId::new();
        let node_ref: BrowserNodeRef =
            serde_json::from_value(json!(BrowserObservationId::new().to_string())).unwrap();
        let body = serde_json::to_vec(&json!({
            "observationId": observation_id,
            "nodeRef": node_ref,
            "action": "scroll",
            "target": {
                "nodeRef": node_ref,
                "role": "button",
                "name": "More",
                "tagName": "button",
                "bounds": { "x": 0, "y": 0, "width": 10, "height": 10 },
                "href": null,
                "formAction": null,
                "editable": false
            },
            "url": "https://example.com/",
            "title": "Example"
        }))
        .unwrap();
        let (base_url, captured) = spawn_broker(200, body, Duration::ZERO).await;
        let runtime = DesktopBrowserRuntime::new(&base_url, "test-token").unwrap();

        runtime
            .perform(
                BrowserSessionId::new(),
                observation_id,
                node_ref,
                BrowserAction::Scroll {
                    delta_x: 12.0,
                    delta_y: 640.0,
                },
            )
            .await
            .unwrap();

        let payload: Value = serde_json::from_slice(&captured.await.unwrap().body).unwrap();
        assert_eq!(payload["action"], "perform");
        assert_eq!(payload["operation"], "scroll");
        assert_eq!(payload["deltaX"], 12.0);
        assert_eq!(payload["deltaY"], 640.0);
        assert!(payload.get("selector").is_none());
    }

    #[tokio::test]
    async fn target_switching_uses_an_opaque_target_reference() {
        let (base_url, captured) = spawn_broker(200, output_body(), Duration::ZERO).await;
        let runtime = DesktopBrowserRuntime::new(&base_url, "test-token").unwrap();
        let target: BrowserTargetRef = serde_json::from_value(json!("owned-target")).unwrap();

        runtime
            .switch_target(BrowserSessionId::new(), target)
            .await
            .unwrap();

        let payload: Value = serde_json::from_slice(&captured.await.unwrap().body).unwrap();
        assert_eq!(payload["action"], "switch_target");
        assert_eq!(payload["targetRef"], "owned-target");
    }

    #[tokio::test]
    async fn health_check_uses_the_health_endpoint_and_authentication() {
        let (base_url, captured) = spawn_broker(200, b"{}".to_vec(), Duration::ZERO).await;
        let runtime =
            DesktopBrowserRuntime::new(&format!("{base_url}/broker"), "health-token").unwrap();

        runtime.health_check().await.unwrap();

        let captured = captured.await.unwrap();
        assert_eq!(captured.request_line, "GET /broker/health HTTP/1.1");
        assert!(captured
            .headers
            .to_ascii_lowercase()
            .contains("authorization: bearer health-token"));
    }

    #[tokio::test]
    async fn broker_errors_are_typed_limited_and_token_redacted() {
        let token = "never-expose-this-token";
        let body = serde_json::to_vec(&json!({
            "error": {
                "code": "unauthorized",
                "message": format!("authorization failed for {token}")
            }
        }))
        .unwrap();
        let (base_url, _) = spawn_broker(403, body, Duration::ZERO).await;
        let runtime = DesktopBrowserRuntime::new(&base_url, token).unwrap();

        let error = runtime
            .observe(BrowserSessionId::new(), BrowserObserveOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            BrowserError::BrokerRejected { status: 403, ref message }
                if message.contains("[redacted]") && !message.contains(token)
        ));
        assert!(!error.to_string().contains(token));
    }

    #[tokio::test]
    async fn network_policy_rejections_preserve_the_blocked_host() {
        let body = serde_json::to_vec(&json!({
            "error": {
                "code": "network_host_blocked",
                "message": "The browser blocked an unauthorized network request.",
                "host": "STATIC.Example.COM."
            }
        }))
        .unwrap();
        let (base_url, _) = spawn_broker(403, body, Duration::ZERO).await;
        let runtime = DesktopBrowserRuntime::new(&base_url, "test-token").unwrap();

        let error = runtime
            .observe(BrowserSessionId::new(), BrowserObserveOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            BrowserError::NetworkBlocked { ref host } if host == "static.example.com"
        ));
    }

    #[tokio::test]
    async fn network_grants_use_the_same_authenticated_broker_contract() {
        let (base_url, captured) = spawn_broker(200, output_body(), Duration::ZERO).await;
        let runtime = DesktopBrowserRuntime::new(&base_url, "test-broker-token").unwrap();
        let session = BrowserSessionId::new();

        runtime
            .grant_network_access(
                session,
                BrowserNetworkGrant::new(["example.com", "static.example.com"]).unwrap(),
            )
            .await
            .unwrap();

        let captured = captured.await.unwrap();
        let payload: Value = serde_json::from_slice(&captured.body).unwrap();
        assert_eq!(payload["sessionId"], session.to_string());
        assert_eq!(payload["action"], "grant_network_access");
        assert_eq!(
            payload["allowedHosts"],
            json!(["example.com", "static.example.com"])
        );
    }

    #[tokio::test]
    async fn stale_broker_observations_keep_the_reobserve_signal() {
        let body = serde_json::to_vec(&json!({
            "error": {
                "code": "stale_observation",
                "message": "The observed element changed or moved."
            }
        }))
        .unwrap();
        let (base_url, _) = spawn_broker(409, body, Duration::ZERO).await;
        let runtime = DesktopBrowserRuntime::new(&base_url, "test-token").unwrap();

        let error = runtime
            .observe(BrowserSessionId::new(), BrowserObserveOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            BrowserError::StaleObservation { ref reason }
                if reason == "The observed element changed or moved."
        ));
    }

    #[tokio::test]
    async fn response_content_length_is_rejected_before_allocation() {
        let (base_url, _) = spawn_broker(200, vec![b'x'; 512], Duration::ZERO).await;
        let config = DesktopBrowserRuntimeConfig {
            max_response_bytes: 128,
            ..DesktopBrowserRuntimeConfig::default()
        };
        let runtime = DesktopBrowserRuntime::with_config(&base_url, "token", config).unwrap();

        assert!(matches!(
            runtime
                .observe(BrowserSessionId::new(), BrowserObserveOptions::default())
                .await,
            Err(BrowserError::BrokerResponseTooLarge {
                actual: 512,
                maximum: 128
            })
        ));
    }

    #[tokio::test]
    async fn request_timeout_is_enforced() {
        let (base_url, _) = spawn_broker(200, output_body(), Duration::from_millis(200)).await;
        let config = DesktopBrowserRuntimeConfig {
            request_timeout: Duration::from_millis(40),
            ..DesktopBrowserRuntimeConfig::default()
        };
        let runtime = DesktopBrowserRuntime::with_config(&base_url, "token", config).unwrap();

        assert!(matches!(
            runtime
                .observe(BrowserSessionId::new(), BrowserObserveOptions::default())
                .await,
            Err(BrowserError::Timeout(_))
        ));
    }
}
