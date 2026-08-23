use super::super::apply_provider_auth;
use super::OpenAiCompatibleProvider;
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

const PROBE_RATE_LIMIT_RETRIES: usize = 2;
const PROBE_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

/// One connection test performs several independent feature probes. They share
/// this client so independent probes start concurrently on capable endpoints.
/// The first rate-limit response switches the rest of this short-lived probe
/// session to a single serial lane, including the failed request's retry.
#[derive(Clone)]
pub(in crate::provider) struct OpenAiProbeClient {
    provider: OpenAiCompatibleProvider,
    serial_fallback: Arc<AtomicBool>,
    serial_lane: Arc<Semaphore>,
    cooldown_until: Arc<Mutex<Instant>>,
}

pub(in crate::provider) struct OpenAiProbeResponse {
    response: reqwest::Response,
    permit: Option<OwnedSemaphorePermit>,
}

impl OpenAiProbeResponse {
    pub(in crate::provider) fn into_parts(
        self,
    ) -> (reqwest::Response, Option<OwnedSemaphorePermit>) {
        (self.response, self.permit)
    }
}

impl OpenAiProbeClient {
    pub(in crate::provider) fn new(provider: &OpenAiCompatibleProvider) -> Self {
        Self {
            provider: provider.clone(),
            serial_fallback: Arc::new(AtomicBool::new(false)),
            serial_lane: Arc::new(Semaphore::new(1)),
            cooldown_until: Arc::new(Mutex::new(Instant::now())),
        }
    }

    pub(in crate::provider) async fn send(
        &self,
        path: &str,
        payload: &Value,
    ) -> Result<OpenAiProbeResponse, String> {
        let url = format!("{}{}", self.provider.base_url.trim_end_matches('/'), path);
        let mut rate_limit_retry = 0;
        loop {
            self.wait_for_cooldown().await;
            let permit = self.acquire_serial_fallback_permit().await?;
            let response = tokio::time::timeout(
                PROBE_REQUEST_TIMEOUT,
                apply_provider_auth(
                    self.provider.client.post(&url),
                    self.provider.auth,
                    &self.provider.api_key,
                )
                .header(CONTENT_TYPE, "application/json")
                .json(payload)
                .send(),
            )
            .await
            .map_err(|_| "request timed out after 20 seconds".to_string())?
            .map_err(|error| error.to_string())?;

            if response.status() != reqwest::StatusCode::TOO_MANY_REQUESTS
                || rate_limit_retry >= PROBE_RATE_LIMIT_RETRIES
            {
                return Ok(OpenAiProbeResponse { response, permit });
            }

            rate_limit_retry += 1;
            let delay = retry_after_delay(&response, rate_limit_retry);
            self.activate_serial_fallback(delay).await;
            // Consume the body before reusing the connection. Diagnostics for
            // the final attempt remain available to the caller.
            let _ = response.text().await;
            drop(permit);
        }
    }

    pub(in crate::provider) async fn schedule_embedded_rate_limit_retry(
        &self,
        retry_index: usize,
        retry_after: Option<Duration>,
    ) -> bool {
        if retry_index > PROBE_RATE_LIMIT_RETRIES {
            return false;
        }
        let delay = retry_after
            .map(|delay| delay.min(PROBE_MAX_RETRY_AFTER))
            .unwrap_or_else(|| Duration::from_secs(1_u64 << retry_index.min(4)));
        self.activate_serial_fallback(delay).await;
        true
    }

    async fn acquire_serial_fallback_permit(&self) -> Result<Option<OwnedSemaphorePermit>, String> {
        if !self.serial_fallback.load(Ordering::Acquire) {
            return Ok(None);
        }
        Arc::clone(&self.serial_lane)
            .acquire_owned()
            .await
            .map(Some)
            .map_err(|_| "compatibility probe serial fallback gate closed".to_string())
    }

    async fn wait_for_cooldown(&self) {
        loop {
            let deadline = *self.cooldown_until.lock().await;
            let now = Instant::now();
            if deadline <= now {
                return;
            }
            tokio::time::sleep_until(deadline).await;
        }
    }

    async fn activate_serial_fallback(&self, delay: Duration) {
        self.serial_fallback.store(true, Ordering::Release);
        let target = Instant::now() + delay;
        let mut cooldown_until = self.cooldown_until.lock().await;
        if target > *cooldown_until {
            *cooldown_until = target;
        }
    }
}

