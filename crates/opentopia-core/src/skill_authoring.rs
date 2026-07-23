use crate::skills::{descriptor_for_skill_file, SkillDescriptor, SkillScope};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

const MAX_PROMPT_CHARS: usize = 12_000;
const MAX_NAME_CHARS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 1_024;
const MAX_INSTRUCTIONS_BYTES: usize = 64 * 1024;
const MAX_DEFAULT_PROMPT_CHARS: usize = 512;
const MAX_RESOURCE_FILES: usize = 24;
const MAX_RESOURCE_FILE_BYTES: usize = 64 * 1024;
const MAX_RESOURCE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillResourceDraft {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillDraft {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub display_name: String,
    pub short_description: String,
    pub default_prompt: String,
    #[serde(default)]
    pub resources: Vec<SkillResourceDraft>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillDraftPreview {
    pub draft: SkillDraft,
    pub skill_md: String,
    pub openai_yaml: String,
    pub target_path: PathBuf,
    pub target_exists: bool,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreatedSkill {
    pub skill: SkillDescriptor,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Error)]
pub enum SkillAuthoringError {
    #[error("skill request is invalid: {0}")]
    InvalidRequest(String),
    #[error("generated skill draft is invalid: {0}")]
    InvalidDraft(String),
    #[error("model did not return a valid skill draft: {0}")]
    InvalidModelOutput(String),
    #[error("skill already exists: {0}")]
    Conflict(String),
    #[error("skill root is unavailable: {0}")]
    RootUnavailable(String),
    #[error("skill root escapes the selected workspace: {0}")]
    UnsafeRoot(String),
    #[error("skill could not be written: {0}")]
    Write(String),
}

pub fn validate_skill_generation_prompt(prompt: &str) -> Result<String, SkillAuthoringError> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(SkillAuthoringError::InvalidRequest(
            "describe what the Skill should do".to_string(),
        ));
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(SkillAuthoringError::InvalidRequest(format!(
            "description exceeds {MAX_PROMPT_CHARS} characters"
        )));
    }
    Ok(prompt.to_string())
}

pub fn skill_authoring_system_prompt() -> &'static str {
    r#"You create concise, production-ready Codex Skills from a user's natural-language request.
Return only JSON matching the supplied schema.

Requirements:
- Use a short, action-oriented ASCII skill name in lowercase hyphen-case, at most 64 characters.
- Put every trigger and 'when to use' condition in description, not in the body.
- Write the instructions as imperative Markdown for another coding agent. Include only non-obvious workflow, constraints, validation, and resource routing. Keep it concise and below 500 lines.
- Generate interface metadata for agents/openai.yaml. defaultPrompt must be one short sentence that explicitly mentions $<skill-name>.
- Add resources only when they materially improve repeatability. Put detailed knowledge in references/, deterministic helpers in scripts/, and reusable text templates in assets/. Keep references one level from SKILL.md and mention when to read them.
- Do not create README, changelog, installation, quick-reference, or other process documentation.
- Resource paths must be relative, use forward slashes, and start with references/, scripts/, or assets/. Resource contents must be UTF-8 text.
- Do not include SKILL.md or agents/openai.yaml in resources; the application creates them.
- Never include secrets, credentials, machine-specific absolute paths, TODO placeholders, or claims that validation already ran."#
}

pub fn skill_draft_json_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "name",
            "description",
            "instructions",
            "displayName",
            "shortDescription",
            "defaultPrompt",
            "resources"
        ],
        "properties": {
            "name": {
                "type": "string",
                "description": "Lowercase ASCII hyphen-case skill folder name, at most 64 characters."
            },
            "description": {
                "type": "string",
                "description": "What the Skill does and the situations or user requests that should trigger it."
            },
            "instructions": {
                "type": "string",
                "description": "Concise imperative Markdown body for SKILL.md, including a useful H1 heading."
            },
            "displayName": {
                "type": "string",
                "description": "Human-facing title for the Skill."
            },
            "shortDescription": {
                "type": "string",
                "description": "Short human-facing UI summary, at most 64 characters."
            },
            "defaultPrompt": {
                "type": "string",
                "description": "One-sentence example prompt that explicitly mentions $<skill-name>."
            },
            "resources": {
                "type": "array",
                "maxItems": MAX_RESOURCE_FILES,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["path", "content"],
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    }
                }
            }
        }
    })
}

