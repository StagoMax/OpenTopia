use axum::http::{HeaderMap, HeaderName, HeaderValue};
use chrono::Utc;
use std::time::Instant;
use uuid::Uuid;

pub(crate) const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-opentopia-request-id");
pub(crate) const CLIENT_STARTED_AT_HEADER: HeaderName =
    HeaderName::from_static("x-opentopia-client-started-at-ms");
pub(crate) const SERVER_DURATION_HEADER: HeaderName =
    HeaderName::from_static("x-opentopia-server-duration-ms");
pub(crate) const CLIENT_TO_SERVER_HEADER: HeaderName =
    HeaderName::from_static("x-opentopia-client-to-server-ms");

#[derive(Clone, Copy)]
pub(crate) struct ConversationSendTrace {
    request_id: Uuid,
    started_at: Instant,
    client_to_server_ms: Option<u64>,
}

impl ConversationSendTrace {
    pub(crate) fn from_headers(headers: &HeaderMap) -> Self {
        let request_id = headers
            .get(&REQUEST_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| Uuid::parse_str(value).ok())
            .unwrap_or_else(Uuid::new_v4);
        let client_to_server_ms = headers
            .get(&CLIENT_STARTED_AT_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok())
            .and_then(|started_at| Utc::now().timestamp_millis().checked_sub(started_at))
            .filter(|elapsed| (0..=86_400_000).contains(elapsed))
            .map(|elapsed| elapsed as u64);
        Self {
            request_id,
            started_at: Instant::now(),
            client_to_server_ms,
        }
    }

    pub(crate) fn phase(&self, phase: &'static str, thread_id: Uuid, turn_id: Option<Uuid>) {
        tracing::info!(
            request_id = %self.request_id,
            %thread_id,
            ?turn_id,
            phase,
            elapsed_ms = self.elapsed_ms(),
            client_to_server_ms = ?self.client_to_server_ms,
            "conversation send trace"
        );
    }

    pub(crate) fn phase_with_count(
        &self,
        phase: &'static str,
        thread_id: Uuid,
        turn_id: Option<Uuid>,
        item_count: usize,
    ) {
        tracing::info!(
            request_id = %self.request_id,
            %thread_id,
            ?turn_id,
            phase,
            elapsed_ms = self.elapsed_ms(),
            client_to_server_ms = ?self.client_to_server_ms,
            item_count,
            "conversation send trace"
        );
    }

    pub(crate) fn apply_response_headers(&self, headers: &mut HeaderMap) {
        headers.insert(
            REQUEST_ID_HEADER,
            HeaderValue::from_str(&self.request_id.to_string())
                .expect("request IDs are valid header values"),
        );
        headers.insert(
            SERVER_DURATION_HEADER,
            HeaderValue::from_str(&self.elapsed_ms().to_string())
                .expect("server durations are valid header values"),
        );
        if let Some(client_to_server_ms) = self.client_to_server_ms {
            headers.insert(
                CLIENT_TO_SERVER_HEADER,
                HeaderValue::from_str(&client_to_server_ms.to_string())
                    .expect("client-to-server durations are valid header values"),
            );
        }
    }

    fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_valid_request_ids_and_adds_timing_headers() {
        let request_id = Uuid::new_v4();
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            REQUEST_ID_HEADER,
            HeaderValue::from_str(&request_id.to_string()).unwrap(),
        );
        request_headers.insert(
            CLIENT_STARTED_AT_HEADER,
            HeaderValue::from_str(&(Utc::now().timestamp_millis() - 25).to_string()).unwrap(),
        );

        let trace = ConversationSendTrace::from_headers(&request_headers);
        let mut response_headers = HeaderMap::new();
        trace.apply_response_headers(&mut response_headers);
        let request_id_text = request_id.to_string();

        assert_eq!(
            response_headers
                .get(&REQUEST_ID_HEADER)
                .and_then(|value| value.to_str().ok()),
            Some(request_id_text.as_str())
        );
        assert!(response_headers.get(&SERVER_DURATION_HEADER).is_some());
        assert!(response_headers.get(&CLIENT_TO_SERVER_HEADER).is_some());
    }
}