fn retry_after_delay(response: &reqwest::Response, retry_index: usize) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .map(|delay| delay.min(PROBE_MAX_RETRY_AFTER))
        .unwrap_or_else(|| Duration::from_secs(1_u64 << retry_index.min(4)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ProviderFeatureSupport;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn runs_independent_probes_concurrently_before_rate_limiting() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let active_requests = Arc::new(AtomicUsize::new(0));
        let peak_requests = Arc::new(AtomicUsize::new(0));
        let server_active_requests = Arc::clone(&active_requests);
        let server_peak_requests = Arc::clone(&peak_requests);
        let server = tokio::spawn(async move {
            let mut handlers = Vec::new();
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let active_requests = Arc::clone(&server_active_requests);
                let peak_requests = Arc::clone(&server_peak_requests);
                handlers.push(tokio::spawn(async move {
                    let mut buffer = [0_u8; 2_048];
                    let _ = socket.read(&mut buffer).await.unwrap();
                    let active = active_requests.fetch_add(1, Ordering::SeqCst) + 1;
                    peak_requests.fetch_max(active, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                        )
                        .await
                        .unwrap();
                    active_requests.fetch_sub(1, Ordering::SeqCst);
                }));
            }
            for handler in handlers {
                handler.await.unwrap();
            }
            server_peak_requests.load(Ordering::SeqCst)
        });

        let provider = OpenAiCompatibleProvider::new(
            format!("http://{address}/v1"),
            "test-key",
            "probe-model",
        );
        let client = OpenAiProbeClient::new(&provider);
        let send_probe = |client: OpenAiProbeClient| async move {
            let payload = serde_json::json!({ "model": "probe-model" });
            let response = client.send("/chat/completions", &payload).await.unwrap();
            let (response, permit) = response.into_parts();
            let _ = response.text().await.unwrap();
            drop(permit);
        };
        tokio::join!(
            send_probe(client.clone()),
            send_probe(client.clone()),
            send_probe(client),
        );

        assert_eq!(server.await.unwrap(), 3);
    }

    #[tokio::test]
    async fn rate_limit_switches_retries_and_remaining_probes_to_serial() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let retry_active = Arc::new(AtomicUsize::new(0));
        let retry_peak = Arc::new(AtomicUsize::new(0));
        let server_retry_active = Arc::clone(&retry_active);
        let server_retry_peak = Arc::clone(&retry_peak);
        let server = tokio::spawn(async move {
            let mut initial_sockets = Vec::new();
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0_u8; 2_048];
                let _ = socket.read(&mut buffer).await.unwrap();
                initial_sockets.push(socket);
            }
            for mut socket in initial_sockets {
                socket
                    .write_all(
                        b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .await
                    .unwrap();
            }

            let mut retry_handlers = Vec::new();
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let active = Arc::clone(&server_retry_active);
                let peak = Arc::clone(&server_retry_peak);
                retry_handlers.push(tokio::spawn(async move {
                    let mut buffer = [0_u8; 2_048];
                    let _ = socket.read(&mut buffer).await.unwrap();
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                        )
                        .await
                        .unwrap();
                    active.fetch_sub(1, Ordering::SeqCst);
                }));
            }
            for handler in retry_handlers {
                handler.await.unwrap();
            }
            server_retry_peak.load(Ordering::SeqCst)
        });

        let provider = OpenAiCompatibleProvider::new(
            format!("http://{address}/v1"),
            "test-key",
            "probe-model",
        );
        let client = OpenAiProbeClient::new(&provider);
        let send_probe = |client: OpenAiProbeClient| async move {
            let payload = serde_json::json!({ "model": "probe-model" });
            let response = client.send("/chat/completions", &payload).await.unwrap();
            let (response, permit) = response.into_parts();
            let _ = response.text().await.unwrap();
            drop(permit);
        };
        tokio::join!(
            send_probe(client.clone()),
            send_probe(client.clone()),
            send_probe(client),
        );

        assert_eq!(server.await.unwrap(), 1);
    }

    #[tokio::test]
    async fn embedded_sse_rate_limit_retries_the_probe_in_serial_mode() {
        const TOKEN: &str = "opentopia-tool-probe-v1";
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for attempt in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0_u8; 4_096];
                let size = socket.read(&mut buffer).await.unwrap();
                requests.push(String::from_utf8_lossy(&buffer[..size]).into_owned());
                let body = if attempt == 0 {
                    "data: {\"error\":{\"type\":\"rate_limit_error\",\"message\":\"Concurrency limit exceeded\",\"retry_after\":0}}\n\n"
                } else {
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"probe_call\",\"function\":{\"name\":\"compatibility_probe\",\"arguments\":\"{\\\"token\\\":\\\"opentopia-tool-probe-v1\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n"
                };
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
            requests
        });

        let provider = OpenAiCompatibleProvider::new(
            format!("http://{address}/v1"),
            "test-key",
            "probe-model",
        );
        let client = OpenAiProbeClient::new(&provider);
        let payload = serde_json::json!({
            "model": "probe-model",
            "messages": [{"role": "user", "content": "Call the probe."}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "compatibility_probe",
                    "parameters": {
                        "type": "object",
                        "properties": {"token": {"type": "string"}},
                        "required": ["token"]
                    }
                }
            }],
            "stream": true
        });
        let outcome = provider
            .probe_openai_function_tool_roundtrip(&client, "/chat/completions", payload, TOKEN)
            .await;
        let requests = server.await.unwrap();

        assert_eq!(outcome.support, ProviderFeatureSupport::Supported);
        assert_eq!(requests.len(), 2);
        assert!(requests
            .iter()
            .all(|request| request.contains(r#""stream":true"#)));
        assert!(client.serial_fallback.load(Ordering::Acquire));
    }
}