pub fn parse_skill_draft_response(response: &str) -> Result<SkillDraft, SkillAuthoringError> {
    let response = response.trim();
    if response.is_empty() {
        return Err(SkillAuthoringError::InvalidModelOutput(
            "provider returned empty text".to_string(),
        ));
    }
    let parsed = serde_json::from_str::<SkillDraft>(response).or_else(|direct_error| {
        let start = response.find('{');
        let end = response.rfind('}');
        match (start, end) {
            (Some(start), Some(end)) if start < end => {
                serde_json::from_str::<SkillDraft>(&response[start..=end])
            }
            _ => Err(direct_error),
        }
    });
    let draft =
        parsed.map_err(|error| SkillAuthoringError::InvalidModelOutput(error.to_string()))?;
    validate_skill_draft(draft)
}

pub fn validate_skill_draft(mut draft: SkillDraft) -> Result<SkillDraft, SkillAuthoringError> {
    draft.name = draft.name.trim().to_ascii_lowercase();
    draft.description = draft.description.trim().to_string();
    draft.instructions = draft.instructions.trim().to_string();
    draft.display_name = draft.display_name.trim().to_string();
    draft.short_description = draft.short_description.trim().to_string();
    draft.default_prompt = draft.default_prompt.trim().to_string();
    for resource in &mut draft.resources {
        resource.path = resource.path.trim().replace('\\', "/");
    }

    validate_skill_name(&draft.name)?;
    validate_text_chars("description", &draft.description, MAX_DESCRIPTION_CHARS)?;
    validate_text_chars("displayName", &draft.display_name, MAX_NAME_CHARS)?;
    validate_text_chars("shortDescription", &draft.short_description, MAX_NAME_CHARS)?;
    validate_text_chars(
        "defaultPrompt",
        &draft.default_prompt,
        MAX_DEFAULT_PROMPT_CHARS,
    )?;
    if draft.instructions.is_empty() {
        return Err(SkillAuthoringError::InvalidDraft(
            "instructions cannot be empty".to_string(),
        ));
    }
    if draft.instructions.len() > MAX_INSTRUCTIONS_BYTES {
        return Err(SkillAuthoringError::InvalidDraft(format!(
            "instructions exceed {MAX_INSTRUCTIONS_BYTES} UTF-8 bytes"
        )));
    }
    let explicit_name = format!("${}", draft.name);
    if !draft.default_prompt.contains(&explicit_name) {
        return Err(SkillAuthoringError::InvalidDraft(format!(
            "defaultPrompt must mention {explicit_name}"
        )));
    }
    if draft.resources.len() > MAX_RESOURCE_FILES {
        return Err(SkillAuthoringError::InvalidDraft(format!(
            "resources contain more than {MAX_RESOURCE_FILES} files"
        )));
    }

    let mut seen = HashSet::new();
    let mut total_bytes = 0usize;
    for resource in &draft.resources {
        validate_resource_path(&resource.path)?;
        if !seen.insert(resource.path.to_ascii_lowercase()) {
            return Err(SkillAuthoringError::InvalidDraft(format!(
                "duplicate resource path: {}",
                resource.path
            )));
        }
        let bytes = resource.content.len();
        if bytes > MAX_RESOURCE_FILE_BYTES {
            return Err(SkillAuthoringError::InvalidDraft(format!(
                "resource '{}' exceeds {MAX_RESOURCE_FILE_BYTES} UTF-8 bytes",
                resource.path
            )));
        }
        total_bytes = total_bytes.saturating_add(bytes);
        if total_bytes > MAX_RESOURCE_BYTES {
            return Err(SkillAuthoringError::InvalidDraft(format!(
                "resources exceed {MAX_RESOURCE_BYTES} UTF-8 bytes in total"
            )));
        }
    }
    Ok(draft)
}

pub fn preview_skill_draft(
    draft: SkillDraft,
    scope: SkillScope,
    workspace_root: Option<&Path>,
) -> Result<SkillDraftPreview, SkillAuthoringError> {
    let draft = validate_skill_draft(draft)?;
    let root = skill_write_root(scope, workspace_root)?;
    let target_path = root.join(&draft.name);
    let target_exists = target_path.exists();
    let mut files = vec![
        PathBuf::from("SKILL.md"),
        PathBuf::from("agents/openai.yaml"),
    ];
    files.extend(
        draft
            .resources
            .iter()
            .map(|resource| PathBuf::from(&resource.path)),
    );
    Ok(SkillDraftPreview {
        skill_md: render_skill_md(&draft),
        openai_yaml: render_openai_yaml(&draft),
        draft,
        target_path,
        target_exists,
        files,
    })
}

