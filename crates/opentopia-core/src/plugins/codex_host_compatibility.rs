#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CodexHostSkillRestriction {
    pub(super) replacement: Option<&'static str>,
}

/// Some first-party Codex packages intentionally ship instructions and assets
/// whose executable surface is injected by the Codex Desktop host. Projecting
/// those Skills in OpenTopia advertises tools and runtimes that do not exist.
///
/// `openai-primary-runtime` is a machine-readable marketplace/runtime boundary,
/// so future packages from that runtime are rejected without adding task- or
/// file-specific rules. Known native equivalents are named only to make the
/// diagnostics actionable; the package remains discoverable without its Skill
/// contribution.
pub(super) fn codex_host_skill_restriction(plugin_id: &str) -> Option<CodexHostSkillRestriction> {
    let (plugin_name, marketplace_name) = plugin_id.rsplit_once('@')?;
    if marketplace_name == "openai-primary-runtime" {
        let replacement = match plugin_name {
            "documents" => Some("Documents"),
            "pdf" => Some("PDF"),
            "spreadsheets" => Some("Spreadsheet"),
            _ => None,
        };
        return Some(CodexHostSkillRestriction { replacement });
    }

    let replacement = match plugin_id {
        "browser@openai-bundled" | "chrome@openai-bundled" => "Browser Automation",
        "computer-use@openai-bundled" => "Computer Use",
        _ => return None,
    };
    Some(CodexHostSkillRestriction {
        replacement: Some(replacement),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_runtime_packages_are_host_restricted_as_a_family() {
        let packages = [
            ("documents", Some("Documents")),
            ("pdf", Some("PDF")),
            ("presentations", None),
            ("spreadsheets", Some("Spreadsheet")),
            ("template-creator", None),
        ];

        for (plugin_name, expected_replacement) in packages {
            assert_eq!(
                codex_host_skill_restriction(&format!("{plugin_name}@openai-primary-runtime")),
                Some(CodexHostSkillRestriction {
                    replacement: expected_replacement,
                })
            );
        }
    }

    #[test]
    fn portable_codex_marketplaces_are_not_restricted() {
        assert_eq!(codex_host_skill_restriction("portable@community"), None);
        assert_eq!(
            codex_host_skill_restriction("github@openai-curated-remote"),
            None
        );
    }
}
