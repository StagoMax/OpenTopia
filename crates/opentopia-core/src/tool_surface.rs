use crate::model::{CollaborationMode, ExperienceMode};
use crate::tools::ToolSource;

/// Product-level tool bundles. Availability is decided only from the selected
/// work/flow and collaboration modes; individual tools do not own ad-hoc gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolBundle {
    Common,
    Flow,
    Plan,
    Goal,
    External,
}

pub fn tool_bundle(name: &str, source: &ToolSource) -> ToolBundle {
    if !matches!(source, ToolSource::Core) {
        return ToolBundle::External;
    }
    if name.starts_with("flow_") {
        return ToolBundle::Flow;
    }
    match name {
        "request_user_input" => ToolBundle::Plan,
        "set_plan" | "update_plan" | "complete_task" => ToolBundle::Goal,
        _ => ToolBundle::Common,
    }
}

pub fn bundle_is_visible(
    bundle: ToolBundle,
    experience_mode: ExperienceMode,
    collaboration_mode: CollaborationMode,
) -> bool {
    match bundle {
        ToolBundle::Common | ToolBundle::External => true,
        ToolBundle::Flow => experience_mode == ExperienceMode::Flow,
        ToolBundle::Plan => collaboration_mode == CollaborationMode::Plan,
        ToolBundle::Goal => collaboration_mode == CollaborationMode::Goal,
    }
}

pub fn external_namespace(name: &str, source: &ToolSource) -> Option<(String, String)> {
    match source {
        ToolSource::Core => None,
        ToolSource::BundledPlugin { plugin_name } => Some((
            plugin_name.clone(),
            format!("Tools supplied by the {plugin_name} bundled plugin."),
        )),
        ToolSource::Mcp => {
            let namespace = name
                .split_once("__")
                .map(|(prefix, _)| prefix)
                .filter(|prefix| !prefix.is_empty())
                .unwrap_or("mcp")
                .to_string();
            Some((
                namespace.clone(),
                format!("Tools supplied by the {namespace} MCP server."),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_bundles_are_orthogonal() {
        assert!(bundle_is_visible(
            ToolBundle::Plan,
            ExperienceMode::Code,
            CollaborationMode::Plan
        ));
        assert!(!bundle_is_visible(
            ToolBundle::Goal,
            ExperienceMode::Code,
            CollaborationMode::Plan
        ));
        assert!(bundle_is_visible(
            ToolBundle::Flow,
            ExperienceMode::Flow,
            CollaborationMode::Default
        ));
        assert!(!bundle_is_visible(
            ToolBundle::Flow,
            ExperienceMode::Work,
            CollaborationMode::Default
        ));
    }
}
