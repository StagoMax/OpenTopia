use super::*;
use crate::model_context::{content_fingerprint, TokenEstimateDetail};
use crate::settings::{OpenAiProtocol, PromptCachePolicy};
use crate::tools::{SpreadsheetTool, Tool};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

fn model_request() -> ModelRequest {
    ModelRequest {
        instructions: CompiledModelContext {
            items: vec![ModelContextItem::text(
                ContextItemKind::BaseInstructions,
                ContextRole::System,
                "test:system",
                "system",
                ContextCacheScope::Stable,
                crate::model_context::ContextSensitivity::Public,
            )],
            prompt_cache_key: None,
        },
        input: ModelInputLedger {
            current_user: ModelUserInput {
                message: "current".to_string(),
                content: Vec::new(),
            },
            ..Default::default()
        },
        tool_candidates: Vec::new(),
        previous_response_items: Vec::new(),
        provider_transcript: None,
        previous_response_id: None,
        prompt_cache_breakpoint_policy: PromptCacheBreakpointPolicy::StableOnly,
        final_output_json_schema: None,
    }
}

fn tool_request() -> ModelRequest {
    let mut request = model_request();
    request.tool_candidates = vec![ProviderToolCandidate::direct(
        "lookup",
        "Look up a value",
        json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"]
        }),
    )];
    request
}

include!("tests/fundamentals.rs");
include!("tests/protocol.rs");
include!("tests/chat_codec.rs");
include!("tests/stream_transport.rs");
include!("tests/capability.rs");
include!("tests/release_gate.rs");
