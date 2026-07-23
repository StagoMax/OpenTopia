use crate::plugins::{discover_plugins, PluginScope};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use thiserror::Error;

const MAX_SKILLS_PER_TURN: usize = 5;
const MAX_SKILL_BYTES: usize = 64 * 1024;
const MAX_TOTAL_SKILL_BYTES: usize = 128 * 1024;
const MAX_SKILL_SOURCE_BYTES: u64 = 1024 * 1024;
const MAX_SKILL_DISCOVERY_BYTES: usize = 64 * 1024;
const MAX_DISCOVERY_DEPTH: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub scope: SkillScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillScope {
    Workspace,
    User,
}

#[derive(Debug, Clone)]
pub struct LoadedSkill {
    pub descriptor: SkillDescriptor,
    pub instructions: String,
    pub truncated: bool,
}

impl LoadedSkill {
    pub fn render_for_model(&self) -> String {
        format!(
            "<skill>\nName: {}\nDescription: {}\nPath: {}\nTruncated: {}\n\n{}\n</skill>",
            self.descriptor.name,
            self.descriptor.description,
            self.descriptor.path.display(),
            self.truncated,
            self.instructions
        )
    }
}

#[derive(Debug, Error)]
pub enum SkillError {
    #[error("too many skills selected: {actual}; maximum is {maximum}")]
    TooMany { actual: usize, maximum: usize },
    #[error("unknown or unavailable skill: {0}")]
    Unknown(String),
    #[error("skill cannot be read: {0}")]
    Read(String),
    #[error("skill file is too large: {path}; size is {actual} bytes, maximum is {maximum} bytes")]
    TooLarge {
        path: String,
        actual: u64,
        maximum: u64,
    },
    #[error("skill metadata is invalid: {0}")]
    InvalidMetadata(String),
}

#[derive(Debug, Default, Deserialize)]
struct SkillFrontMatter {
    name: Option<String>,
    description: Option<String>,
}

pub fn discover_skills(workspace_root: Option<&Path>) -> Vec<SkillDescriptor> {
    let mut roots = Vec::new();
    if let Some(workspace_root) = workspace_root {
        roots.push((workspace_root.join(".agents/skills"), SkillScope::Workspace));
        roots.push((workspace_root.join(".codex/skills"), SkillScope::Workspace));
    }
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        roots.push((PathBuf::from(codex_home).join("skills"), SkillScope::User));
    } else if let Some(home) = home_dir() {
        roots.push((home.join(".codex/skills"), SkillScope::User));
    }

    let mut descriptors = Vec::new();
    let mut seen = HashSet::new();
    for (root, scope) in roots {
        collect_skill_files(
            &root,
            &root,
            scope,
            None,
            None,
            0,
            &mut seen,
            &mut descriptors,
        );
    }
    for plugin in discover_plugins(workspace_root) {
        let Some(skill_root) = plugin.skill_root.as_deref() else {
            continue;
        };
        let scope = match plugin.scope {
            PluginScope::Workspace => SkillScope::Workspace,
            PluginScope::User | PluginScope::Codex => SkillScope::User,
        };
        collect_skill_files(
            skill_root,
            skill_root,
            scope,
            Some(&plugin.id),
            Some(&plugin.name),
            0,
            &mut seen,
            &mut descriptors,
        );
    }
    descriptors.sort_by(|left, right| {
        scope_rank(left.scope)
            .cmp(&scope_rank(right.scope))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.id.cmp(&right.id))
    });
    descriptors
}

pub fn load_selected_skills(
    workspace_root: Option<&Path>,
    ids: &[String],
) -> Result<Vec<LoadedSkill>, SkillError> {
    if ids.len() > MAX_SKILLS_PER_TURN {
        return Err(SkillError::TooMany {
            actual: ids.len(),
            maximum: MAX_SKILLS_PER_TURN,
        });
    }
    let catalog = discover_skills(workspace_root)
        .into_iter()
        .map(|skill| (skill.id.clone(), skill))
        .collect::<HashMap<_, _>>();
    let mut loaded = Vec::new();
    let mut seen = HashSet::new();
    let mut remaining = MAX_TOTAL_SKILL_BYTES;
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let descriptor = catalog
            .get(id)
            .cloned()
            .ok_or_else(|| SkillError::Unknown(id.clone()))?;
        let limit = MAX_SKILL_BYTES.min(remaining);
        let (bytes, truncated) = read_skill_file(&descriptor.path, limit)?;
        let instructions = String::from_utf8_lossy(&bytes).into_owned();
        remaining = remaining.saturating_sub(bytes.len());
        loaded.push(LoadedSkill {
            descriptor,
            instructions,
            truncated,
        });
    }
    Ok(loaded)
}