pub fn create_skill_from_draft(
    draft: SkillDraft,
    scope: SkillScope,
    workspace_root: Option<&Path>,
) -> Result<CreatedSkill, SkillAuthoringError> {
    let preview = preview_skill_draft(draft, scope, workspace_root)?;
    let root = preview
        .target_path
        .parent()
        .ok_or_else(|| SkillAuthoringError::RootUnavailable("missing parent".to_string()))?;
    fs::create_dir_all(root)
        .map_err(|error| SkillAuthoringError::RootUnavailable(error.to_string()))?;
    let canonical_root = root
        .canonicalize()
        .map_err(|error| SkillAuthoringError::RootUnavailable(error.to_string()))?;
    if scope == SkillScope::Workspace {
        let workspace = workspace_root.ok_or_else(|| {
            SkillAuthoringError::InvalidRequest(
                "workspaceRoot is required for a workspace Skill".to_string(),
            )
        })?;
        let canonical_workspace = workspace
            .canonicalize()
            .map_err(|error| SkillAuthoringError::RootUnavailable(error.to_string()))?;
        if !canonical_root.starts_with(&canonical_workspace) {
            return Err(SkillAuthoringError::UnsafeRoot(
                canonical_root.display().to_string(),
            ));
        }
    }

    let target = canonical_root.join(&preview.draft.name);
    if target.exists() {
        return Err(SkillAuthoringError::Conflict(target.display().to_string()));
    }
    let staging = canonical_root.join(format!(".opentopia-skill-{}", Uuid::new_v4().simple()));
    fs::create_dir(&staging).map_err(|error| SkillAuthoringError::Write(error.to_string()))?;

    let write_result = (|| -> Result<(), SkillAuthoringError> {
        write_new_file(&staging.join("SKILL.md"), preview.skill_md.as_bytes())?;
        write_new_file(
            &staging.join("agents/openai.yaml"),
            preview.openai_yaml.as_bytes(),
        )?;
        for resource in &preview.draft.resources {
            write_new_file(
                &staging.join(Path::new(&resource.path)),
                resource.content.as_bytes(),
            )?;
        }
        fs::rename(&staging, &target).map_err(|error| {
            if error.kind() == ErrorKind::AlreadyExists || target.exists() {
                SkillAuthoringError::Conflict(target.display().to_string())
            } else {
                SkillAuthoringError::Write(error.to_string())
            }
        })?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    let skill_file = target.join("SKILL.md");
    let descriptor = descriptor_for_skill_file(skill_file, scope).map_err(|error| {
        SkillAuthoringError::Write(format!("created Skill cannot be discovered: {error}"))
    })?;
    let files = preview
        .files
        .into_iter()
        .map(|path| target.join(path))
        .collect();
    Ok(CreatedSkill {
        skill: descriptor,
        files,
    })
}

fn validate_skill_name(name: &str) -> Result<(), SkillAuthoringError> {
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(SkillAuthoringError::InvalidDraft(format!(
            "name must contain 1-{MAX_NAME_CHARS} characters"
        )));
    }
    let bytes = name.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        || !bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
    {
        return Err(SkillAuthoringError::InvalidDraft(
            "name must use lowercase ASCII letters, digits, and single hyphen separators"
                .to_string(),
        ));
    }
    if name.contains("--") {
        return Err(SkillAuthoringError::InvalidDraft(
            "name cannot contain consecutive hyphens".to_string(),
        ));
    }
    Ok(())
}

fn validate_text_chars(
    field: &str,
    value: &str,
    maximum: usize,
) -> Result<(), SkillAuthoringError> {
    let count = value.chars().count();
    if count == 0 || count > maximum {
        return Err(SkillAuthoringError::InvalidDraft(format!(
            "{field} must contain 1-{maximum} characters"
        )));
    }
    Ok(())
}

fn validate_resource_path(path: &str) -> Result<(), SkillAuthoringError> {
    if path.is_empty() || Path::new(path).is_absolute() || path.contains(':') {
        return Err(SkillAuthoringError::InvalidDraft(format!(
            "resource path must be relative: {path}"
        )));
    }
    let components = Path::new(path).components().collect::<Vec<_>>();
    if components.len() < 2
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(SkillAuthoringError::InvalidDraft(format!(
            "resource path is unsafe: {path}"
        )));
    }
    let first = components[0].as_os_str().to_string_lossy();
    if !matches!(first.as_ref(), "references" | "scripts" | "assets") {
        return Err(SkillAuthoringError::InvalidDraft(format!(
            "resource path must start with references/, scripts/, or assets/: {path}"
        )));
    }
    if components
        .iter()
        .any(|component| component.as_os_str().to_string_lossy().starts_with('.'))
    {
        return Err(SkillAuthoringError::InvalidDraft(format!(
            "hidden resource paths are not allowed: {path}"
        )));
    }
    Ok(())
}

