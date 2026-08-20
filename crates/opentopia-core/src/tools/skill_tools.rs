use super::{
    decode_typed_tool_input, derived_tool_schema, enforce_policy_decision, tool_resource_key,
    truncate_chars, Tool, ToolExecutionPolicy, ToolInvocationContext, TypedTool,
};
use crate::execution_authorization::ToolExecutionIntent;
use crate::model::{ModelContentPart, ToolCall, ToolResult};
use crate::policy::PolicyDecision;
use crate::skill_authoring::{
    create_skill_from_draft, preview_skill_draft, skill_target_path, SkillDraft, SkillResourceDraft,
};
use crate::skills::{discover_skills, load_skill_slice, SkillScope, MAX_SKILL_BYTES};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyToolInput {}

pub struct ListSkillsTool;

#[async_trait]
impl TypedTool for ListSkillsTool {
    type Input = EmptyToolInput;

    fn name(&self) -> &str {
        "list_skills"
    }

    fn description(&self) -> &str {
        "List available capability instructions (Skills) without loading their instructions."
    }

    fn execution_policy(&self, _input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec!["skills:catalog".to_string()])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        _input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let skills = discover_skills(Some(&ctx.workspace_root))
            .into_iter()
            .filter(|skill| {
                ctx.capability_projection.allows_skill(&skill.id)
                    && skill
                        .plugin_id
                        .as_ref()
                        .is_none_or(|plugin_id| ctx.capability_projection.allows_plugin(plugin_id))
            })
            .collect::<Vec<_>>();
        let value = serde_json::to_value(&skills)?;
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string_pretty(&value)?,
            content: vec![ModelContentPart::json(value)],
            metadata: json!({ "count": skills.len() }),
        })
    }
}

impl_typed_tool!(ListSkillsTool);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadSkillInput {
    /// Skill ID returned by list_skills.
    pub(super) id: String,
    /// Byte offset to start reading from. Defaults to 0.
    #[serde(default)]
    pub(super) offset: u64,
    /// Maximum bytes to return, capped at 65536.
    #[serde(default)]
    #[schemars(range(min = 1))]
    pub(super) limit: Option<u64>,
}

pub struct ReadSkillTool;

#[async_trait]
impl TypedTool for ReadSkillTool {
    type Input = ReadSkillInput;

    fn name(&self) -> &str {
        "read_skill"
    }

    fn description(&self) -> &str {
        "Read one Skill's instructions after deciding it is relevant to the current task. Returns at most 64 KB per call; when the result reports a next offset, call again with that offset to read the rest."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![tool_resource_key("skill", &input.id)])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let id = input.id.trim();
        anyhow::ensure!(!id.is_empty(), "read_skill id must be a non-empty string");
        anyhow::ensure!(
            ctx.capability_projection.allows_skill(id),
            "Skill is outside the active ExecutionContext projection: {id}"
        );
        let limit = input.limit.map_or(MAX_SKILL_BYTES, |value| {
            (value as usize).min(MAX_SKILL_BYTES)
        });
        // load_skill_slice resolves the opaque ID against the bounded, canonicalized Skill
        // catalog. It cannot be used as a general-purpose path read, including for user Skills
        // that intentionally live outside the thread workspace.
        let slice = load_skill_slice(Some(&ctx.workspace_root), id, input.offset, limit)?;
        if let Some(plugin_id) = slice.descriptor.plugin_id.as_ref() {
            anyhow::ensure!(
                ctx.capability_projection.allows_plugin(plugin_id),
                "Skill plugin is outside the active ExecutionContext projection: {plugin_id}"
            );
        }
        let output = slice.render_for_model();
        Ok(ToolResult {
            call_id,
            output: output.clone(),
            content: vec![ModelContentPart::text(output)],
            metadata: json!({
                "id": slice.descriptor.id,
                "name": slice.descriptor.name,
                "path": slice.descriptor.path,
                "offset": slice.offset,
                "nextOffset": slice.next_offset,
                "totalBytes": slice.total_bytes
            }),
        })
    }
}

impl_typed_tool!(ReadSkillTool);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSkillToolInput {
    /// Short action-oriented lowercase hyphen-case name, at most 64 characters.
    name: String,
    /// What the Skill does and the concrete situations that should trigger it.
    description: String,
    /// Concise imperative Markdown for another agent.
    instructions: String,
    /// Installation scope. Defaults to user.
    #[serde(default)]
    scope: Option<SkillScope>,
    /// Optional human-facing title.
    #[serde(default)]
    display_name: Option<String>,
    /// Optional UI summary, at most 64 characters.
    #[serde(default)]
    short_description: Option<String>,
    /// Optional one-sentence example that mentions the Skill.
    #[serde(default)]
    default_prompt: Option<String>,
    /// Optional UTF-8 text resources.
    #[serde(default)]
    #[schemars(length(max = 24))]
    resources: Vec<SkillResourceDraft>,
}

pub struct CreateSkillTool;

#[async_trait]
impl TypedTool for CreateSkillTool {
    type Input = CreateSkillToolInput;

    fn name(&self) -> &str {
        "create_skill"
    }

    fn description(&self) -> &str {
        "Create a reusable Skill directly from the current conversation. Use when the user asks to summarize, preserve, or turn the current work into a Skill. Synthesize concise instructions and any materially useful resources from conversation context, then call this tool without a separate draft/review workflow. Default to a user Skill unless the user explicitly asks for the current project. After success, tell the user the Skill name, purpose, path, and files created."
    }

    fn execution_intent(&self, input: &Self::Input, workspace_root: &Path) -> ToolExecutionIntent {
        let scope = input.scope.unwrap_or(SkillScope::User);
        let workspace = (scope == SkillScope::Workspace).then_some(workspace_root);
        let paths = skill_target_path(scope, workspace, input.name.trim())
            .ok()
            .into_iter();
        ToolExecutionIntent::workspace_mutation(paths)
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let scope = input.scope.unwrap_or(SkillScope::User);
        let name = input.name.trim().to_ascii_lowercase();
        let description = input.description.trim().to_string();
        let draft = SkillDraft {
            display_name: input
                .display_name
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| skill_display_name(&name)),
            short_description: input
                .short_description
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| truncate_chars(&description, 64)),
            default_prompt: input
                .default_prompt
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("Use ${name} to apply this reusable workflow.")),
            name,
            description,
            instructions: input.instructions,
            resources: input.resources,
        };
        let workspace_root =
            (scope == SkillScope::Workspace).then_some(ctx.workspace_root.as_path());
        let preview = preview_skill_draft(draft.clone(), scope, workspace_root)?;
        enforce_policy_decision(ctx.policy.inspect_write(&preview.target_path), &ctx)?;
        let created = create_skill_from_draft(draft, scope, workspace_root)?;
        let files = created
            .files
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        let output = format!(
            "Created Skill `{}`.\nScope: {}\nPurpose: {}\nPath: {}\nFiles:\n- {}",
            created.skill.name,
            match scope {
                SkillScope::Workspace => "workspace",
                SkillScope::User => "user",
            },
            created.skill.description,
            created.skill.path.display(),
            files.join("\n- ")
        );
        let skill = serde_json::to_value(&created.skill)?;
        Ok(ToolResult::text(
            call_id,
            output,
            json!({
                "success": true,
                "createdSkill": skill,
                "changedPath": created.skill.path,
                "changedPaths": files,
                "fileCount": created.files.len()
            }),
        ))
    }
}

impl_typed_tool!(CreateSkillTool);

fn skill_display_name(name: &str) -> String {
    name.split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