fn collect_skill_files(
    root: &Path,
    directory: &Path,
    scope: SkillScope,
    plugin_id: Option<&str>,
    plugin_name: Option<&str>,
    depth: usize,
    seen: &mut HashSet<String>,
    output: &mut Vec<SkillDescriptor>,
) {
    if depth > MAX_DISCOVERY_DEPTH || !directory.is_dir() {
        return;
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_skill_files(
                root,
                &path,
                scope,
                plugin_id,
                plugin_name,
                depth + 1,
                seen,
                output,
            );
            continue;
        }
        if !file_type.is_file() || !entry.file_name().eq_ignore_ascii_case("SKILL.md") {
            continue;
        }
        let canonical = match path.canonicalize() {
            Ok(path) => path,
            Err(_) => continue,
        };
        let canonical_root = match root.canonicalize() {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !canonical.starts_with(canonical_root) {
            continue;
        }
        let id = skill_id(scope, &canonical);
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Ok(descriptor) = descriptor_from_file(
            id,
            canonical,
            scope,
            plugin_id.map(ToOwned::to_owned),
            plugin_name,
        ) {
            output.push(descriptor);
        }
    }
}

fn descriptor_from_file(
    id: String,
    path: PathBuf,
    scope: SkillScope,
    plugin_id: Option<String>,
    plugin_name: Option<&str>,
) -> Result<SkillDescriptor, SkillError> {
    let (bytes, _) = read_skill_file(&path, MAX_SKILL_DISCOVERY_BYTES)?;
    let text = String::from_utf8_lossy(&bytes);
    let metadata = parse_front_matter(&text, &path)?;
    let fallback_name = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("Skill")
        .to_string();
    let mut name = metadata
        .name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(fallback_name);
    if let Some(plugin_name) = plugin_name {
        name = format!("{plugin_name}:{name}");
    }
    Ok(SkillDescriptor {
        id,
        name,
        description: metadata.description.unwrap_or_default(),
        path,
        scope,
        plugin_id,
    })
}

pub(crate) fn descriptor_for_skill_file(
    path: PathBuf,
    scope: SkillScope,
) -> Result<SkillDescriptor, SkillError> {
    let canonical = path
        .canonicalize()
        .map_err(|_| SkillError::Read(path.display().to_string()))?;
    descriptor_from_file(skill_id(scope, &canonical), canonical, scope, None, None)
}

fn read_skill_file(path: &Path, read_limit: usize) -> Result<(Vec<u8>, bool), SkillError> {
    let path_display = path.display().to_string();
    let metadata = fs::metadata(path).map_err(|_| SkillError::Read(path_display.clone()))?;
    if !metadata.is_file() {
        return Err(SkillError::Read(path_display));
    }
    if metadata.len() > MAX_SKILL_SOURCE_BYTES {
        return Err(SkillError::TooLarge {
            path: path_display,
            actual: metadata.len(),
            maximum: MAX_SKILL_SOURCE_BYTES,
        });
    }

    let file = File::open(path).map_err(|_| SkillError::Read(path.display().to_string()))?;
    let sentinel_limit = read_limit.saturating_add(1);
    let mut reader = BufReader::new(file).take(sentinel_limit as u64);
    let mut bytes = Vec::with_capacity(sentinel_limit);
    reader
        .read_to_end(&mut bytes)
        .map_err(|_| SkillError::Read(path.display().to_string()))?;
    let truncated = bytes.len() > read_limit || metadata.len() > bytes.len() as u64;
    bytes.truncate(read_limit);
    Ok((bytes, truncated))
}

fn parse_front_matter(text: &str, path: &Path) -> Result<SkillFrontMatter, SkillError> {
    let Some(rest) = text.strip_prefix("---") else {
        return Ok(SkillFrontMatter::default());
    };
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'));
    let Some(rest) = rest else {
        return Ok(SkillFrontMatter::default());
    };
    let end = rest
        .find("\n---")
        .ok_or_else(|| SkillError::InvalidMetadata(path.display().to_string()))?;
    serde_yaml::from_str(&rest[..end])
        .map_err(|_| SkillError::InvalidMetadata(path.display().to_string()))
}

fn skill_id(scope: SkillScope, path: &Path) -> String {
    format!(
        "{}:{}",
        match scope {
            SkillScope::Workspace => "workspace",
            SkillScope::User => "user",
        },
        path.to_string_lossy().replace('\\', "/")
    )
}