fn skill_write_root(
    scope: SkillScope,
    workspace_root: Option<&Path>,
) -> Result<PathBuf, SkillAuthoringError> {
    match scope {
        SkillScope::Workspace => workspace_root
            .map(|workspace| workspace.join(".agents/skills"))
            .ok_or_else(|| {
                SkillAuthoringError::InvalidRequest(
                    "workspaceRoot is required for a workspace Skill".to_string(),
                )
            }),
        SkillScope::User => {
            if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
                return Ok(PathBuf::from(codex_home).join("skills"));
            }
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from)
                .map(|home| home.join(".codex/skills"))
                .ok_or_else(|| {
                    SkillAuthoringError::RootUnavailable(
                        "CODEX_HOME, USERPROFILE, and HOME are unset".to_string(),
                    )
                })
        }
    }
}

fn render_skill_md(draft: &SkillDraft) -> String {
    format!(
        "---\nname: {}\ndescription: {}\n---\n\n{}\n",
        yaml_string(&draft.name),
        yaml_string(&draft.description),
        draft.instructions
    )
}

fn render_openai_yaml(draft: &SkillDraft) -> String {
    format!(
        "interface:\n  display_name: {}\n  short_description: {}\n  default_prompt: {}\n",
        yaml_string(&draft.display_name),
        yaml_string(&draft.short_description),
        yaml_string(&draft.default_prompt)
    )
}

fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string cannot fail")
}

fn write_new_file(path: &Path, content: &[u8]) -> Result<(), SkillAuthoringError> {
    let parent = path
        .parent()
        .ok_or_else(|| SkillAuthoringError::Write("file has no parent".to_string()))?;
    fs::create_dir_all(parent).map_err(|error| SkillAuthoringError::Write(error.to_string()))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| SkillAuthoringError::Write(error.to_string()))?;
    file.write_all(content)
        .and_then(|_| file.sync_all())
        .map_err(|error| SkillAuthoringError::Write(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "opentopia-skill-authoring-{}",
                Uuid::new_v4().simple()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn draft() -> SkillDraft {
        SkillDraft {
            name: "review-api".to_string(),
            description: "Review HTTP API changes. Use for endpoint and schema reviews."
                .to_string(),
            instructions: "# Review API\n\nInspect contracts before implementation.".to_string(),
            display_name: "Review API".to_string(),
            short_description: "Review API contracts and changes".to_string(),
            default_prompt: "Use $review-api to review this endpoint change.".to_string(),
            resources: vec![SkillResourceDraft {
                path: "references/checklist.md".to_string(),
                content: "# Checklist\n\n- Check compatibility.".to_string(),
            }],
        }
    }

    #[test]
    fn parses_structured_output_wrapped_in_prose() {
        let json = serde_json::to_string(&draft()).unwrap();
        let parsed = parse_skill_draft_response(&format!("Draft:\n{json}\nDone.")).unwrap();
        assert_eq!(parsed.name, "review-api");
    }

    #[test]
    fn rejects_resource_traversal_and_missing_explicit_prompt_name() {
        let mut unsafe_draft = draft();
        unsafe_draft.resources[0].path = "references/../secret.md".to_string();
        assert!(matches!(
            validate_skill_draft(unsafe_draft),
            Err(SkillAuthoringError::InvalidDraft(_))
        ));

        let mut ambiguous = draft();
        ambiguous.default_prompt = "Review this API.".to_string();
        assert!(matches!(
            validate_skill_draft(ambiguous),
            Err(SkillAuthoringError::InvalidDraft(_))
        ));
    }

    #[test]
    fn creates_complete_workspace_skill_atomically_and_rejects_conflict() {
        let workspace = TestDir::new();
        let created =
            create_skill_from_draft(draft(), SkillScope::Workspace, Some(&workspace.0)).unwrap();

        assert_eq!(created.skill.name, "review-api");
        assert_eq!(created.skill.scope, SkillScope::Workspace);
        assert!(workspace
            .0
            .join(".agents/skills/review-api/SKILL.md")
            .is_file());
        assert!(workspace
            .0
            .join(".agents/skills/review-api/agents/openai.yaml")
            .is_file());
        assert!(workspace
            .0
            .join(".agents/skills/review-api/references/checklist.md")
            .is_file());
        assert!(matches!(
            create_skill_from_draft(draft(), SkillScope::Workspace, Some(&workspace.0)),
            Err(SkillAuthoringError::Conflict(_))
        ));
        assert!(fs::read_dir(workspace.0.join(".agents/skills"))
            .unwrap()
            .flatten()
            .all(|entry| !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".opentopia-skill-")));
    }

    #[test]
    fn schema_requires_every_top_level_property() {
        let schema = skill_draft_json_schema();
        let properties = schema["properties"].as_object().unwrap();
        let required = schema["required"].as_array().unwrap();
        assert_eq!(properties.len(), required.len());
        for key in properties.keys() {
            assert!(required.iter().any(|value| value.as_str() == Some(key)));
        }
    }
}
