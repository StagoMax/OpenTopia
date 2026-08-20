use crate::settings::{ProviderAdapterKind, ProviderReasoningProtocol};
use serde_json::{json, Value};

/// Side effects imposed by a negotiated reasoning envelope. The request
/// builder consumes these flags; model-family knowledge never leaks into it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct AppliedReasoning {
    pub(super) enabled: bool,
    pub(super) omit_tool_choice: bool,
    pub(super) omit_temperature: bool,
}

/// Applies one persisted wire contract. `fallback_effort` is used by
/// capability probes and by thinking envelopes that require an explicit switch
/// even when the user has not selected an effort.
pub(super) fn apply_reasoning_protocol(
    protocol: ProviderReasoningProtocol,
    configured_effort: Option<&str>,
    fallback_effort: Option<&str>,
    payload: &mut Value,
) -> AppliedReasoning {
    let effort = configured_effort
        .filter(|value| !value.is_empty())
        .or(fallback_effort.filter(|value| !value.is_empty()));
    match protocol {
        ProviderReasoningProtocol::Omit => AppliedReasoning::default(),
        ProviderReasoningProtocol::ChatReasoningEffort => {
            if let Some(effort) = effort {
                payload["reasoning_effort"] = json!(effort);
            }
            AppliedReasoning {
                enabled: effort.is_some_and(reasoning_is_enabled),
                omit_temperature: effort.is_some_and(reasoning_is_enabled),
                ..AppliedReasoning::default()
            }
        }
        ProviderReasoningProtocol::ChatThinkingReasoningEffort => {
            let effort = effort.unwrap_or("high");
            if reasoning_is_enabled(effort) {
                payload["thinking"] = json!({ "type": "enabled" });
                payload["reasoning_effort"] = json!(effort);
                AppliedReasoning {
                    enabled: true,
                    omit_temperature: true,
                    ..AppliedReasoning::default()
                }
            } else {
                payload["thinking"] = json!({ "type": "disabled" });
                AppliedReasoning::default()
            }
        }
        ProviderReasoningProtocol::ChatThinkingHighMaxNoToolChoice => {
            let effort = effort.unwrap_or("high");
            if reasoning_is_enabled(effort) {
                payload["thinking"] = json!({ "type": "enabled" });
                payload["reasoning_effort"] = json!(high_or_max_effort(effort));
                AppliedReasoning {
                    enabled: true,
                    omit_tool_choice: true,
                    omit_temperature: true,
                }
            } else {
                payload["thinking"] = json!({ "type": "disabled" });
                AppliedReasoning::default()
            }
        }
        ProviderReasoningProtocol::ResponsesReasoning => {
            if let Some(effort) = effort {
                payload["reasoning"] = json!({ "effort": effort });
            }
            AppliedReasoning {
                enabled: effort.is_some_and(reasoning_is_enabled),
                omit_temperature: effort.is_some_and(reasoning_is_enabled),
                ..AppliedReasoning::default()
            }
        }
    }
}

/// Candidate ordering is only a probe optimization. Every candidate is a
/// structural wire envelope and the successful value is persisted in the
/// adapter profile; production requests never repeat this name-based hint.
pub(super) fn reasoning_probe_candidates(
    adapter: ProviderAdapterKind,
    model: &str,
) -> Vec<ProviderReasoningProtocol> {
    match adapter {
        ProviderAdapterKind::OpenAiChat => {
            let mut candidates = Vec::with_capacity(4);
            push_unique(&mut candidates, chat_probe_hint(model));
            push_unique(
                &mut candidates,
                ProviderReasoningProtocol::ChatReasoningEffort,
            );
            push_unique(
                &mut candidates,
                ProviderReasoningProtocol::ChatThinkingReasoningEffort,
            );
            push_unique(
                &mut candidates,
                ProviderReasoningProtocol::ChatThinkingHighMaxNoToolChoice,
            );
            push_unique(&mut candidates, ProviderReasoningProtocol::Omit);
            candidates
        }
        ProviderAdapterKind::OpenAiResponses => vec![
            ProviderReasoningProtocol::ResponsesReasoning,
            ProviderReasoningProtocol::Omit,
        ],
        _ => vec![ProviderReasoningProtocol::Omit],
    }
}