fn scope_rank(scope: SkillScope) -> u8 {
    match scope {
        SkillScope::Workspace => 0,
        SkillScope::User => 1,
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use uuid::Uuid;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("opentopia-skills-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn skill(&self, name: &str, content: &str) -> PathBuf {
            let directory = self.0.join(".agents/skills").join(name);
            fs::create_dir_all(&directory).unwrap();
            let path = directory.join("SKILL.md");
            let mut file = fs::File::create(&path).unwrap();
            file.write_all(content.as_bytes()).unwrap();
            path
        }

        fn oversized_skill(&self, name: &str) -> PathBuf {
            let path = self.skill(name, "---\nname: Oversized\n---\n");
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(MAX_SKILL_SOURCE_BYTES + 1)
                .unwrap();
            path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovers_workspace_skill_and_parses_yaml_metadata() {
        let dir = TestDir::new();
        dir.skill(
            "review",
            "---\nname: Review\ndescription: Review code safely\n---\n\nDo the review.\n",
        );
        let skills = discover_skills(Some(&dir.0));
        let skill = skills.iter().find(|skill| skill.name == "Review").unwrap();
        assert_eq!(skill.scope, SkillScope::Workspace);
        assert_eq!(skill.description, "Review code safely");
    }

    #[test]
    fn loads_only_discovered_ids_and_deduplicates_selection() {
        let dir = TestDir::new();
        dir.skill("review", "---\nname: Review\n---\nInstructions\n");
        let descriptor = discover_skills(Some(&dir.0))
            .into_iter()
            .find(|skill| skill.name == "Review")
            .unwrap();
        let loaded =
            load_selected_skills(Some(&dir.0), &[descriptor.id.clone(), descriptor.id]).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].instructions.contains("Instructions"));
        assert!(matches!(
            load_selected_skills(Some(&dir.0), &["workspace:missing".to_string()]),
            Err(SkillError::Unknown(_))
        ));
    }

    #[test]
    fn discovers_plugin_skills_with_a_namespaced_name() {
        let dir = TestDir::new();
        let plugin_root = dir.0.join(".opentopia/plugins/review-kit");
        fs::create_dir_all(plugin_root.join(".codex-plugin")).unwrap();
        fs::create_dir_all(plugin_root.join("skills/review")).unwrap();
        fs::write(
            plugin_root.join(".codex-plugin/plugin.json"),
            r#"{"name":"review-kit","skills":"./skills"}"#,
        )
        .unwrap();
        fs::write(
            plugin_root.join("skills/review/SKILL.md"),
            "---\nname: Review\ndescription: Review changes\n---\nInstructions\n",
        )
        .unwrap();

        let skills = discover_skills(Some(&dir.0));
        let skill = skills
            .iter()
            .find(|skill| skill.name == "review-kit:Review")
            .expect("plugin Skill should be discovered");
        assert!(skill
            .plugin_id
            .as_deref()
            .is_some_and(|id| id.starts_with("workspace:")));
        assert_eq!(skill.scope, SkillScope::Workspace);
    }

    #[test]
    fn rejects_excessive_skill_selection() {
        let ids = (0..=MAX_SKILLS_PER_TURN)
            .map(|index| index.to_string())
            .collect::<Vec<_>>();
        assert!(matches!(
            load_selected_skills(None, &ids),
            Err(SkillError::TooMany { .. })
        ));
    }

    #[test]
    fn skips_skill_files_larger_than_the_source_limit() {
        let dir = TestDir::new();
        let path = dir.oversized_skill("oversized");

        assert!(discover_skills(Some(&dir.0))
            .iter()
            .all(|skill| skill.path != path));
        assert!(matches!(
            descriptor_from_file(
                "workspace:oversized".to_string(),
                path,
                SkillScope::Workspace,
                None,
                None,
            ),
            Err(SkillError::TooLarge {
                actual,
                maximum: MAX_SKILL_SOURCE_BYTES,
                ..
            }) if actual == MAX_SKILL_SOURCE_BYTES + 1
        ));
    }

    #[test]
    fn loads_at_most_the_per_skill_limit_from_an_allowed_large_file() {
        let dir = TestDir::new();
        let content = format!(
            "---\nname: Large\ndescription: Bounded read\n---\n{}",
            "x".repeat(MAX_SKILL_BYTES + 1024)
        );
        dir.skill("large", &content);
        let descriptor = discover_skills(Some(&dir.0))
            .into_iter()
            .find(|skill| skill.name == "Large")
            .unwrap();

        let loaded = load_selected_skills(Some(&dir.0), &[descriptor.id]).unwrap();

        assert_eq!(loaded[0].instructions.len(), MAX_SKILL_BYTES);
        assert!(loaded[0].truncated);
    }
}
