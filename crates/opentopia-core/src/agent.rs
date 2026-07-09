use crate::model::{AgentEventPayload, Message, MessageRole, ToolCall};
use crate::policy::{BasicPolicyEngine, PermissionMode};
use crate::provider::{MockProvider, ModelProvider, ModelRequest, OpenAiCompatibleProvider};
use crate::tools::{ToolContext, ToolRegistry};
use anyhow::Context;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AgentCore {
    provider: Arc<dyn ModelProvider>,
    tools: ToolRegistry,
}

impl Default for AgentCore {
    fn default() -> Self {
        Self {
            provider: Arc::new(MockProvider),
            tools: ToolRegistry::with_builtins(),
        }
    }
}

impl AgentCore {
    pub fn from_env() -> Self {
        let provider: Arc<dyn ModelProvider> = OpenAiCompatibleProvider::from_env()
            .map(|provider| Arc::new(provider) as Arc<dyn ModelProvider>)
            .unwrap_or_else(|| Arc::new(MockProvider));
        Self {
            provider,
            tools: ToolRegistry::with_builtins(),
        }
    }

    pub fn new(provider: Arc<dyn ModelProvider>, tools: ToolRegistry) -> Self {
        Self { provider, tools }
    }

    pub async fn run_turn(&self, input: AgentTurnInput) -> anyhow::Result<Vec<AgentEventPayload>> {
        let mut events = Vec::new();
        events.push(AgentEventPayload::TurnStarted {
            user_message_id: input.user_message_id,
        });
        events.push(AgentEventPayload::ModelDelta {
            text: "Analyzing workspace context...\n".to_string(),
        });

        let policy = Arc::new(BasicPolicyEngine::new(
            input.workspace_root.clone(),
            input.permission_mode,
        ));
        let tool_ctx = ToolContext {
            workspace_root: input.workspace_root.clone(),
            policy,
        };

        if let Some(task) = ParsedTask::parse(&input.content) {
            let result = match self.execute_parsed_task(task, tool_ctx, &mut events).await {
                Ok(result) => result,
                Err(err) if err.to_string().contains("approval required") => {
                    let reason = err.to_string();
                    events.push(AgentEventPayload::ApprovalRequested {
                        approval_id: Uuid::new_v4(),
                        reason: reason.clone(),
                        action: input.content.clone(),
                    });
                    format!(
                        "This action needs approval before OpenTopia can continue.\n\n```text\n{}\n```",
                        reason
                    )
                }
                Err(err) => return Err(err),
            };
            let assistant_message = Message::text(input.thread_id, MessageRole::Assistant, result);
            events.push(AgentEventPayload::AssistantMessage {
                message: assistant_message,
            });
            events.push(AgentEventPayload::TurnFinished {
                summary: "Command task completed.".to_string(),
            });
            return Ok(events);
        }

        let listed_files = self
            .execute_tool("list_files", json!({ "path": "." }), tool_ctx.clone(), &mut events)
            .await?
            .output;

        let response = self
            .provider
            .complete(ModelRequest {
                system_prompt: "You are OpenTopia, a local-first coding agent.".to_string(),
                user_message: format!(
                    "User request:\n{}\n\nWorkspace root listing:\n{}",
                    input.content, listed_files
                ),
            })
            .await?;

        let text = format!(
            "{}\n\nI inspected the workspace root and found:\n\n```text\n{}\n```\n\nYou can also use deterministic local tools such as `/read`, `/write`, `/run`, `/diff`, and `/patch`.",
            response.text, listed_files
        );
        let assistant_message = Message::text(input.thread_id, MessageRole::Assistant, text);
        events.push(AgentEventPayload::AssistantMessage {
            message: assistant_message,
        });
        events.push(AgentEventPayload::TurnFinished {
            summary: "Mock agent turn completed.".to_string(),
        });

        Ok(events)
    }

