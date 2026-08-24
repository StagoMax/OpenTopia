use super::{context_compaction_details, generate_context_summary};
use crate::{AppState, ProviderSettings, ProviderTransportKind, SessionStore};
use async_trait::async_trait;
use opentopia_core::{
    ModelRequest, RoundContextCompactionRequest, RoundContextCompactionResult,
    RoundContextCompactor,
};
use std::collections::HashSet;
use std::fmt;

#[derive(Clone)]
pub(crate) struct ServerRoundContextCompactor {
    state: AppState,
    provider: ProviderSettings,
}

impl ServerRoundContextCompactor {
    pub(crate) fn new(state: AppState, provider: ProviderSettings) -> Self {
        Self { state, provider }
    }
}

impl fmt::Debug for ServerRoundContextCompactor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerRoundContextCompactor")
            .field("provider_id", &self.provider.id)
            .field("model", &self.provider.model)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl RoundContextCompactor for ServerRoundContextCompactor {
    async fn compact(
        &self,
        request: RoundContextCompactionRequest,
    ) -> anyhow::Result<RoundContextCompactionResult> {
        anyhow::ensure!(
            self.provider.effective_transport() != ProviderTransportKind::Mock,
            "durable round compaction requires a real provider"
        );

        let queued = self
            .state
            .store
            .list_queued_turn_messages(request.thread_id)?
            .into_iter()
            .collect::<HashSet<_>>();
        let messages = self
            .state
            .store
            .list_messages(request.thread_id)?
            .into_iter()
            .filter(|message| !queued.contains(&message.id))
            .collect::<Vec<_>>();
        let events = self.state.store.list_events(request.thread_id, None)?;
        let summary = generate_context_summary(
            &self.state,
            request.thread_id,
            Some(request.turn_id),
            &request.agent_path,
            &messages,
            &events,
            "round_pressure",
            None,
            Some(&self.provider),
            &request.model_request,
            request.event_sender.as_ref(),
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
        let details = Some(context_compaction_details(
            &self.state,
            request.thread_id,
            &summary,
        ));
        let covered_tool_call_ids = covered_provider_tool_call_ids(&request.model_request);

        Ok(RoundContextCompactionResult {
            summary,
            details,
            covered_tool_call_ids,
        })
    }
}

fn covered_provider_tool_call_ids(request: &ModelRequest) -> HashSet<String> {
    request
        .input
        .conversation
        .iter()
        .flat_map(|message| message.tool_results.iter())
        .chain(request.input.tool_results.iter())
        .map(|result| result.call_id.clone())
        .filter(|call_id| !call_id.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::covered_provider_tool_call_ids;
    use opentopia_core::{
        ModelConversationMessage, ModelConversationRole, ModelRequest, ProviderToolResult,
    };
    use serde_json::Value;

    fn result(call_id: &str, name: &str) -> ProviderToolResult {
        ProviderToolResult {
            call_id: call_id.to_string(),
            name: name.to_string(),
            output: "done".to_string(),
            content: Vec::new(),
            is_error: false,
            metadata: Value::Null,
        }
    }

    #[test]
    fn covered_tool_results_map_back_to_provider_call_ids() {
        let mut request = ModelRequest {
            instructions: Default::default(),
            input: Default::default(),
            tool_candidates: Vec::new(),
            previous_response_items: Vec::new(),
            provider_transcript: None,
            previous_response_id: None,
            prompt_cache_breakpoint_policy: Default::default(),
            final_output_json_schema: None,
        };
        request.input.conversation = vec![ModelConversationMessage {
            role: ModelConversationRole::Tool,
            content: String::new(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: vec![result("provider-call-7", "read_file")],
        }];
        request.input.tool_results = vec![result("provider-call-8", "shell")];

        let covered = covered_provider_tool_call_ids(&request);
        assert!(covered.contains("provider-call-7"));
        assert!(covered.contains("provider-call-8"));
    }
}
