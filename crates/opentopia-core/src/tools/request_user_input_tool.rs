use super::{
    decode_typed_tool_input, derived_tool_schema, enforce_policy_decision, Tool,
    ToolExecutionPolicy, ToolInvocationContext, TypedTool,
};
use crate::execution_authorization::ToolExecutionIntent;
use crate::model::{
    CollaborationMode, ModelContentPart, ToolCall, ToolResult, UserInputOption, UserInputQuestion,
    UserInputRequest,
};
use crate::policy::PolicyDecision;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use uuid::Uuid;

const MAX_USER_INPUT_QUESTIONS: usize = 3;
const MAX_USER_INPUT_OPTIONS: usize = 3;
const MAX_USER_INPUT_ID_CHARS: usize = 64;
const MAX_USER_INPUT_HEADER_CHARS: usize = 24;
const MAX_USER_INPUT_QUESTION_CHARS: usize = 500;
const MAX_USER_INPUT_LABEL_CHARS: usize = 100;
const MAX_USER_INPUT_DESCRIPTION_CHARS: usize = 500;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestUserInputInput {
    /// One to three concise user decisions.
    #[schemars(length(min = 1, max = 3))]
    questions: Vec<RequestUserInputQuestionInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RequestUserInputQuestionInput {
    /// Legacy caller-supplied identifier. New calls receive a compact stable ID.
    #[serde(default)]
    id: Option<String>,
    /// Short card heading.
    header: String,
    question: String,
    #[schemars(length(min = 2, max = 3))]
    options: Vec<RequestUserInputOptionInput>,
    /// Zero-based recommended option index. Omit when no option is preferred.
    #[serde(default)]
    recommended: Option<usize>,
    /// Legacy override. New calls always allow a custom answer.
    #[serde(default = "default_allow_custom")]
    allow_custom: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RequestUserInputOptionInput {
    /// Legacy caller-supplied identifier. New calls receive a compact stable ID.
    #[serde(default)]
    id: Option<String>,
    label: String,
    description: String,
    /// Legacy per-option recommendation flag.
    #[serde(default)]
    recommended: bool,
}

fn default_allow_custom() -> bool {
    true
}

pub struct RequestUserInputTool;

#[async_trait]
impl TypedTool for RequestUserInputTool {
    type Input = RequestUserInputInput;

    fn name(&self) -> &str {
        "request_user_input"
    }

    fn description(&self) -> &str {
        "In Plan mode, pause the current Turn when materially different approaches require a user choice. Provide one to three concise questions with two to three labeled options; IDs and custom-answer support are generated automatically. Set an optional zero-based recommended index on a question. The same Turn resumes after the answer."
    }

    fn validate_context(&self, ctx: &ToolInvocationContext) -> anyhow::Result<()> {
        anyhow::ensure!(
            ctx.agent_path == "/root",
            "only the root agent may ask the user a structured decision question"
        );
        anyhow::ensure!(
            ctx.collaboration_mode == CollaborationMode::Plan,
            "request_user_input is only available in Plan mode"
        );
        Ok(())
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        _ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        anyhow::ensure!(
            !input.questions.is_empty() && input.questions.len() <= MAX_USER_INPUT_QUESTIONS,
            "request_user_input requires one to {MAX_USER_INPUT_QUESTIONS} questions"
        );

        let mut question_ids = HashSet::new();
        let mut questions = Vec::with_capacity(input.questions.len());
        for (question_index, question) in input.questions.into_iter().enumerate() {
            let id = question
                .id
                .map(|id| validate_user_input_id("question id", id))
                .transpose()?
                .unwrap_or_else(|| format!("q{}", question_index + 1));
            anyhow::ensure!(
                question_ids.insert(id.clone()),
                "duplicate question id: {id}"
            );
            let header =
                validate_user_input_text("header", question.header, MAX_USER_INPUT_HEADER_CHARS)?;
            let prompt = validate_user_input_text(
                "question",
                question.question,
                MAX_USER_INPUT_QUESTION_CHARS,
            )?;
            anyhow::ensure!(
                (2..=MAX_USER_INPUT_OPTIONS).contains(&question.options.len()),
                "question {id} requires two to {MAX_USER_INPUT_OPTIONS} options"
            );

            let mut option_ids = HashSet::new();
            let mut option_labels = HashSet::new();
            if let Some(recommended) = question.recommended {
                anyhow::ensure!(
                    recommended < question.options.len(),
                    "question {id} recommended index {recommended} is out of range"
                );
            }
            let mut recommended_count = 0usize;
            let mut options = Vec::with_capacity(question.options.len());
            for (option_index, option) in question.options.into_iter().enumerate() {
                let option_id = option
                    .id
                    .map(|id| validate_user_input_id("option id", id))
                    .transpose()?
                    .unwrap_or_else(|| format!("o{}", option_index + 1));
                anyhow::ensure!(
                    option_ids.insert(option_id.clone()),
                    "question {id} contains duplicate option id: {option_id}"
                );
                let label = validate_user_input_text(
                    "option label",
                    option.label,
                    MAX_USER_INPUT_LABEL_CHARS,
                )?;
                anyhow::ensure!(
                    option_labels.insert(label.to_lowercase()),
                    "question {id} contains duplicate option label: {label}"
                );
                let description = validate_user_input_text(
                    "option description",
                    option.description,
                    MAX_USER_INPUT_DESCRIPTION_CHARS,
                )?;
                let recommended = question
                    .recommended
                    .map(|index| index == option_index)
                    .unwrap_or(option.recommended);
                recommended_count += usize::from(recommended);
                options.push(UserInputOption {
                    id: option_id,
                    label,
                    description,
                    recommended,
                });
            }
            anyhow::ensure!(
                recommended_count <= 1,
                "question {id} may have at most one recommended option"
            );
            questions.push(UserInputQuestion {
                id,
                header,
                question: prompt,
                options,
                allow_custom: question.allow_custom,
            });
        }

        let request = UserInputRequest {
            request_id: Uuid::new_v4(),
            questions,
        };
        Ok(ToolResult {
            call_id,
            output: format!(
                "Waiting for the user to answer {} planning decision(s).",
                request.questions.len()
            ),
            content: vec![ModelContentPart::json(json!({
                "status": "waiting_for_user_input",
                "requestId": request.request_id,
            }))],
            metadata: json!({
                "toolName": "request_user_input",
                "userInputRequest": request,
                "success": true,
            }),
        })
    }
}

impl_typed_tool!(RequestUserInputTool);

fn validate_user_input_id(field: &str, value: String) -> anyhow::Result<String> {
    let value = validate_user_input_text(field, value, MAX_USER_INPUT_ID_CHARS)?;
    anyhow::ensure!(
        value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-')),
        "request_user_input {field} must contain only letters, numbers, underscores, or hyphens"
    );
    Ok(value)
}

fn validate_user_input_text(
    field: &str,
    value: String,
    max_chars: usize,
) -> anyhow::Result<String> {
    let value = value.trim().to_string();
    anyhow::ensure!(
        !value.is_empty(),
        "request_user_input {field} cannot be empty"
    );
    anyhow::ensure!(
        value.chars().count() <= max_chars,
        "request_user_input {field} exceeds the {max_chars} character limit"
    );
    Ok(value)
}
