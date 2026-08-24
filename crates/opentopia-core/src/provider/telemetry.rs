use super::{ModelStreamDelta, ProviderTransportCallback, ProviderTransportEvent};
use std::time::{Duration, Instant};
use uuid::Uuid;

const STREAM_PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

/// Observes provider output below the semantic response buffer. It intentionally
/// records only counts and timings: model text and tool arguments never enter
/// diagnostic logs or progress events.
pub(crate) struct ProviderStreamTelemetry {
    request_id: Uuid,
    attempt: usize,
    started_at: Instant,
    last_progress_at: Instant,
    output_events: usize,
    output_bytes: usize,
    last_reported_events: usize,
}

impl ProviderStreamTelemetry {
    pub(crate) fn new(request_id: Uuid, attempt: usize) -> Self {
        let now = Instant::now();
        Self {
            request_id,
            attempt,
            started_at: now,
            last_progress_at: now,
            output_events: 0,
            output_bytes: 0,
            last_reported_events: 0,
        }
    }

    pub(crate) fn observe(
        &mut self,
        delta: &ModelStreamDelta,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<()> {
        let bytes = output_bytes(delta);
        if bytes == 0 {
            return Ok(());
        }

        self.output_events = self.output_events.saturating_add(1);
        self.output_bytes = self.output_bytes.saturating_add(bytes);
        if self.output_events == 1 {
            tracing::info!(
                target: "opentopia::provider_timing",
                request_id = %self.request_id,
                attempt = self.attempt,
                "provider output started"
            );
            on_transport(ProviderTransportEvent::OutputStarted {
                attempt: self.attempt,
            })?;
        }

        if self.last_progress_at.elapsed() >= STREAM_PROGRESS_INTERVAL {
            self.emit_progress(on_transport)?;
        }
        Ok(())
    }

    pub(crate) fn finish_progress(
        &mut self,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<()> {
        if self.output_events > self.last_reported_events {
            self.emit_progress(on_transport)?;
        }
        Ok(())
    }

    pub(crate) fn emit_commit_started(
        &self,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<()> {
        let elapsed_ms = elapsed_millis(self.started_at);
        tracing::info!(
            target: "opentopia::provider_timing",
            request_id = %self.request_id,
            attempt = self.attempt,
            output_events = self.output_events,
            output_bytes = self.output_bytes,
            elapsed_ms,
            "provider response validated; semantic commit starting"
        );
        on_transport(ProviderTransportEvent::ResponseCommitStarted {
            attempt: self.attempt,
            output_events: self.output_events,
            output_bytes: self.output_bytes,
            elapsed_ms,
        })
    }

    fn emit_progress(
        &mut self,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<()> {
        let elapsed_ms = elapsed_millis(self.started_at);
        tracing::info!(
            target: "opentopia::provider_timing",
            request_id = %self.request_id,
            attempt = self.attempt,
            output_events = self.output_events,
            output_bytes = self.output_bytes,
            elapsed_ms,
            "provider stream progressing"
        );
        on_transport(ProviderTransportEvent::StreamProgress {
            attempt: self.attempt,
            output_events: self.output_events,
            output_bytes: self.output_bytes,
            elapsed_ms,
        })?;
        self.last_reported_events = self.output_events;
        self.last_progress_at = Instant::now();
        Ok(())
    }
}

pub(crate) fn emit_response_headers(
    request_id: Uuid,
    attempt: usize,
    status: u16,
    on_transport: &mut ProviderTransportCallback<'_>,
) -> anyhow::Result<()> {
    tracing::info!(
        target: "opentopia::provider_timing",
        %request_id,
        attempt,
        status,
        "provider response headers received"
    );
    on_transport(ProviderTransportEvent::ResponseHeaders { attempt, status })
}

fn output_bytes(delta: &ModelStreamDelta) -> usize {
    match delta {
        ModelStreamDelta::Text { text } | ModelStreamDelta::Reasoning { text } => text.len(),
        ModelStreamDelta::ToolCall {
            id,
            name,
            arguments_delta,
            ..
        } => {
            id.as_deref().map_or(0, str::len)
                + name.as_deref().map_or(0, str::len)
                + arguments_delta.len()
        }
        ModelStreamDelta::Usage { .. } => 0,
    }
}

fn elapsed_millis(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_start_and_final_progress_do_not_expose_content() {
        let request_id = Uuid::from_u128(7);
        let mut telemetry = ProviderStreamTelemetry::new(request_id, 2);
        let mut events = Vec::new();
        let mut transport = |event| {
            events.push(event);
            Ok(())
        };

        telemetry
            .observe(
                &ModelStreamDelta::Reasoning {
                    text: "private reasoning".to_string(),
                },
                &mut transport,
            )
            .unwrap();
        telemetry.finish_progress(&mut transport).unwrap();

        assert!(matches!(
            events[0],
            ProviderTransportEvent::OutputStarted { attempt: 2 }
        ));
        assert!(matches!(
            events[1],
            ProviderTransportEvent::StreamProgress {
                attempt: 2,
                output_events: 1,
                output_bytes: 17,
                ..
            }
        ));
    }
}
