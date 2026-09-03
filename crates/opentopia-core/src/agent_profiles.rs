use crate::sandbox::SandboxMode;
use crate::{ContributionKind, PluginDescriptor};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_contribution_id: Option<String>,
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
            source_plugin_id: None,
            source_contribution_id: None,
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
            denied_tools: vec!["apply_patch".to_string(), "create_skill".to_string()],
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

    pub fn load_with_plugin_profiles(
        workspace_root: &Path,
        plugins: &[PluginDescriptor],
        enabled_plugin_ids: &BTreeSet<String>,
    ) -> Self {
        let mut registry = Self::load(workspace_root);
        let mut contributions =
            BTreeMap::<String, Vec<(&PluginDescriptor, &crate::PluginContribution)>>::new();
        for plugin in plugins
            .iter()
            .filter(|plugin| enabled_plugin_ids.contains(&plugin.id))
        {
            for contribution in plugin
                .capability_manifest
                .contributions
                .iter()
                .filter(|contribution| contribution.kind == ContributionKind::AgentProfile)
            {
                contributions
                    .entry(contribution.local_id.clone())
                    .or_default()
                    .push((plugin, contribution));
            }
        }
        for (local_id, registrations) in contributions {
            if registrations.len() > 1 {
                registry.warnings.push(format!(
                    "ignored agent profile `{local_id}`: active plugins declare conflicting contributions"
                ));
                continue;
            }
            let (plugin, contribution) = registrations[0];
            let Some(reference) = contribution.declared_path_reference() else {
                registry.warnings.push(format!(
                    "ignored {}: agent profile contribution has no package-relative path",
                    contribution.id
                ));
                continue;
            };
            registry.load_plugin_profile(
                &plugin.id,
                &contribution.id,
                &contribution.local_id,
                &plugin.path,
                reference,
            );
        }
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

    fn load_plugin_profile(
        &mut self,
        plugin_id: &str,
        contribution_id: &str,
        local_id: &str,
        plugin_root: &Path,
        reference: &str,
    ) {
        let root = match plugin_root.canonicalize() {
            Ok(root) => root,
            Err(error) => {
                self.warnings.push(format!(
                    "ignored {contribution_id}: failed to resolve plugin root: {error}"
                ));
                return;
            }
        };
        let path = match root.join(reference.trim_start_matches("./")).canonicalize() {
            Ok(path) if path.starts_with(&root) && path.is_file() => path,
            Ok(_) => {
                self.warnings.push(format!(
                    "ignored {contribution_id}: profile path escapes the plugin package"
                ));
                return;
            }
            Err(error) => {
                self.warnings.push(format!(
                    "ignored {contribution_id}: failed to resolve profile: {error}"
                ));
                return;
            }
        };
        let source = match fs::read_to_string(&path) {
            Ok(source) if source.len() <= 256 * 1024 => source,
            Ok(_) => {
                self.warnings.push(format!(
                    "ignored {contribution_id}: profile exceeds 256 KiB"
                ));
                return;
            }
            Err(error) => {
                self.warnings.push(format!(
                    "ignored {contribution_id}: failed to read {}: {error}",
                    path.display()
                ));
                return;
            }
        };
        let parsed = match path.extension().and_then(|value| value.to_str()) {
            Some("json") => {
                serde_json::from_str::<AgentProfile>(&source).map_err(anyhow::Error::from)
            }
            _ => toml::from_str::<AgentProfile>(&source).map_err(anyhow::Error::from),
        };
        let mut profile = match parsed {
            Ok(profile) => profile,
            Err(error) => {
                self.warnings.push(format!(
                    "ignored {contribution_id}: invalid agent profile: {error}"
                ));
                return;
            }
        };
        if !is_valid_profile_name(&profile.name) || profile.name != local_id {
            self.warnings.push(format!(
                "ignored {contribution_id}: profile name must equal contribution id `{local_id}`"
            ));
            return;
        }
        if self.profiles.contains_key(&profile.name) {
            self.warnings.push(format!(
                "ignored {contribution_id}: agent profile `{}` is already registered",
                profile.name
            ));
            return;
        }
        profile.source_plugin_id = Some(plugin_id.to_string());
        profile.source_contribution_id = Some(contribution_id.to_string());
        self.profiles.insert(profile.name.clone(), profile);
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

    #[test]
    fn plugin_profiles_are_package_scoped_and_cannot_override_builtins() {
        let root =
            std::env::temp_dir().join(format!("opentopia-plugin-profile-{}", Uuid::new_v4()));
        let plugin_root = root.join("plugin");
        fs::create_dir_all(plugin_root.join("agents")).unwrap();
        fs::write(
            plugin_root.join("agents/reviewer.toml"),
            r#"
name = "reviewer"
description = "Plugin reviewer"
developer_instructions = "Review the selected domain."
denied_tools = ["computer"]
"#,
        )
        .unwrap();
        fs::write(
            plugin_root.join("agents/worker.toml"),
            r#"
name = "worker"
description = "Override"
developer_instructions = "Override built-in."
"#,
        )
        .unwrap();

        let mut registry = AgentProfileRegistry::default();
        registry.load_plugin_profile(
            "plugin",
            "plugin/reviewer",
            "reviewer",
            &plugin_root,
            "agents/reviewer.toml",
        );
        registry.load_plugin_profile(
            "plugin",
            "plugin/worker",
            "worker",
            &plugin_root,
            "agents/worker.toml",
        );
        let reviewer = registry.get("reviewer").unwrap();
        assert_eq!(reviewer.source_plugin_id.as_deref(), Some("plugin"));
        assert_eq!(
            registry.get("worker").unwrap().description,
            "Implementation-focused agent for a concrete, bounded work item."
        );
        assert!(registry
            .warnings()
            .iter()
            .any(|warning| warning.contains("already registered")));
        fs::remove_dir_all(root).unwrap();
    }
}
