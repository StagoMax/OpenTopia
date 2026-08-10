use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasePromptModule {
    pub id: &'static str,
    pub content: &'static str,
}

/// The fixed core prompt is assembled strictly in this order.
///
/// Add, remove, or reorder modules here. Each module owns one policy concern so
/// its wording can be maintained without editing an unrelated prompt section.
pub const BASE_PROMPT_MODULES: &[BasePromptModule] = &[
    BasePromptModule {
        id: "identity_and_objective",
        content: include_str!("prompts/base/identity_and_objective.md"),
    },
    BasePromptModule {
        id: "instruction_hierarchy",
        content: include_str!("prompts/base/instruction_hierarchy.md"),
    },
    BasePromptModule {
        id: "request_interpretation",
        content: include_str!("prompts/base/request_interpretation.md"),
    },
    BasePromptModule {
        id: "workspace_discipline",
        content: include_str!("prompts/base/workspace_discipline.md"),
    },
    BasePromptModule {
        id: "codebase_exploration",
        content: include_str!("prompts/base/codebase_exploration.md"),
    },
    BasePromptModule {
        id: "git_safety",
        content: include_str!("prompts/base/git_safety.md"),
    },
    BasePromptModule {
        id: "skills",
        content: include_str!("prompts/base/skills.md"),
    },
    BasePromptModule {
        id: "tool_loop",
        content: include_str!("prompts/base/tool_loop.md"),
    },
    BasePromptModule {
        id: "validation",
        content: include_str!("prompts/base/validation.md"),
    },
    BasePromptModule {
        id: "communication",
        content: include_str!("prompts/base/communication.md"),
    },
    BasePromptModule {
        id: "completion",
        content: include_str!("prompts/base/completion.md"),
    },
];

static ASSEMBLED_BASE_PROMPT: OnceLock<String> = OnceLock::new();

pub fn base_agent_prompt() -> &'static str {
    ASSEMBLED_BASE_PROMPT.get_or_init(|| assemble_prompt_modules(BASE_PROMPT_MODULES))
}

pub fn base_prompt_module_ids() -> Vec<&'static str> {
    BASE_PROMPT_MODULES.iter().map(|module| module.id).collect()
}

fn assemble_prompt_modules(modules: &[BasePromptModule]) -> String {
    let mut prompt = modules
        .iter()
        .map(|module| module.content.trim())
        .collect::<Vec<_>>()
        .join("\n\n");
    prompt.push('\n');
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn base_prompt_manifest_has_stable_unique_order() {
        let ids = base_prompt_module_ids();
        assert_eq!(
            ids,
            [
                "identity_and_objective",
                "instruction_hierarchy",
                "request_interpretation",
                "workspace_discipline",
                "codebase_exploration",
                "git_safety",
                "skills",
                "tool_loop",
                "validation",
                "communication",
                "completion",
            ]
        );
        assert_eq!(ids.iter().copied().collect::<HashSet<_>>().len(), ids.len());
        assert!(BASE_PROMPT_MODULES
            .iter()
            .all(|module| !module.content.trim().is_empty()));
    }

    #[test]
    fn assembler_owns_module_separators_and_final_newline() {
        let modules = [
            BasePromptModule {
                id: "first",
                content: "\nfirst\n\n",
            },
            BasePromptModule {
                id: "second",
                content: " second ",
            },
        ];
        assert_eq!(assemble_prompt_modules(&modules), "first\n\nsecond\n");
    }
}