pub(super) fn default_reasoning_protocol(
    adapter: ProviderAdapterKind,
    _model: &str,
) -> ProviderReasoningProtocol {
    match adapter {
        ProviderAdapterKind::OpenAiChat => ProviderReasoningProtocol::ChatReasoningEffort,
        ProviderAdapterKind::OpenAiResponses => ProviderReasoningProtocol::ResponsesReasoning,
        _ => ProviderReasoningProtocol::Omit,
    }
}

pub(super) fn reasoning_protocol_label(protocol: ProviderReasoningProtocol) -> &'static str {
    match protocol {
        ProviderReasoningProtocol::Omit => "omit reasoning fields",
        ProviderReasoningProtocol::ChatReasoningEffort => "Chat reasoning_effort",
        ProviderReasoningProtocol::ChatThinkingReasoningEffort => {
            "Chat thinking + reasoning_effort"
        }
        ProviderReasoningProtocol::ChatThinkingHighMaxNoToolChoice => {
            "Chat thinking + high/max effort without tool_choice"
        }
        ProviderReasoningProtocol::ResponsesReasoning => "Responses reasoning.effort",
    }
}

fn chat_probe_hint(model: &str) -> ProviderReasoningProtocol {
    let model = normalized_model_name(model);
    if model.starts_with("deepseek-v4-flash")
        || model.starts_with("deepseek-v4-pro")
        || model.starts_with("deepseek-reasoner")
    {
        ProviderReasoningProtocol::ChatThinkingHighMaxNoToolChoice
    } else if model.starts_with("glm") || model.starts_with("chatglm") {
        ProviderReasoningProtocol::ChatThinkingReasoningEffort
    } else {
        ProviderReasoningProtocol::ChatReasoningEffort
    }
}

fn normalized_model_name(model: &str) -> String {
    let model = model.trim().to_ascii_lowercase();
    let model = model.rsplit('/').next().unwrap_or(&model);
    model.split(':').next().unwrap_or(model).to_string()
}

fn push_unique(
    candidates: &mut Vec<ProviderReasoningProtocol>,
    candidate: ProviderReasoningProtocol,
) {
    if !candidates.contains(&candidate) {
        candidates.push(candidate);
    }
}

fn reasoning_is_enabled(effort: &str) -> bool {
    !matches!(effort, "none" | "minimal")
}

fn high_or_max_effort(effort: &str) -> &'static str {
    match effort {
        "xhigh" | "max" => "max",
        _ => "high",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_models_receive_every_chat_envelope_candidate() {
        let candidates =
            reasoning_probe_candidates(ProviderAdapterKind::OpenAiChat, "future-model-v1");
        assert_eq!(
            candidates,
            vec![
                ProviderReasoningProtocol::ChatReasoningEffort,
                ProviderReasoningProtocol::ChatThinkingReasoningEffort,
                ProviderReasoningProtocol::ChatThinkingHighMaxNoToolChoice,
                ProviderReasoningProtocol::Omit,
            ]
        );
    }

    #[test]
    fn thinking_constraints_are_data_driven_by_the_protocol() {
        let mut payload = json!({"tool_choice": "auto", "temperature": 0.4});
        let applied = apply_reasoning_protocol(
            ProviderReasoningProtocol::ChatThinkingHighMaxNoToolChoice,
            Some("xhigh"),
            None,
            &mut payload,
        );
        assert_eq!(payload["reasoning_effort"], "max");
        assert!(applied.omit_tool_choice);
        assert!(applied.omit_temperature);
    }
}
