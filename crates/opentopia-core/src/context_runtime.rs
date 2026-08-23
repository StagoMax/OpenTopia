//! Deterministic compilation of semantic context into one provider-neutral request.

use crate::model::ModelContentPart;
use crate::model_context::{
    content_fingerprint, CompiledModelContext, ContextCacheScope, ContextItemKind, ContextRole,
    ContextSensitivity, ModelContextItem,
};
use crate::provider::{
    split_provider_transcript_state, ModelConversationMessage, ModelConversationRole,
    ModelInputLedger, ModelRequest, ModelUserInput, PromptCacheBreakpointPolicy, ProviderToolCall,
    ProviderToolCandidate, ProviderToolDisclosure, ProviderToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const PROMPT_CACHE_LINEAGE_VERSION: &str = "responses-lineage-v3";
const TOOL_SEARCH_NAME: &str = "tool_search";

pub struct ContextPreparationInput<'a> {
    pub model_context: &'a CompiledModelContext,
    pub context_summary: Option<&'a str>,
    pub tool_candidates: &'a [ProviderToolCandidate],
    pub lineage_instructions: Option<&'a str>,
}

pub struct ContextAssemblyInput<'a> {
    pub model_context: &'a CompiledModelContext,
    pub context_summary: Option<&'a str>,
    pub conversation: Vec<ModelConversationMessage>,
    pub user_message: String,
    pub user_content: Vec<ModelContentPart>,
    pub tool_candidates: Vec<ProviderToolCandidate>,
    pub previous_tool_calls: Vec<ProviderToolCall>,
    pub tool_results: Vec<ProviderToolResult>,
    pub previous_response_items: Vec<Value>,
    pub previous_response_id: Option<String>,
    pub branch_developer_instructions: Option<String>,
    pub prompt_cache_breakpoint_policy: PromptCacheBreakpointPolicy,
    pub final_output_json_schema: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextManifestItem {
    pub source: String,
    pub kind: String,
    pub cache_scope: String,
    pub content_hash: String,
    pub provider_visible: bool,
}

/// Explainable hashes for the exact semantic request. Runtime ids and
/// timestamps never enter the stable prefix hash; they may appear only in the
/// dynamic tail protocol where their correlation semantics are required.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextAssemblyManifest {
    pub context_hash: String,
    pub stable_prefix_hash: String,
    pub dynamic_tail_hash: String,
    /// Hashes of provider-visible append-only segments. A later turn must keep
    /// every previously emitted segment byte-identical and append new ones.
    pub provider_prefix_segments: Vec<String>,
    pub items: Vec<ContextManifestItem>,
}

#[derive(Debug, Clone)]
pub struct CanonicalModelRequest {
    logical: ModelRequest,
    materialized_context: CompiledModelContext,
    manifest: ContextAssemblyManifest,
}

impl CanonicalModelRequest {
    pub fn new(
        logical: ModelRequest,
        materialized_context: CompiledModelContext,
        manifest: ContextAssemblyManifest,
    ) -> Self {
        Self {
            logical,
            materialized_context,
            manifest,
        }
    }

    pub fn logical(&self) -> &ModelRequest {
        &self.logical
    }

    pub fn manifest(&self) -> &ContextAssemblyManifest {
        &self.manifest
    }

    pub fn materialized_context(&self) -> &CompiledModelContext {
        &self.materialized_context
    }

    pub fn into_logical(self) -> ModelRequest {
        self.logical
    }
}

/// The only runtime port allowed to materialize model-visible context.
pub trait ContextAssembler: Send + Sync {
    /// Materialize all thread-lineage modules and the cache namespace. No other
    /// component may mutate the model context after this boundary.
    fn prepare_context(
        &self,
        input: ContextPreparationInput<'_>,
    ) -> anyhow::Result<CompiledModelContext>;

