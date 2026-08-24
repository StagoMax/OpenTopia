use super::{
    bundle_is_visible, calibrated_input_estimate, estimate_provider_tool_surface_tokens,
    external_namespace, json, mcp_tool_declares_image_inspection, normalize_tool_argument_keys,
    redact_model_observation, tool_bundle, AgentCore, AgentEventPayload, Arc, AtomicBool,
    AtomicOrdering, BTreeMap, CancellationToken, CanonicalModelRequest, CollaborationMode,
    CompiledModelContext, ContextAssemblyInput, HashSet, ModelCallPurpose, ModelContentPart,
    ModelConversationMessage, ModelGatewayMetricEvent, ModelResponse, ModelStreamDelta,
    MultiAgentMode, PromptCacheBreakpointPolicy, ProviderFeatureSupport, ProviderRequestCheckpoint,
    ProviderToolCall, ProviderToolCandidate, ProviderToolDisclosure, ProviderToolNamespace,
    ProviderToolResult, ProviderTransportEvent, ToolClass, ToolExposurePolicy, ToolSource,
    TurnEvents, Uuid, Value, AUTOMATIC_TOOL_DISCLOSURE_COUNT_THRESHOLD,
    AUTOMATIC_TOOL_DISCLOSURE_TOKEN_THRESHOLD, DEFAULT_EAGER_OFFICE_TOOLS, MAX_TOOL_SEARCH_RESULTS,
    TOOL_SEARCH_NAME,
};

impl AgentCore {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn assemble_model_request(
        &self,
        model_context: &CompiledModelContext,
        context_summary: Option<&str>,
        conversation: Vec<ModelConversationMessage>,
        user_message: String,
        user_content: Vec<ModelContentPart>,
        tool_candidates: Vec<ProviderToolCandidate>,
        previous_tool_calls: Vec<ProviderToolCall>,
        tool_results: Vec<ProviderToolResult>,
        previous_response_items: Vec<Value>,
        previous_response_id: Option<String>,
        branch_developer_instructions: Option<String>,
    ) -> anyhow::Result<CanonicalModelRequest> {
        self.kernel.context_assembler.compile(ContextAssemblyInput {
            model_context,
            context_summary,
            conversation,
            user_message,
            user_content,
            tool_candidates,
            previous_tool_calls,
            tool_results,
            previous_response_items,
            previous_response_id,
            branch_developer_instructions,
            prompt_cache_breakpoint_policy: PromptCacheBreakpointPolicy::AppendOnlyUsers,
            final_output_json_schema: None,
        })
    }

