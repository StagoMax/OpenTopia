mod chat;
mod responses;
mod shared;

pub(in crate::provider) use chat::{
    compile_openai_tools, legacy_tool_observation, normalize_provider_tool_calls,
    openai_messages_with_reasoning, openai_portable_messages_with_reasoning,
    responses_system_instructions, OPENAI_CHAT_ASSISTANT_STATE_TYPE,
    OPENAI_CHAT_NATIVE_TRANSCRIPT_FORMAT, OPENAI_CHAT_PORTABLE_TRANSCRIPT_FORMAT,
};
#[cfg(test)]
pub(in crate::provider) use chat::{
    normalize_provider_arguments, openai_messages, openai_portable_messages,
    openai_strict_function_schema, openai_tools,
};

#[cfg(test)]
pub(in crate::provider) use responses::responses_tools;
pub(in crate::provider) use responses::{
    add_responses_prompt_cache_breakpoint, compile_responses_tools, responses_input,
};

#[cfg(test)]
pub(in crate::provider) use shared::responses_tool_result_output;
pub(in crate::provider) use shared::{
    nonredundant_tool_result_content, provider_tool_result_content,
};
