use crate::sandbox::SandboxMode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentProfile {
    pub name: String,
    pub description: String,
    pub developer_instructions: String,
    #[serde(default)]
    pub nickname_candidates: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_reasoning_effort: Option<String>,
    #[serde(default)]
    pub sandbox_mode: Option<SandboxMode>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub denied_tools: Vec<String>,
}

impl AgentProfile {
    fn default_profile() -> Self {
        Self {
            name: "default".to_string(),
            description: "General-purpose agent that inherits the parent configuration.".to_string(),
            developer_instructions: "Own the delegated task, use judgment about tools and validation, and return a concise evidence-backed result to the requesting agent.".to_string(),
            nickname_candidates: Vec::new(),
            model: None,
            model_reasoning_effort: None,
            sandbox_mode: None,
            allowed_tools: None,
            denied_tools: Vec::new(),
        }
    }

    fn worker_profile() -> Self {
        Self {
            name: "worker".to_string(),
            description: "Implementation-focused agent for a concrete, bounded work item.".to_string(),
            developer_instructions: "Work only on the assigned implementation scope. Inspect before editing, preserve unrelated changes, verify proportionally to risk, and report exact files and checks.".to_string(),
            ..Self::default_profile()
        }
    }

    fn explorer_profile() -> Self {
        Self {
            name: "explorer".to_string(),
            description: "Read-only agent for codebase exploration and evidence gathering.".to_string(),
            developer_instructions: "Explore and analyze without modifying files or external state. Return concrete evidence with paths, symbols, and unresolved uncertainty.".to_string(),
            sandbox_mode: Some(SandboxMode::ReadOnly),
            denied_tools: vec![
                "write_file".to_string(),
                "apply_patch".to_string(),
                "spreadsheet".to_string(),
            ],
            ..Self::default_profile()
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentProfileRegistry {
    profiles: BTreeMap<String, AgentProfile>,
    warnings: Vec<String>,
}

impl Default for AgentProfileRegistry {
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        for profile in [
            AgentProfile::default_profile(),
            AgentProfile::worker_profile(),
            AgentProfile::explorer_profile(),
        ] {
            profiles.insert(profile.name.clone(), profile);
        }
        Self {
            profiles,
            warnings: Vec::new(),
        }
    }
}

impl AgentProfileRegistry {
    pub fn load(workspace_root: &Path) -> Self {
        let mut registry = Self::default();
        if let Some(codex_home) = codex_home() {
            registry.load_directory(&codex_home.join("agents"));
        }
        registry.load_directory(&workspace_root.join(".codex").join("agents"));
        registry
    }

    pub fn get(&self, name: &str) -> Option<&AgentProfile> {
        self.profiles.get(name.trim())
    }

    pub fn list(&self) -> Vec<AgentProfile> {
        self.profiles.values().cloned().collect()
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn load_directory(&mut self, directory: &Path) {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                self.warnings
                    .push(format!("failed to read {}: {error}", directory.display()));
                return;
            }
        };
        let mut paths = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("toml"))
            .collect::<Vec<_>>();
        paths.sort();
        for path in paths {
            match fs::read_to_string(&path)
                .map_err(anyhow::Error::from)
                .and_then(|source| toml::from_str::<AgentProfile>(&source).map_err(Into::into))
            {
                Ok(profile) if is_valid_profile_name(&profile.name) => {
                    self.profiles.insert(profile.name.clone(), profile);
                }
                Ok(profile) => self.warnings.push(format!(
                    "ignored {}: invalid agent profile name `{}`",
                    path.display(),
                    profile.name
                )),
                Err(error) => self
                    .warnings
                    .push(format!("ignored {}: {error}", path.display())),
            }
        }
    }
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from)
                .map(|home| home.join(".codex"))
        })
}

fn is_valid_profile_name(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn project_profiles_override_builtins() {
        let root = std::env::temp_dir().join(format!("opentopia-agent-profile-{}", Uuid::new_v4()));
        let directory = root.join(".codex").join("agents");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("worker.toml"),
            r#"
name = "worker"
description = "Project worker"
developer_instructions = "Use project conventions."
model_reasoning_effort = "high"
"#,
        )
        .unwrap();

        let registry = AgentProfileRegistry::load(&root);
        let profile = registry.get("worker").unwrap();
        assert_eq!(profile.description, "Project worker");
        assert_eq!(profile.model_reasoning_effort.as_deref(), Some("high"));
        fs::remove_dir_all(root).unwrap();
    }
}