    async fn execute_parsed_task(
        &self,
        task: ParsedTask,
        ctx: ToolContext,
        events: &mut Vec<AgentEventPayload>,
    ) -> anyhow::Result<String> {
        let result = match task {
            ParsedTask::List { path } => {
                self.execute_tool("list_files", json!({ "path": path }), ctx, events)
                    .await?
            }
            ParsedTask::Read { path } => {
                self.execute_tool("read_file", json!({ "path": path }), ctx, events)
                    .await?
            }
            ParsedTask::Write { path, content } => {
                let result = self
                    .execute_tool("write_file", json!({ "path": path, "content": content }), ctx, events)
                    .await?;
                if let Some(changed_path) = result.metadata.get("changedPath").and_then(|value| value.as_str()) {
                    events.push(AgentEventPayload::FileChanged {
                        path: changed_path.into(),
                        summary: "File written by write_file tool.".to_string(),
                    });
                }
                result
            }
            ParsedTask::Run { command } => {
                self.execute_tool("shell", json!({ "command": command }), ctx, events)
                    .await?
            }
            ParsedTask::Diff => {
                self.execute_tool("git_diff", json!({}), ctx, events).await?
            }
            ParsedTask::Patch { patch } => {
                let result = self
                    .execute_tool("apply_patch", json!({ "patch": patch }), ctx, events)
                    .await?;
                events.push(AgentEventPayload::FileChanged {
                    path: input_placeholder_path(),
                    summary: "Patch applied by apply_patch tool.".to_string(),
                });
                result
            }
        };

        Ok(format!(
            "Completed `{}`.\n\n```text\n{}\n```",
            result.metadata
                .get("toolName")
                .and_then(|value| value.as_str())
                .unwrap_or("tool"),
            result.output
        ))
    }

    async fn execute_tool(
        &self,
        name: &str,
        input: serde_json::Value,
        ctx: ToolContext,
        events: &mut Vec<AgentEventPayload>,
    ) -> anyhow::Result<crate::model::ToolResult> {
        let call = ToolCall::new(name, input);
        events.push(AgentEventPayload::ToolCallStarted { call: call.clone() });
        let mut result = self
            .tools
            .get(name)
            .with_context(|| format!("{name} tool not registered"))?
            .execute(call, ctx)
            .await?;
        if let Some(object) = result.metadata.as_object_mut() {
            object.insert("toolName".to_string(), json!(name));
        }
        events.push(AgentEventPayload::ToolCallFinished {
            result: result.clone(),
        });
        Ok(result)
    }
}

#[derive(Debug, Clone)]
pub struct AgentTurnInput {
    pub thread_id: Uuid,
    pub user_message_id: Uuid,
    pub workspace_root: PathBuf,
    pub content: String,
    pub permission_mode: PermissionMode,
}

#[derive(Debug, Clone)]
enum ParsedTask {
    List { path: String },
    Read { path: String },
    Write { path: String, content: String },
    Run { command: String },
    Diff,
    Patch { patch: String },
}

impl ParsedTask {
    fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("/diff") || trimmed.eq_ignore_ascii_case("diff") {
            return Some(Self::Diff);
        }
        if let Some(path) = strip_command(trimmed, "/list").or_else(|| strip_command(trimmed, "list")) {
            return Some(Self::List {
                path: default_path(path),
            });
        }
        if let Some(path) = strip_command(trimmed, "/read").or_else(|| strip_command(trimmed, "read")) {
            return Some(Self::Read {
                path: path.trim().to_string(),
            });
        }
        if let Some(command) = strip_command(trimmed, "/run")
            .or_else(|| strip_command(trimmed, "run"))
            .or_else(|| strip_command(trimmed, "shell:"))
        {
            return Some(Self::Run {
                command: command.trim().to_string(),
            });
        }
        if let Some(rest) = strip_command(trimmed, "/write").or_else(|| strip_command(trimmed, "write")) {
            let mut lines = rest.lines();
            let path = lines.next()?.trim().to_string();
            let content = lines.collect::<Vec<_>>().join("\n");
            if !path.is_empty() {
                return Some(Self::Write { path, content });
            }
        }
        if let Some(patch) = strip_command(trimmed, "/patch").or_else(|| strip_command(trimmed, "patch")) {
            if !patch.trim().is_empty() {
                return Some(Self::Patch {
                    patch: patch.to_string(),
                });
            }
        }
        None
    }
}

fn input_placeholder_path() -> PathBuf {
    PathBuf::from(".")
}

fn strip_command<'a>(input: &'a str, command: &str) -> Option<&'a str> {
    if command.ends_with(':') {
        return input.strip_prefix(command).map(str::trim).filter(|value| !value.is_empty());
    }

    if input == command {
        return Some("");
    }

    input.strip_prefix(command).and_then(|rest| {
        if rest.chars().next().is_some_and(char::is_whitespace) {
            Some(rest.trim())
        } else {
            None
        }
    })
}

fn default_path(path: &str) -> String {
    if path.trim().is_empty() {
        ".".to_string()
    } else {
        path.trim().to_string()
    }
}