    pub(super) async fn complete_model(
        &self,
        request: CanonicalModelRequest,
        round: usize,
        provider_compatibility_hash: &str,
        events: &mut TurnEvents,
        cancellation: Option<&CancellationToken>,
    ) -> anyhow::Result<ModelResponse> {
        let request_id = Uuid::new_v4();
        let tool_input_schemas = request
            .logical()
            .tool_candidates
            .iter()
            .map(|candidate| (candidate.name.clone(), candidate.input_schema.clone()))
            .collect::<BTreeMap<_, _>>();
        let input_breakdown = request.logical().token_estimate_breakdown();
        let local_input_estimate = calibrated_input_estimate(events, input_breakdown.total);
        let materialized_context = request.materialized_context().clone();
        events.push(AgentEventPayload::ModelContextBuilt {
            request_id,
            round,
            context_hash: request.manifest().context_hash.clone(),
            stable_prefix_hash: Some(request.manifest().stable_prefix_hash.clone()),
            dynamic_tail_hash: Some(request.manifest().dynamic_tail_hash.clone()),
            token_estimate: local_input_estimate,
            purpose: ModelCallPurpose::AgentRound,
            token_breakdown: Some(input_breakdown.clone()),
            items: materialized_context.items,
        });
        let request_snapshot = serde_json::to_value(request.logical())
            .map(|value| redact_model_observation(&value))
            .unwrap_or_else(|error| json!({ "serializationError": error.to_string() }));
        events.push(AgentEventPayload::ModelRequest {
            request_id,
            round,
            request: request_snapshot,
        });
        let prepared = self.kernel.model_gateway.prepare(request_id, request)?;
        let checkpoint =
            prepared
                .wire_transcript
                .clone()
                .map(|transcript| ProviderRequestCheckpoint {
                    compatibility_hash: provider_compatibility_hash.to_string(),
                    transcript,
                });
        events.push(AgentEventPayload::ProviderRequestSent {
            request_id,
            round,
            attempt: 1,
            adapter: prepared.adapter.clone(),
            method: prepared.method.clone(),
            endpoint: prepared.endpoint.clone(),
            cache_trace: prepared.cache_trace.clone(),
            body: prepared.observation_body.clone(),
            checkpoint,
        });
        let live_event_sender = events.sender.clone();
        let mut transport_events = Vec::new();
        let mut on_transport = |observation| {
            let mut payloads = Vec::new();
            match observation {
                ProviderTransportEvent::Retry {
                    attempt,
                    retry_kind,
                    retry_index,
                    retry_limit,
                    reason,
                    cache_trace,
                    body,
                } => {
                    if reason.contains("stored response cursor unavailable") {
                        payloads.push(AgentEventPayload::ProviderContextStateInvalidated {
                            provider_id: None,
                            model: None,
                            reason: reason.clone(),
                        });
                    }
                    payloads.push(AgentEventPayload::ProviderRequestRetried {
                        request_id,
                        round,
                        attempt,
                        retry_kind,
                        retry_index,
                        retry_limit,
                        reason,
                        cache_trace,
                        body,
                    });
                }
                ProviderTransportEvent::Response {
                    attempt,
                    status,
                    response_id,
                    body,
                } => payloads.push(AgentEventPayload::ProviderResponseReceived {
                    request_id,
                    round,
                    attempt,
                    status,
                    response_id,
                    body,
                }),
            }
            for payload in payloads {
                let published_live =
                    !matches!(payload, AgentEventPayload::ProviderResponseReceived { .. });
                if published_live {
                    if let Some(sender) = &live_event_sender {
                        let _ = sender.send(payload.clone());
                    }
                }
                transport_events.push((payload, published_live));
            }
            Ok(())
        };
        let mut latest_usage = None;
        let mut proposed_plan_parser = (self.collaboration_mode == CollaborationMode::Plan)
            .then(super::proposed_plan::ProposedPlanStreamParser::default);
        let first_token_pending = Arc::new(AtomicBool::new(false));
        let metric_pending = Arc::clone(&first_token_pending);
        let metric_event_sender = events.sender.clone();
        let mut on_metric = |metric| {
            match metric {
                ModelGatewayMetricEvent::FirstOutputTokenReceived {
                    request_id: metric_request_id,
                } => {
                    debug_assert_eq!(metric_request_id, request_id);
                    metric_pending.store(true, AtomicOrdering::SeqCst);
                    if let Some(sender) = &metric_event_sender {
                        let _ = sender
                            .send(AgentEventPayload::ProviderFirstTokenReceived { request_id });
                    }
                }
            }
            Ok(())
        };
        let delta_pending = Arc::clone(&first_token_pending);
        let mut on_delta = |delta: ModelStreamDelta| {
            if delta_pending.swap(false, AtomicOrdering::SeqCst) {
                events.record(AgentEventPayload::ProviderFirstTokenReceived { request_id });
            }
            match delta {
                ModelStreamDelta::Text { text } => {
                    let visible = proposed_plan_parser
                        .as_mut()
                        .map(|parser| parser.push_str(&text))
                        .unwrap_or(text);
                    if !visible.is_empty() {
                        events.push(AgentEventPayload::ModelDelta { text: visible });
                    }
                }
                ModelStreamDelta::Reasoning { text } => {
                    events.push(AgentEventPayload::ReasoningDelta { text });
                }
                ModelStreamDelta::Usage { usage } => {
                    latest_usage = Some(usage);
                }
                ModelStreamDelta::ToolCall { .. } => {}
            }
            Ok(())
        };
        let stream = self.kernel.model_gateway.stream_prepared(
            prepared,
            &mut on_delta,
            &mut on_transport,
            &mut on_metric,
        );
        let response = if let Some(cancellation) = cancellation {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    Err(anyhow::anyhow!("turn cancelled while waiting for provider response"))
                }
                response = stream => response,
            }
        } else {
            stream.await
        };
        drop(on_delta);
        drop(on_transport);
        if let Some(parser) = proposed_plan_parser {
            let visible = parser.finish();
            if !visible.is_empty() {
                events.push(AgentEventPayload::ModelDelta { text: visible });
            }
        }
        if first_token_pending.swap(false, AtomicOrdering::SeqCst) {
            events.record(AgentEventPayload::ProviderFirstTokenReceived { request_id });
        }
        let latest_usage = latest_usage.or_else(|| {
            response
                .as_ref()
                .ok()
                .and_then(|response| response.usage.clone())
        });
        for (payload, published_live) in transport_events {
            if published_live {
                events.record(payload);
            } else {
                // A response marks the end of the provider wait in the UI. In
                // atomic tool rounds it can arrive before validated deltas are
                // released, so publish it only after those deltas are flushed.
                events.push(payload);
            }
        }
        if let Some(usage) = latest_usage {
            events.push(AgentEventPayload::TokenUsage {
                request_id: Some(request_id),
                round: Some(round),
                purpose: ModelCallPurpose::AgentRound,
                input_tokens: usage.input_tokens as usize,
                output_tokens: usage.output_tokens as usize,
                total_tokens: usage.total_tokens as usize,
                cached_input_tokens: usage.cached_input_tokens.map(|value| value as usize),
                cache_write_tokens: usage.cache_write_tokens.map(|value| value as usize),
                reasoning_tokens: usage.reasoning_tokens.map(|value| value as usize),
                local_input_estimate: Some(local_input_estimate),
                input_breakdown: Some(input_breakdown),
            });
        }
        let mut response = response?;
        let mut normalized_keys = Vec::new();
        for call in &mut response.tool_calls {
            let Some(schema) = tool_input_schemas.get(&call.name) else {
                continue;
            };
            normalized_keys.extend(
                normalize_tool_argument_keys(schema, &mut call.arguments)
                    .into_iter()
                    .map(|normalization| {
                        format!(
                            "{}:{} ({}→{})",
                            call.name, normalization.path, normalization.from, normalization.to
                        )
                    }),
            );
        }
        if !normalized_keys.is_empty() {
            events.push(AgentEventPayload::ContextWarning {
                stage: "tool_argument_key_normalization".to_string(),
                message: format!(
                    "Normalized tool argument key spelling to the advertised schema: {}",
                    normalized_keys.join(", ")
                ),
            });
        }
        Ok(response)
    }

    pub(super) fn eligible_provider_tool_candidates(&self) -> Vec<ProviderToolCandidate> {
        let agents_available = self.collaboration.is_some()
            && self.agent_runtime_settings.multi_agent != MultiAgentMode::Off;
        let structured_input_exposable = self.agent_depth == 0;
        self.tool_host
            .catalog
            .list()
            .into_iter()
            .filter(|name| {
                agents_available || self.tool_host.catalog.class(name) != Some(ToolClass::Agent)
            })
            .filter(|name| {
                structured_input_exposable
                    || self.tool_host.catalog.class(name) != Some(ToolClass::StructuredInput)
            })
            // The root agent owns the shared task plan. Children report results
            // to the parent instead of mutating the parent's plan namespace.
            .filter(|name| {
                self.agent_depth == 0
                    || self.tool_host.catalog.class(name) != Some(ToolClass::WorkForm)
            })
            .filter(|name| {
                let source = self
                    .tool_host
                    .catalog
                    .source(name)
                    .unwrap_or(ToolSource::Core);
                bundle_is_visible(
                    tool_bundle(
                        self.tool_host
                            .catalog
                            .class(name)
                            .unwrap_or(ToolClass::Standard),
                        &source,
                    ),
                    self.experience_mode,
                    self.collaboration_mode,
                )
            })
            .filter(|name| self.tool_host.model_supports_vision || name != "computer")
            .filter(|name| self.tool_is_allowed(name))
            // Compatibility executors can remain callable by persisted and
            // internal routes without any disclosure policy exposing them.
            .filter(|name| self.tool_host.catalog.is_model_visible(name))
            // MCP tools bound as attachment-inspection backends are implementation
            // details of view_attachment, not a competing model-visible route.
            .filter(|name| {
                !self.tool_host.active_mcp_tools.iter().any(|tool| {
                    tool.public_name == *name && mcp_tool_declares_image_inspection(tool)
                })
            })
            .filter_map(|name| {
                self.tool_host.catalog.get(&name).map(|tool| {
                    ProviderToolCandidate::direct(name, tool.description(), tool.schema())
                })
            })
            .collect()
    }

    pub(super) fn native_tool_search_active(&self, eligible: &[ProviderToolCandidate]) -> bool {
        let has_deferred_external_tools = eligible.iter().any(|candidate| {
            self.tool_host.catalog.source(&candidate.name) != Some(ToolSource::Core)
                && !self.is_default_eager_office_tool(&candidate.name)
        });
        has_deferred_external_tools
            && self.tool_exposure_policy != ToolExposurePolicy::Eager
            && self.provider_tool_protocol.hosted_tool_search == ProviderFeatureSupport::Supported
            && self.provider_tool_protocol.deferred_tool_loading
                == ProviderFeatureSupport::Supported
    }

    pub(super) fn progressive_tool_disclosure_active(
        &self,
        eligible: &[ProviderToolCandidate],
    ) -> bool {
        let external = eligible
            .iter()
            .filter(|candidate| {
                self.tool_host.catalog.source(&candidate.name) != Some(ToolSource::Core)
            })
            .cloned()
            .collect::<Vec<_>>();
        if external.is_empty() {
            return false;
        }
        match self.tool_exposure_policy {
            ToolExposurePolicy::Eager => false,
            ToolExposurePolicy::Progressive => true,
            ToolExposurePolicy::Automatic => {
                external.len() >= AUTOMATIC_TOOL_DISCLOSURE_COUNT_THRESHOLD
                    || estimate_provider_tool_surface_tokens(&external)
                        >= AUTOMATIC_TOOL_DISCLOSURE_TOKEN_THRESHOLD
            }
        }
    }

    pub(super) fn is_default_eager_office_tool(&self, name: &str) -> bool {
        let Some((_, expected_plugin)) = DEFAULT_EAGER_OFFICE_TOOLS
            .iter()
            .find(|(tool_name, _)| *tool_name == name)
        else {
            return false;
        };
        matches!(
            self.tool_host.catalog.source(name),
            Some(ToolSource::BundledPlugin { plugin_name }) if plugin_name == *expected_plugin
        )
    }

    pub(super) fn client_deferred_tool_candidate(
        &self,
        candidate: &ProviderToolCandidate,
        defer_all_external: bool,
    ) -> bool {
        if self.tool_host.catalog.source(&candidate.name) == Some(ToolSource::Core) {
            return false;
        }
        if self.is_default_eager_office_tool(&candidate.name) {
            return false;
        }
        if self.attachment_preloaded_tools.contains(&candidate.name) {
            return false;
        }
        defer_all_external
    }

    pub(super) fn deferred_namespace_catalog(&self, eligible: &[ProviderToolCandidate]) -> String {
        let namespaces = eligible
            .iter()
            .filter_map(|candidate| {
                let source = self.tool_host.catalog.source(&candidate.name)?;
                external_namespace(&candidate.name, &source)
            })
            .collect::<BTreeMap<_, _>>();
        if namespaces.is_empty() {
            return String::new();
        }
        let groups = namespaces
            .into_iter()
            .map(|(name, description)| format!("{name}: {description}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!(" Available deferred tool groups: {groups}")
    }

    pub(super) fn provider_tool_candidates(&self) -> Vec<ProviderToolCandidate> {
        let mut eligible = self.eligible_provider_tool_candidates();
        if self.native_tool_search_active(&eligible) {
            for candidate in &mut eligible {
                if self.is_default_eager_office_tool(&candidate.name) {
                    continue;
                }
                let source = self
                    .tool_host
                    .catalog
                    .source(&candidate.name)
                    .unwrap_or(ToolSource::Core);
                let Some((name, description)) = external_namespace(&candidate.name, &source) else {
                    continue;
                };
                if self.provider_tool_protocol.namespace_tools == ProviderFeatureSupport::Supported
                {
                    candidate.disclosure = ProviderToolDisclosure::DeferredNamespace;
                    candidate.namespace = Some(ProviderToolNamespace { name, description });
                } else {
                    candidate.disclosure = ProviderToolDisclosure::DeferredIndividual;
                }
            }
            return eligible;
        }
        if self.tool_exposure_policy == ToolExposurePolicy::Eager {
            return eligible;
        }

        let defer_all_external = self.progressive_tool_disclosure_active(&eligible);
        let has_deferred_tools = eligible
            .iter()
            .any(|candidate| self.client_deferred_tool_candidate(candidate, defer_all_external));
        if !has_deferred_tools {
            return eligible;
        }

        let deferred = eligible
            .iter()
            .filter(|candidate| self.client_deferred_tool_candidate(candidate, defer_all_external))
            .cloned()
            .collect::<Vec<_>>();
        let search_description = format!(
            "Search the deferred tool catalog by capability. Matching tools are made available on the next model round; use the returned names rather than guessing an unloaded tool schema.{}",
            self.deferred_namespace_catalog(&deferred)
        );
        let mut exposed = eligible
            .into_iter()
            .filter(|candidate| !self.client_deferred_tool_candidate(candidate, defer_all_external))
            .collect::<Vec<_>>();
        exposed.push(ProviderToolCandidate::direct(
            TOOL_SEARCH_NAME,
            search_description,
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Capability or action to search for."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_TOOL_SEARCH_RESULTS,
                        "description": "Maximum matches to reveal."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        ));
        exposed
    }

    /// Reconciles a checkpoint's disclosure state with the currently executable
    /// catalog. Previously revealed tools remain revealed, but their contracts
    /// are always replaced with the current definitions.
    pub(super) fn refresh_resumed_tool_candidates(
        &self,
        saved: &[ProviderToolCandidate],
    ) -> Vec<ProviderToolCandidate> {
        let baseline = self.provider_tool_candidates();
        let baseline_by_name = baseline
            .iter()
            .cloned()
            .map(|candidate| (candidate.name.clone(), candidate))
            .collect::<BTreeMap<_, _>>();
        let eligible_by_name = self
            .eligible_provider_tool_candidates()
            .into_iter()
            .map(|candidate| (candidate.name.clone(), candidate))
            .collect::<BTreeMap<_, _>>();
        let mut refreshed = Vec::new();
        let mut included = HashSet::new();

        for candidate in saved {
            let current = baseline_by_name
                .get(&candidate.name)
                .or_else(|| eligible_by_name.get(&candidate.name));
            if let Some(current) = current {
                if included.insert(current.name.clone()) {
                    refreshed.push(current.clone());
                }
            }
        }
        for candidate in baseline {
            if included.insert(candidate.name.clone()) {
                refreshed.push(candidate);
            }
        }
        refreshed
    }

    pub(super) fn search_deferred_tools(
        &self,
        query: &str,
        limit: usize,
    ) -> Vec<ProviderToolCandidate> {
        let terms = query
            .split(|character: char| !character.is_alphanumeric() && character != '_')
            .map(str::trim)
            .filter(|term| !term.is_empty())
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        if terms.is_empty() {
            return Vec::new();
        }

        let eligible = self.eligible_provider_tool_candidates();
        let defer_all_external = self.progressive_tool_disclosure_active(&eligible);
        let mut matches = eligible
            .into_iter()
            .filter(|candidate| self.client_deferred_tool_candidate(candidate, defer_all_external))
            .filter_map(|candidate| {
                let name = candidate.name.to_lowercase();
                let description = candidate.description.to_lowercase();
                let matched = terms
                    .iter()
                    .filter(|term| {
                        name.contains(term.as_str()) || description.contains(term.as_str())
                    })
                    .count();
                (matched > 0).then_some((matched, candidate))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.name.cmp(&right.name))
        });
        matches
            .into_iter()
            .take(limit.min(MAX_TOOL_SEARCH_RESULTS))
            .map(|(_, candidate)| candidate)
            .collect()
    }

    pub(super) fn reveal_tools_from_search_result(
        &self,
        result: &ProviderToolResult,
        exposed: &mut Vec<ProviderToolCandidate>,
    ) -> bool {
        if result.name != TOOL_SEARCH_NAME || result.is_error {
            return false;
        }
        let names = result
            .metadata
            .get("revealedTools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<HashSet<_>>();
        if names.is_empty() {
            return false;
        }
        let existing = exposed
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect::<HashSet<_>>();
        let mut additions = self
            .eligible_provider_tool_candidates()
            .into_iter()
            .filter(|candidate| names.contains(candidate.name.as_str()))
            .filter(|candidate| !existing.contains(candidate.name.as_str()))
            .collect::<Vec<_>>();
        let changed = !additions.is_empty();
        exposed.append(&mut additions);
        changed
    }

    /// Structured user decisions belong to Plan mode. Only the root agent owns
    /// the interactive boundary.
    pub(super) fn request_user_input_is_available(&self) -> bool {
        self.collaboration_mode == CollaborationMode::Plan
            && self.agent_depth == 0
            && self.tool_host.catalog.get("request_user_input").is_some()
            && self.tool_is_allowed("request_user_input")
    }

    pub(super) fn tool_is_allowed(&self, name: &str) -> bool {
        let plugin_enabled = match self.tool_host.catalog.source(name) {
            Some(ToolSource::BundledPlugin { plugin_name }) => {
                self.enabled_bundled_plugins.contains(&plugin_name)
                    && self.capability_projection.allows_plugin(&plugin_name)
            }
            _ => true,
        };
        plugin_enabled
            && self.capability_projection.allows_tool(name)
            && !self.denied_tools.contains(name)
            && self
                .allowed_tools
                .as_ref()
                .map(|allowed| allowed.contains(name))
                .unwrap_or(true)
    }

    pub(super) fn insert_tool_source_metadata(&self, name: &str, metadata: &mut Value) {
        let Some(object) = metadata.as_object_mut() else {
            return;
        };
        match self.tool_host.catalog.source(name) {
            Some(ToolSource::Core) => {
                object.insert("toolSource".to_string(), json!("core"));
            }
            Some(ToolSource::BundledPlugin { plugin_name }) => {
                object.insert("toolSource".to_string(), json!("bundled_plugin"));
                object.insert("pluginName".to_string(), json!(plugin_name));
            }
            Some(ToolSource::Mcp) => {
                object.insert("toolSource".to_string(), json!("mcp"));
            }
            None => {}
        }
    }
}