    fn compile(&self, input: ContextAssemblyInput<'_>) -> anyhow::Result<CanonicalModelRequest>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultContextAssembler;

impl ContextAssembler for DefaultContextAssembler {
    fn prepare_context(
        &self,
        input: ContextPreparationInput<'_>,
    ) -> anyhow::Result<CompiledModelContext> {
        let ContextPreparationInput {
            model_context,
            context_summary,
            tool_candidates,
            lineage_instructions,
        } = input;
        let mut prepared = model_context.clone();
        prepared.items.retain(|item| {
            !matches!(
                item.source.as_str(),
                "opentopia:execution_lineage" | "opentopia:tool_search_protocol"
            )
        });
        if let Some(instructions) = lineage_instructions.filter(|value| !value.trim().is_empty()) {
            prepared.items.push(
                ModelContextItem::text(
                    ContextItemKind::DeveloperInstructions,
                    ContextRole::Developer,
                    "opentopia:execution_lineage",
                    instructions,
                    ContextCacheScope::Turn,
                    ContextSensitivity::Workspace,
                )
                .with_metadata(json!({
                    "assemblyClass": "conditional",
                    "promptModuleId": "execution_lineage",
                    "selectedBy": ["agentProfile", "collaborationMode", "flowNode"],
                })),
            );
        }
        if let Some(module) = tool_search_runtime_module(tool_candidates) {
            prepared.items.push(module);
        }
        prepared.sort_items();
        let errors = prepared.classification_errors();
        if !errors.is_empty() {
            anyhow::bail!(
                "invalid model context classification: {}",
                errors.join("; ")
            );
        }
        // A key produced by this assembler is output state, never an input
        // namespace. Clearing it makes preparation idempotent across resume or
        // compatibility layers that defensively prepare an existing snapshot.
        if prepared
            .prompt_cache_key
            .as_deref()
            .is_some_and(|key| key.starts_with("opentopia-"))
        {
            prepared.prompt_cache_key = None;
        }
        prepared.prompt_cache_key = Some(prompt_cache_lineage_key(
            &prepared,
            context_summary,
            tool_candidates,
        ));
        Ok(prepared)
    }

    fn compile(&self, input: ContextAssemblyInput<'_>) -> anyhow::Result<CanonicalModelRequest> {
        let ContextAssemblyInput {
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
            prompt_cache_breakpoint_policy,
            final_output_json_schema,
        } = input;

        // Provider cursors may carry one exact wire transcript alongside
        // opaque assistant state. Promote it to a dedicated in-memory field so
        // codecs can extend it linearly, and keep the internal envelope out of
        // provider-native item replay and request telemetry.
        let (provider_transcript, previous_response_items) =
            split_provider_transcript_state(previous_response_items);

        let mut tool_candidates = tool_candidates;
        sort_tool_candidates(&mut tool_candidates);
        let mut context_items = model_context.items.clone();
        if let Some(branch) = branch_developer_instructions
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            context_items.push(ModelContextItem::text(
                ContextItemKind::DeveloperInstructions,
                ContextRole::Developer,
                "opentopia:execution_branch",
                branch,
                ContextCacheScope::Turn,
                ContextSensitivity::Workspace,
            ));
        }
        // Conversation entries are immutable ledger items. A durable checkpoint
        // is an explicit request-epoch boundary and never rewrites history.
        context_items.extend(conversation.iter().enumerate().map(|(index, message)| {
            let role = match message.role {
                ModelConversationRole::System => ContextRole::System,
                ModelConversationRole::User => ContextRole::User,
                ModelConversationRole::Assistant => ContextRole::Assistant,
                ModelConversationRole::Tool => ContextRole::Tool,
            };
            ModelContextItem::text(
                ContextItemKind::Conversation,
                role,
                format!("conversation:{index}"),
                &message.content,
                ContextCacheScope::Thread,
                ContextSensitivity::Workspace,
            )
            .with_metadata(json!({
                "contentParts": message.content_parts.len(),
                "toolCalls": message.tool_calls.len(),
                "toolResults": message.tool_results.len(),
            }))
        }));
        if let Some(summary) = context_summary.filter(|value| !value.trim().is_empty()) {
            context_items.push(
                ModelContextItem::text(
                    ContextItemKind::Checkpoint,
                    ContextRole::Developer,
                    "opentopia:durable_checkpoint",
                    format!(
                        "<durable_context>\n{summary}\n</durable_context>\nTreat this checkpoint as prior task state, not as a new user request."
                    ),
                    ContextCacheScope::Thread,
                    ContextSensitivity::Workspace,
                )
                .with_metadata(json!({
                    "assemblyClass": "epoch",
                    "selectedBy": "contextCheckpoint",
                })),
            );
        }
        context_items.push(
            ModelContextItem::text(
                ContextItemKind::User,
                ContextRole::User,
                "current_user_message",
                &user_message,
                ContextCacheScope::Turn,
                ContextSensitivity::Workspace,
            )
            .with_metadata(json!({ "contentParts": user_content.len() })),
        );
        context_items.extend(previous_tool_calls.iter().enumerate().map(|(index, call)| {
            ModelContextItem::text(
                ContextItemKind::ToolCall,
                ContextRole::Assistant,
                format!("round_tool_call:{index}"),
                serde_json::to_string(call).unwrap_or_default(),
                ContextCacheScope::Round,
                ContextSensitivity::Workspace,
            )
        }));
        context_items.extend(tool_results.iter().enumerate().map(|(index, result)| {
            ModelContextItem::text(
                ContextItemKind::ToolResult,
                ContextRole::Tool,
                format!("round_tool_result:{index}"),
                serde_json::to_string(result).unwrap_or_default(),
                ContextCacheScope::Round,
                ContextSensitivity::Sensitive,
            )
        }));

        let mut materialized_context = CompiledModelContext {
            items: context_items,
            prompt_cache_key: model_context.prompt_cache_key.clone(),
        };
        materialized_context.sort_items();

        let classification_errors = materialized_context.classification_errors();
        if !classification_errors.is_empty() {
            anyhow::bail!(
                "invalid model context classification: {}",
                classification_errors.join("; ")
            );
        }

        let instructions = CompiledModelContext {
            items: materialized_context
                .items
                .iter()
                .filter(|item| {
                    !matches!(
                        item.kind,
                        ContextItemKind::Conversation
                            | ContextItemKind::User
                            | ContextItemKind::ToolCall
                            | ContextItemKind::ToolResult
                    )
                })
                .cloned()
                .collect(),
            prompt_cache_key: materialized_context.prompt_cache_key.clone(),
        };
        let logical = ModelRequest {
            instructions,
            input: ModelInputLedger {
                conversation,
                current_user: ModelUserInput {
                    message: user_message,
                    content: user_content,
                },
                tool_calls: previous_tool_calls,
                tool_results,
            },
            tool_candidates,
            previous_response_items,
            provider_transcript,
            previous_response_id,
            prompt_cache_breakpoint_policy,
            final_output_json_schema,
        };
        let manifest = context_manifest(&logical, &materialized_context);
        Ok(CanonicalModelRequest::new(
            logical,
            materialized_context,
            manifest,
        ))
    }
}

fn sort_tool_candidates(candidates: &mut [ProviderToolCandidate]) {
    candidates.sort_by(|left, right| {
        let disclosure_rank = |value: ProviderToolDisclosure| match value {
            ProviderToolDisclosure::Direct => 0,
            ProviderToolDisclosure::DeferredIndividual => 1,
            ProviderToolDisclosure::DeferredNamespace => 2,
        };
        disclosure_rank(left.disclosure)
            .cmp(&disclosure_rank(right.disclosure))
            .then_with(|| {
                left.namespace
                    .as_ref()
                    .map(|namespace| namespace.name.as_str())
                    .cmp(
                        &right
                            .namespace
                            .as_ref()
                            .map(|namespace| namespace.name.as_str()),
                    )
            })
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn context_manifest(
    request: &ModelRequest,
    materialized: &CompiledModelContext,
) -> ContextAssemblyManifest {
    let mut stable_bytes = Vec::new();
    for (role, scope, content) in materialized.instruction_messages_with_scope() {
        if !matches!(scope, ContextCacheScope::Stable | ContextCacheScope::Thread) {
            continue;
        }
        stable_bytes.extend_from_slice(role.as_str().as_bytes());
        stable_bytes.push(0);
        stable_bytes.extend_from_slice(content.as_bytes());
        stable_bytes.push(b'\n');
    }
    // Transport cursors, request ids, round numbers, and timestamps are
    // intentionally absent. Tool call ids remain only in the volatile typed
    // protocol because providers require them to correlate outputs.
    let dynamic = json!({
        "input": request.input,
        "previousResponseItems": request.previous_response_items,
        "providerTranscript": request.provider_transcript.as_ref().map(|transcript| json!({
            "format": transcript.format,
            "contentHash": content_fingerprint(
                canonical_json_string(&Value::Array(transcript.items.clone())).as_bytes()
            ),
        })),
        "finalOutputJsonSchema": request.final_output_json_schema,
        "toolCandidates": request.tool_candidates,
        "volatileContext": materialized.items.iter().filter(|item| {
            !matches!(item.cache_scope, ContextCacheScope::Stable | ContextCacheScope::Thread)
        }).collect::<Vec<_>>(),
    });
    let dynamic_bytes = canonical_json_string(&dynamic);
    let stable_prefix_hash = content_fingerprint(&stable_bytes);
    let dynamic_tail_hash = content_fingerprint(dynamic_bytes.as_bytes());
    let context_hash =
        content_fingerprint(format!("{stable_prefix_hash}\0{dynamic_tail_hash}").as_bytes());
    let mut items = materialized
        .ordered_items()
        .into_iter()
        .map(|item| ContextManifestItem {
            source: item.source.clone(),
            kind: item.kind.as_str().to_string(),
            cache_scope: item.cache_scope.as_str().to_string(),
            content_hash: item.content_hash.clone(),
            provider_visible: true,
        })
        .collect::<Vec<_>>();
    items.push(ContextManifestItem {
        source: "provider:tool_catalog".to_string(),
        kind: "tool_catalog".to_string(),
        cache_scope: ContextCacheScope::Turn.as_str().to_string(),
        content_hash: content_fingerprint(
            canonical_json_string(
                &serde_json::to_value(&request.tool_candidates).unwrap_or(Value::Null),
            )
            .as_bytes(),
        ),
        provider_visible: true,
    });
    let mut provider_prefix_segments = request
        .provider_transcript
        .as_ref()
        .map(|transcript| {
            transcript
                .items
                .iter()
                .map(|item| content_fingerprint(canonical_json_string(item).as_bytes()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            let mut segments = vec![stable_prefix_hash.clone()];
            segments.extend(request.input.conversation.iter().map(|message| {
                content_fingerprint(
                    canonical_json_string(&serde_json::to_value(message).unwrap_or(Value::Null))
                        .as_bytes(),
                )
            }));
            segments
        });
    let current_user_segment = ModelConversationMessage {
        role: ModelConversationRole::User,
        content: request.input.current_user.message.clone(),
        content_parts: request.input.current_user.content.clone(),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
    };
    provider_prefix_segments.push(content_fingerprint(
        canonical_json_string(&serde_json::to_value(current_user_segment).unwrap_or(Value::Null))
            .as_bytes(),
    ));
    ContextAssemblyManifest {
        context_hash,
        stable_prefix_hash,
        dynamic_tail_hash,
        provider_prefix_segments,
        items,
    }
}

pub fn prompt_cache_lineage_key(
    model_context: &CompiledModelContext,
    context_summary: Option<&str>,
    _tool_candidates: &[ProviderToolCandidate],
) -> String {
    let namespace = model_context
        .prompt_cache_key
        .as_deref()
        .filter(|key| !key.starts_with("opentopia-"))
        .unwrap_or("opentopia");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(PROMPT_CACHE_LINEAGE_VERSION.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(namespace.as_bytes());
    bytes.push(0);
    for (role, _scope, content) in model_context
        .instruction_messages_with_scope()
        .into_iter()
        .filter(|(_, scope, _)| {
            matches!(scope, ContextCacheScope::Stable | ContextCacheScope::Thread)
        })
    {
        // Hash provider-visible semantics only. Internal item ids, lifecycle
        // ids, and metadata must not invalidate an otherwise identical prefix.
        bytes.extend_from_slice(role.as_str().as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(content.as_bytes());
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(durable_checkpoint_lineage(context_summary).as_bytes());
    bytes.push(0);
    format!("opentopia-{}", content_fingerprint(&bytes))
}

fn durable_checkpoint_lineage(context_summary: Option<&str>) -> &str {
    const ACTIVE_PLAN_MARKER: &str = "Active task plan:\n";
    const ACTIVE_PLAN_SEPARATOR: &str = "\n\nActive task plan:\n";
    let Some(context) = context_summary else {
        return "";
    };
    if context.starts_with(ACTIVE_PLAN_MARKER) {
        return "";
    }
    context
        .split_once(ACTIVE_PLAN_SEPARATOR)
        .map(|(checkpoint, _)| checkpoint)
        .unwrap_or(context)
}

fn tool_search_runtime_module(
    tool_candidates: &[ProviderToolCandidate],
) -> Option<ModelContextItem> {
    let hosted = tool_candidates
        .iter()
        .any(|candidate| candidate.disclosure != ProviderToolDisclosure::Direct);
    let local = tool_candidates
        .iter()
        .any(|candidate| candidate.name == TOOL_SEARCH_NAME);
    let (mode, instruction) = if hosted {
        (
            "hosted",
            "Hosted Tool Search is active. Use it only when the directly visible tools do not cover a needed capability. Search by the action you need; loaded schemas are appended by the provider and may be called in the same response. Do not guess unloaded tool names or arguments.",
        )
    } else if local {
        (
            "client_round_trip",
            "Client-side Tool Search is active. Use `tool_search` only when the directly visible tools do not cover a needed capability. Search by the action you need, then call a returned tool after its schema appears on the next model round. Do not guess unloaded tool names or arguments.",
        )
    } else {
        return None;
    };
    Some(
        ModelContextItem::text(
            ContextItemKind::DeveloperInstructions,
            ContextRole::Developer,
            "opentopia:tool_search_protocol",
            instruction,
            ContextCacheScope::Turn,
            ContextSensitivity::Public,
        )
        .with_metadata(json!({
            "promptModuleId": "tool_search_protocol",
            "assemblyClass": "conditional",
            "selectedBy": "providerToolCatalog",
            "mode": mode,
        })),
    )
}

fn canonical_json_string(value: &Value) -> String {
    fn canonicalize(value: &Value) -> Value {
        match value {
            Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
            Value::Object(values) => serde_json::to_value(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), canonicalize(value)))
                    .collect::<BTreeMap<_, _>>(),
            )
            .unwrap_or(Value::Null),
            _ => value.clone(),
        }
    }
    serde_json::to_string(&canonicalize(value)).unwrap_or_else(|_| "null".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_context::ContextSensitivity;

    fn input<'a>(
        context: &'a CompiledModelContext,
        user_message: &str,
    ) -> ContextAssemblyInput<'a> {
        ContextAssemblyInput {
            model_context: context,
            context_summary: None,
            conversation: Vec::new(),
            user_message: user_message.to_string(),
            user_content: Vec::new(),
            tool_candidates: Vec::new(),
            previous_tool_calls: Vec::new(),
            tool_results: Vec::new(),
            previous_response_items: Vec::new(),
            previous_response_id: None,
            branch_developer_instructions: None,
            prompt_cache_breakpoint_policy: PromptCacheBreakpointPolicy::AppendOnlyUsers,
            final_output_json_schema: None,
        }
    }

    #[test]
    fn dynamic_turn_tail_does_not_rewrite_the_stable_prefix() {
        let context = CompiledModelContext {
            items: vec![ModelContextItem::text(
                ContextItemKind::BaseInstructions,
                ContextRole::System,
                "opentopia:base",
                "Stable instructions",
                ContextCacheScope::Stable,
                ContextSensitivity::Public,
            )],
            prompt_cache_key: Some("stable-lineage".to_string()),
        };
        let assembler = DefaultContextAssembler;

        let first = assembler
            .compile(input(&context, "first turn"))
            .expect("first request compiles");
        let second = assembler
            .compile(input(&context, "second turn"))
            .expect("second request compiles");

        assert_eq!(
            first.logical().instructions.prompt_cache_key,
            second.logical().instructions.prompt_cache_key
        );
        assert_eq!(
            first.materialized_context().items[0],
            second.materialized_context().items[0]
        );
        assert_eq!(
            first.manifest().stable_prefix_hash,
            second.manifest().stable_prefix_hash
        );
        assert_ne!(
            first.logical().input.current_user.message,
            second.logical().input.current_user.message
        );
        assert_ne!(
            first.manifest().dynamic_tail_hash,
            second.manifest().dynamic_tail_hash
        );
    }

    #[test]
    fn provider_transcript_state_is_promoted_out_of_native_response_items() {
        let context = CompiledModelContext::default();
        let transcript = crate::provider::ProviderWireTranscript {
            format: "test_wire_v1".to_string(),
            items: vec![json!({ "role": "user", "content": "prior" })],
        };
        let native_state = json!({
            "type": "reasoning",
            "id": "reasoning_1",
            "encrypted_content": "opaque",
        });
        let mut assembly = input(&context, "next");
        assembly.previous_response_items = vec![
            crate::provider::provider_transcript_state_item(&transcript),
            native_state.clone(),
        ];

        let request = DefaultContextAssembler
            .compile(assembly)
            .expect("request compiles");

        assert_eq!(request.logical().provider_transcript, Some(transcript));
        assert_eq!(
            request.logical().previous_response_items,
            vec![native_state]
        );
        assert!(serde_json::to_value(request.logical())
            .unwrap()
            .get("providerTranscript")
            .is_none());
    }

    #[test]
    fn assembler_is_an_object_safe_runtime_port() {
        let assembler: &dyn ContextAssembler = &DefaultContextAssembler;
        let request = assembler
            .compile(input(&CompiledModelContext::default(), "question"))
            .expect("request compiles");
        assert_eq!(request.logical().input.current_user.message, "question");
    }

    #[test]
    fn control_identity_changes_only_the_dynamic_tail() {
        let context = CompiledModelContext {
            items: vec![ModelContextItem::text(
                ContextItemKind::BaseInstructions,
                ContextRole::System,
                "opentopia:base",
                "Stable instructions",
                ContextCacheScope::Stable,
                ContextSensitivity::Public,
            )],
            prompt_cache_key: Some("stable-lineage".to_string()),
        };
        let mut first_input = input(&context, "same turn content");
        first_input.previous_tool_calls = vec![ProviderToolCall {
            id: "call_turn_a".to_string(),
            name: "read".to_string(),
            arguments: json!({"path": "README.md"}),
        }];
        let mut second_input = input(&context, "same turn content");
        second_input.previous_tool_calls = vec![ProviderToolCall {
            id: "call_turn_b".to_string(),
            name: "read".to_string(),
            arguments: json!({"path": "README.md"}),
        }];

        let first = DefaultContextAssembler
            .compile(first_input)
            .expect("first request");
        let second = DefaultContextAssembler
            .compile(second_input)
            .expect("second request");

        assert_eq!(
            first.manifest().stable_prefix_hash,
            second.manifest().stable_prefix_hash
        );
        assert_ne!(
            first.manifest().dynamic_tail_hash,
            second.manifest().dynamic_tail_hash
        );
        assert_eq!(
            first.materialized_context().items.last().unwrap().source,
            "round_tool_call:0"
        );
        assert_eq!(
            second.materialized_context().items.last().unwrap().source,
            "round_tool_call:0"
        );
    }

    #[test]
    fn tool_catalog_order_is_canonical() {
        let context = CompiledModelContext::default();
        let tool = |name: &str| ProviderToolCandidate::direct(name, name, json!({"type":"object"}));
        let mut first_input = input(&context, "question");
        first_input.tool_candidates = vec![tool("zeta"), tool("alpha")];
        let mut second_input = input(&context, "question");
        second_input.tool_candidates = vec![tool("alpha"), tool("zeta")];

        let first = DefaultContextAssembler.compile(first_input).expect("first");
        let second = DefaultContextAssembler
            .compile(second_input)
            .expect("second");

        assert_eq!(
            first.manifest().stable_prefix_hash,
            second.manifest().stable_prefix_hash
        );
        assert_eq!(
            first
                .logical()
                .tool_candidates
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "zeta"]
        );
    }

    #[test]
    fn changing_tool_catalog_changes_only_the_dynamic_tail() {
        let context = CompiledModelContext {
            items: vec![ModelContextItem::text(
                ContextItemKind::BaseInstructions,
                ContextRole::System,
                "opentopia:base",
                "Stable instructions",
                ContextCacheScope::Stable,
                ContextSensitivity::Public,
            )],
            prompt_cache_key: Some("stable-lineage".to_string()),
        };
        let tool = |name: &str| ProviderToolCandidate::direct(name, name, json!({"type":"object"}));
        let mut first_input = input(&context, "question");
        first_input.tool_candidates = vec![tool("read_file")];
        let mut second_input = input(&context, "question");
        second_input.tool_candidates = vec![tool("read_file"), tool("mcp_dynamic")];

        let first = DefaultContextAssembler.compile(first_input).expect("first");
        let second = DefaultContextAssembler
            .compile(second_input)
            .expect("second");
        assert_eq!(
            first.manifest().stable_prefix_hash,
            second.manifest().stable_prefix_hash
        );
        assert_ne!(
            first.manifest().dynamic_tail_hash,
            second.manifest().dynamic_tail_hash
        );
    }

    #[test]
    fn changing_branch_instructions_changes_only_the_dynamic_tail() {
        let context = CompiledModelContext {
            items: vec![ModelContextItem::text(
                ContextItemKind::BaseInstructions,
                ContextRole::System,
                "opentopia:base",
                "Stable instructions",
                ContextCacheScope::Stable,
                ContextSensitivity::Public,
            )],
            prompt_cache_key: Some("stable-lineage".to_string()),
        };
        let mut first_input = input(&context, "question");
        first_input.branch_developer_instructions = Some("Plan interaction profile".to_string());
        let mut second_input = input(&context, "question");
        second_input.branch_developer_instructions = Some("Goal interaction profile".to_string());

        let first = DefaultContextAssembler.compile(first_input).expect("first");
        let second = DefaultContextAssembler
            .compile(second_input)
            .expect("second");
        assert_eq!(
            first.manifest().stable_prefix_hash,
            second.manifest().stable_prefix_hash
        );
        assert_ne!(
            first.manifest().dynamic_tail_hash,
            second.manifest().dynamic_tail_hash
        );
        assert!(first
            .materialized_context()
            .items
            .iter()
            .any(|item| item.source == "opentopia:execution_branch"
                && item.cache_scope == ContextCacheScope::Turn));
    }

    #[test]
    fn a_new_turn_preserves_every_prior_provider_prefix_segment() {
        let context = CompiledModelContext {
            items: vec![ModelContextItem::text(
                ContextItemKind::BaseInstructions,
                ContextRole::System,
                "opentopia:base",
                "Stable instructions",
                ContextCacheScope::Stable,
                ContextSensitivity::Public,
            )],
            prompt_cache_key: Some("stable-lineage".to_string()),
        };
        let first = DefaultContextAssembler
            .compile(input(&context, "first user message"))
            .expect("first turn");
        let mut second_input = input(&context, "second user message");
        second_input.conversation = vec![
            ModelConversationMessage {
                role: ModelConversationRole::User,
                content: "first user message".to_string(),
                content_parts: Vec::new(),
                tool_calls: Vec::new(),
                tool_results: Vec::new(),
            },
            ModelConversationMessage {
                role: ModelConversationRole::Assistant,
                content: "first answer".to_string(),
                content_parts: Vec::new(),
                tool_calls: Vec::new(),
                tool_results: Vec::new(),
            },
        ];
        let second = DefaultContextAssembler
            .compile(second_input)
            .expect("second turn");

        assert_eq!(
            first.manifest().provider_prefix_segments,
            second.manifest().provider_prefix_segments
                [..first.manifest().provider_prefix_segments.len()]
        );
    }

    #[test]
    fn context_preparation_is_idempotent() {
        let base = CompiledModelContext {
            items: vec![ModelContextItem::text(
                ContextItemKind::BaseInstructions,
                ContextRole::System,
                "opentopia:base",
                "Stable instructions",
                ContextCacheScope::Stable,
                ContextSensitivity::Public,
            )],
            prompt_cache_key: None,
        };
        let tools = vec![ProviderToolCandidate::direct(
            "tool_search",
            "search tools",
            json!({"type":"object"}),
        )];
        let assembler = DefaultContextAssembler;
        let first = assembler
            .prepare_context(ContextPreparationInput {
                model_context: &base,
                context_summary: None,
                tool_candidates: &tools,
                lineage_instructions: Some("thread policy"),
            })
            .expect("first preparation");
        let second = assembler
            .prepare_context(ContextPreparationInput {
                model_context: &first,
                context_summary: None,
                tool_candidates: &tools,
                lineage_instructions: Some("thread policy"),
            })
            .expect("second preparation");

        assert_eq!(first, second);
        assert_eq!(
            second
                .items
                .iter()
                .filter(|item| item.source == "opentopia:execution_lineage")
                .count(),
            1
        );
        assert_eq!(
            second
                .items
                .iter()
                .filter(|item| item.source == "opentopia:tool_search_protocol")
                .count(),
            1
        );
    }

    #[test]
    fn generated_prompt_cache_lineage_can_be_refreshed_without_hash_chaining() {
        let base = CompiledModelContext {
            items: vec![ModelContextItem::text(
                ContextItemKind::BaseInstructions,
                ContextRole::System,
                "opentopia:base",
                "Stable instructions",
                ContextCacheScope::Stable,
                ContextSensitivity::Public,
            )],
            prompt_cache_key: None,
        };
        let prepared = DefaultContextAssembler
            .prepare_context(ContextPreparationInput {
                model_context: &base,
                context_summary: Some("checkpoint-a"),
                tool_candidates: &[],
                lineage_instructions: None,
            })
            .expect("prepare context");

        let same_checkpoint = prompt_cache_lineage_key(&prepared, Some("checkpoint-a"), &[]);
        let different_checkpoint = prompt_cache_lineage_key(&prepared, Some("checkpoint-b"), &[]);
        assert_eq!(
            prepared.prompt_cache_key.as_deref(),
            Some(same_checkpoint.as_str())
        );
        assert_ne!(
            prepared.prompt_cache_key.as_deref(),
            Some(different_checkpoint.as_str())
        );
    }
}
