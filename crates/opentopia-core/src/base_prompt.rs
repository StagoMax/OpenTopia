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
        id: "personality",
        content: include_str!("prompts/base/personality.md"),
    },
    BasePromptModule {
        id: "writing_style",
        content: include_str!("prompts/base/writing_style.md"),
    },
    BasePromptModule {
        id: "technical_communication",
        content: include_str!("prompts/base/technical_communication.md"),
    },
    BasePromptModule {
        id: "working_with_user",
        content: include_str!("prompts/base/working_with_user.md"),
    },
    BasePromptModule {
        id: "intermediate_commentary",
        content: include_str!("prompts/base/intermediate_commentary.md"),
    },
    BasePromptModule {
        id: "final_answer",
        content: include_str!("prompts/base/final_answer.md"),
    },
    BasePromptModule {
        id: "formatting_and_visualizations",
        content: include_str!("prompts/base/formatting_and_visualizations.md"),
    },
    BasePromptModule {
        id: "working_rules",
        content: include_str!("prompts/base/working_rules.md"),
    },
    BasePromptModule {
        id: "codebase_exploration",
        content: include_str!("prompts/base/codebase_exploration.md"),
    },
    BasePromptModule {
        id: "file_editing_constraints",
        content: include_str!("prompts/base/file_editing_constraints.md"),
    },
    BasePromptModule {
        id: "autonomy_and_persistence",
        content: include_str!("prompts/base/autonomy_and_persistence.md"),
    },
    BasePromptModule {
        id: "destructive_actions",
        content: include_str!("prompts/base/destructive_actions.md"),
    },
    BasePromptModule {
        id: "skills",
        content: include_str!("prompts/base/skills.md"),
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
                "personality",
                "writing_style",
                "technical_communication",
                "working_with_user",
                "intermediate_commentary",
                "final_answer",
                "formatting_and_visualizations",
                "working_rules",
                "codebase_exploration",
                "file_editing_constraints",
                "autonomy_and_persistence",
                "destructive_actions",
                "skills",
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
