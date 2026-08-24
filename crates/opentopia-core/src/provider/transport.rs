use super::{
    provider_error_is_quota_exhausted, truncate_observation_text, ModelStreamDelta,
    ProviderAdapterError, ProviderTransportCallback, ProviderTransportEvent,
};
use crate::model::{ProviderCacheTrace, ProviderRetryKind};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
pub(super) const PROVIDER_NETWORK_RETRY_LIMIT: usize = 5;
pub(super) const PROVIDER_RATE_LIMIT_RETRY_LIMIT: usize = PROVIDER_NETWORK_RETRY_LIMIT;
const PROVIDER_NETWORK_RETRY_DELAYS: [Duration; PROVIDER_NETWORK_RETRY_LIMIT] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];
const PROVIDER_RATE_LIMIT_RETRY_DELAYS: [Duration; PROVIDER_NETWORK_RETRY_LIMIT] = [
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
    Duration::from_secs(30),
];

#[derive(Clone, Copy)]
struct ProviderRequestStartedAt(Instant);

#[derive(Debug, thiserror::Error)]
pub(super) enum ProviderStallError {
    #[error("provider request stalled before response headers for {seconds} seconds")]
    BeforeResponseHeaders { seconds: u64 },
    #[error(
        "provider stream stalled after response headers: no output within {seconds} seconds of request start"
    )]
    BeforeFirstOutput { seconds: u64 },
    #[error("provider stream stalled: no data for {seconds} seconds")]
    Idle { seconds: u64 },
}

pub(super) fn provider_stream_stalled(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ProviderStallError>().is_some()
}

fn retryable_provider_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
        || (status.is_server_error() && status != reqwest::StatusCode::NOT_IMPLEMENTED)
}

fn provider_retry_delay(response: &reqwest::Response, retry_index: usize) -> Duration {
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| Duration::from_secs(seconds.min(60)));
    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        provider_rate_limit_retry_delay(retry_index, retry_after)
    } else {
        retry_after.unwrap_or(PROVIDER_NETWORK_RETRY_DELAYS[retry_index - 1])
    }
}

pub(super) fn provider_rate_limit_retry_delay(
    retry_index: usize,
    retry_after: Option<Duration>,
) -> Duration {
    retry_after.unwrap_or_else(|| {
        PROVIDER_RATE_LIMIT_RETRY_DELAYS[retry_index
            .saturating_sub(1)
            .min(PROVIDER_RATE_LIMIT_RETRY_DELAYS.len() - 1)]
    })
}

fn retryable_provider_network_error(error: &reqwest::Error) -> bool {
    !error.is_builder() && (error.is_connect() || error.is_timeout() || error.is_request())
}

pub(super) async fn send_provider_request_with_network_retries<F>(
    mut request: F,
    first_attempt: usize,
    observation_body: &Value,
    cache_trace: Option<&ProviderCacheTrace>,
    on_transport: &mut ProviderTransportCallback<'_>,
) -> anyhow::Result<(reqwest::Response, usize)>
where
    F: FnMut() -> reqwest::RequestBuilder,
{
    let mut attempt = first_attempt;
    let mut retry_index = 0;
    loop {
        let attempt_started_at = Instant::now();
        let send_result = match tokio::time::timeout(first_output_timeout(), request().send()).await
        {
            Ok(Ok(mut response)) => {
                response
                    .extensions_mut()
                    .insert(ProviderRequestStartedAt(attempt_started_at));
                Ok(response)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => {
                return Err(ProviderStallError::BeforeResponseHeaders {
                    seconds: first_output_timeout().as_secs(),
                }
                .into())
            }
        };
        match send_result {
            Ok(response)
                if retryable_provider_status(response.status())
                    && retry_index < PROVIDER_NETWORK_RETRY_LIMIT =>
            {
                let response_attempt = attempt;
                retry_index += 1;
                attempt += 1;
                let status = response.status();
                let delay = provider_retry_delay(&response, retry_index);
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    let body = response.text().await?;
                    if provider_error_is_quota_exhausted(&body) {
                        on_transport(ProviderTransportEvent::Response {
                            attempt: response_attempt,
                            status: Some(status.as_u16()),
                            response_id: None,
                            body: json!({ "error": truncate_observation_text(&body) }),
                        })?;
                        return Err(ProviderAdapterError::QuotaExhausted { detail: body }.into());
                    }
                }
                let reason = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    format!(
                        "provider rate limited the request; retrying after {} second(s)",
                        delay.as_secs()
                    )
                } else {
                    format!("provider returned transient HTTP {status}; reconnecting")
                };
                on_transport(ProviderTransportEvent::Retry {
                    attempt,
                    retry_kind: ProviderRetryKind::Network,
                    retry_index: Some(retry_index),
                    retry_limit: Some(PROVIDER_NETWORK_RETRY_LIMIT),
                    reason,
                    cache_trace: cache_trace.cloned(),
                    body: observation_body.clone(),
                })?;
                tokio::time::sleep(delay).await;
            }
            Ok(response) => return Ok((response, attempt)),
            Err(error)
                if retryable_provider_network_error(&error)
                    && retry_index < PROVIDER_NETWORK_RETRY_LIMIT =>
            {
                retry_index += 1;
                attempt += 1;
                on_transport(ProviderTransportEvent::Retry {
                    attempt,
                    retry_kind: ProviderRetryKind::Network,
                    retry_index: Some(retry_index),
                    retry_limit: Some(PROVIDER_NETWORK_RETRY_LIMIT),
                    reason: format!("provider connection failed: {error}"),
                    cache_trace: cache_trace.cloned(),
                    body: observation_body.clone(),
                })?;
                tokio::time::sleep(PROVIDER_NETWORK_RETRY_DELAYS[retry_index - 1]).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

/// Default ceiling on the gap between two streamed chunks of one model response.
///
/// A total request timeout is the wrong shape for streaming: it truncates a
/// legitimately long answer while still letting a dead connection hold the turn
/// open for its full duration. Bounding the idle gap instead lets a response run
/// as long as it needs to as long as it keeps producing output.
///
/// The bound is generous because silence is not always failure. A reasoning model
/// can think for minutes before its first token, and an OpenAI-compatible endpoint
/// is not obliged to send keep-alive events while it does.
const DEFAULT_STREAM_IDLE_TIMEOUT_SECS: u64 = 300;

/// Maximum time from starting an HTTP model request to its first normalized
/// output. Response headers and transport heartbeats do not reset this deadline.
const DEFAULT_FIRST_OUTPUT_TIMEOUT_SECS: u64 = 120;

/// Default ceiling on the gap between two Codex App Server events.
///
/// This stream carries a whole turn rather than one model response, so the gaps
/// include tool execution: a build, an install, or a download can legitimately run
/// for many minutes without producing an event. The bound has to clear that, and it
/// still only measures silence, never total turn length.
const DEFAULT_APP_SERVER_IDLE_TIMEOUT_SECS: u64 = 900;

fn env_timeout_secs(key: &str, default_secs: u64) -> Duration {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map_or(Duration::from_secs(default_secs), Duration::from_secs)
}

pub(super) fn stream_idle_timeout() -> Duration {
    env_timeout_secs(
        "OPENTOPIA_STREAM_IDLE_TIMEOUT_SECS",
        DEFAULT_STREAM_IDLE_TIMEOUT_SECS,
    )
}

fn first_output_timeout() -> Duration {
    env_timeout_secs(
        "OPENTOPIA_FIRST_OUTPUT_TIMEOUT_SECS",
        DEFAULT_FIRST_OUTPUT_TIMEOUT_SECS,
    )
}

pub(super) fn app_server_idle_timeout() -> Duration {
    env_timeout_secs(
        "OPENTOPIA_APP_SERVER_IDLE_TIMEOUT_SECS",
        DEFAULT_APP_SERVER_IDLE_TIMEOUT_SECS,
    )
}

pub(super) struct ProviderStreamWatchdog {
    idle_timeout: Duration,
    first_output_timeout: Duration,
    first_output_deadline: Instant,
    output_observed: bool,
}

impl ProviderStreamWatchdog {
    pub(super) fn new(response: &reqwest::Response) -> Self {
        Self::with_timeouts_and_start(
            stream_idle_timeout(),
            first_output_timeout(),
            response
                .extensions()
                .get::<ProviderRequestStartedAt>()
                .map(|started| started.0)
                .unwrap_or_else(Instant::now),
        )
    }

    #[cfg(test)]
    fn with_timeouts(idle_timeout: Duration, first_output_timeout: Duration) -> Self {
        Self::with_timeouts_and_start(idle_timeout, first_output_timeout, Instant::now())
    }

    fn with_timeouts_and_start(
        idle_timeout: Duration,
        first_output_timeout: Duration,
        request_started_at: Instant,
    ) -> Self {
        Self {
            idle_timeout,
            first_output_timeout,
            first_output_deadline: request_started_at + first_output_timeout,
            output_observed: false,
        }
    }

    pub(super) fn observe(&mut self, delta: &ModelStreamDelta) {
        self.output_observed |= delta.contains_output_token();
    }

    pub(super) async fn next_chunk(
        &self,
        response: &mut reqwest::Response,
    ) -> anyhow::Result<Option<impl std::ops::Deref<Target = [u8]>>> {
        let wait = self.next_wait()?;
        match tokio::time::timeout(wait, response.chunk()).await {
            Ok(chunk) => Ok(chunk?),
            Err(_) if !self.output_observed => Err(ProviderStallError::BeforeFirstOutput {
                seconds: self.first_output_timeout.as_secs(),
            }
            .into()),
            Err(_) => Err(ProviderStallError::Idle {
                seconds: self.idle_timeout.as_secs(),
            }
            .into()),
        }
    }

    fn next_wait(&self) -> anyhow::Result<Duration> {
        if self.output_observed {
            return Ok(self.idle_timeout);
        }
        let remaining = self
            .first_output_deadline
            .saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ProviderStallError::BeforeFirstOutput {
                seconds: self.first_output_timeout.as_secs(),
            }
            .into());
        }
        Ok(self.idle_timeout.min(remaining))
    }
}

#[derive(Debug, Default)]
pub(super) struct SseDecoder {
    buffer: Vec<u8>,
    data_lines: Vec<String>,
}

impl SseDecoder {
    pub(super) fn push(&mut self, chunk: &[u8]) -> anyhow::Result<Vec<String>> {
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

    pub(super) fn finish(&mut self) -> anyhow::Result<Vec<String>> {
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

#[cfg(test)]
mod watchdog_tests {
    use super::*;

    #[test]
    fn first_output_deadline_is_not_extended_by_transport_activity() {
        let watchdog = ProviderStreamWatchdog::with_timeouts(
            Duration::from_secs(300),
            Duration::from_secs(180),
        );
        let wait = watchdog.next_wait().expect("first output wait");
        assert!(wait <= Duration::from_secs(180));
        assert!(wait > Duration::from_secs(179));
    }

    #[test]
    fn response_header_wait_consumes_the_first_output_budget() {
        let watchdog = ProviderStreamWatchdog::with_timeouts_and_start(
            Duration::from_secs(300),
            Duration::from_secs(120),
            Instant::now() - Duration::from_secs(121),
        );
        let error = watchdog.next_wait().expect_err("deadline must expire");
        assert!(provider_stream_stalled(&error));
    }

    #[test]
    fn output_switches_the_watchdog_to_the_idle_gap_timeout() {
        let mut watchdog = ProviderStreamWatchdog::with_timeouts(
            Duration::from_secs(300),
            Duration::from_secs(180),
        );
        watchdog.observe(&ModelStreamDelta::Text {
            text: "ready".to_string(),
        });
        assert_eq!(watchdog.next_wait().unwrap(), Duration::from_secs(300));
    }
}
