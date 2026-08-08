use serde::{Deserialize, Serialize};

/// Coarse capabilities inferred from a shell command. This is advisory
/// metadata for scheduling, journaling, and policy checks; it is not an
/// authorization grant and must never widen the active sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellCapability {
    Observation,
    ReadFiles,
    WorkspaceWrite,
    DeleteFiles,
    GitRead,
    GitMutation,
    Network,
    StartProcess,
    BackgroundProcess,
    DynamicExecution,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellAnalysisConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellCommandAnalysis {
    pub capabilities: Vec<ShellCapability>,
    pub destructive: bool,
    pub dynamic: bool,
    pub concrete_targets: Vec<String>,
    pub targets_concrete: bool,
    pub confidence: ShellAnalysisConfidence,
    pub has_pipeline: bool,
    pub has_redirection: bool,
    pub has_background_operator: bool,
    pub destructive_targets_concrete: bool,
    pub has_destructive_target: bool,
    strictly_read_only: bool,
}

impl ShellCommandAnalysis {
    /// True only for a deliberately small allowlist of observation commands.
    /// Pipelines, redirects, backgrounding, dynamic expansion, and unknown
    /// commands all make the answer false.
    pub fn is_strictly_read_only(&self) -> bool {
        self.strictly_read_only
    }

    /// A destructive command with a variable, wildcard, substitution, or no
    /// identifiable target cannot be reviewed as a concrete action. Callers
    /// may return it to the agent for concretization before approval.
    pub fn is_unreviewable_destructive_action(&self) -> bool {
        self.destructive && (!self.destructive_targets_concrete || !self.has_destructive_target)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Word {
    value: String,
    dynamic: bool,
    wildcard: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Sequence,
    Pipeline,
    Redirection,
    Background,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Lexeme {
    Word(Word),
    Operator(Operator),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandFamily {
    Ripgrep,
    SelectString,
    GetContent,
    Git,
    Delete,
    SetContent,
    OutFile,
    Network,
    DynamicRuntime,
    PackageManager,
    Cargo,
    Format,
}

const COMMAND_FAMILIES: &[(&str, CommandFamily)] = &[
    ("rg", CommandFamily::Ripgrep),
    ("ripgrep", CommandFamily::Ripgrep),
    ("select-string", CommandFamily::SelectString),
    ("get-content", CommandFamily::GetContent),
    ("git", CommandFamily::Git),
    ("remove-item", CommandFamily::Delete),
    ("rm", CommandFamily::Delete),
    ("del", CommandFamily::Delete),
    ("erase", CommandFamily::Delete),
    ("rmdir", CommandFamily::Delete),
    ("rd", CommandFamily::Delete),
    ("set-content", CommandFamily::SetContent),
    ("out-file", CommandFamily::OutFile),
    ("curl", CommandFamily::Network),
    ("invoke-webrequest", CommandFamily::Network),
    ("invoke-restmethod", CommandFamily::Network),
    ("wget", CommandFamily::Network),
    ("python", CommandFamily::DynamicRuntime),
    ("python3", CommandFamily::DynamicRuntime),
    ("py", CommandFamily::DynamicRuntime),
    ("node", CommandFamily::DynamicRuntime),
    ("pwsh", CommandFamily::DynamicRuntime),
    ("powershell", CommandFamily::DynamicRuntime),
    ("powershell_ise", CommandFamily::DynamicRuntime),
    ("pnpm", CommandFamily::PackageManager),
    ("npm", CommandFamily::PackageManager),
    ("npx", CommandFamily::PackageManager),
    ("pnpx", CommandFamily::PackageManager),
    ("cargo", CommandFamily::Cargo),
    ("format", CommandFamily::Format),
];

const GIT_READ_SUBCOMMANDS: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "rev-parse",
    "branch",
    "worktree",
    "blame",
    "ls-files",
    "ls-tree",
    "cat-file",
    "describe",
    "remote",
    "tag",
];

const GIT_MUTATION_SUBCOMMANDS: &[&str] = &[
    "add",
    "commit",
    "checkout",
    "switch",
    "restore",
    "merge",
    "rebase",
    "cherry-pick",
    "revert",
    "stash",
    "apply",
    "am",
    "mv",
    "rm",
    "init",
    "clone",
    "fetch",
    "pull",
    "push",
];

const PACKAGE_WRITE_SUBCOMMANDS: &[&str] = &[
    "add",
    "install",
    "i",
    "remove",
    "rm",
    "uninstall",
    "update",
    "upgrade",
    "publish",
    "link",
    "unlink",
];

const CARGO_WRITE_SUBCOMMANDS: &[&str] = &[
    "add",
    "remove",
    "install",
    "uninstall",
    "update",
    "publish",
    "login",
    "logout",
];

pub fn analyze_shell_command(command: &str) -> ShellCommandAnalysis {
    let lexemes = lex(command);
    let mut segments = Vec::<Vec<Word>>::new();
    let mut current = Vec::new();
    let mut has_pipeline = false;
    let mut has_redirection = false;
    let mut has_background_operator = false;

    for lexeme in lexemes {
        match lexeme {
            Lexeme::Word(word) => current.push(word),
            Lexeme::Operator(operator) => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
                match operator {
                    Operator::Pipeline => has_pipeline = true,
                    Operator::Redirection => has_redirection = true,
                    Operator::Background => has_background_operator = true,
                    Operator::Sequence => {}
                }
            }
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }

    let mut aggregate = Aggregate::default();
    for segment in &segments {
        classify_segment(segment, &mut aggregate);
    }

    if has_redirection {
        aggregate.capabilities.push(ShellCapability::WorkspaceWrite);
        aggregate.all_read_only = false;
    }
    if has_background_operator {
        aggregate
            .capabilities
            .push(ShellCapability::BackgroundProcess);
        aggregate.all_read_only = false;
    }
    if segments.is_empty() {
        aggregate.capabilities.push(ShellCapability::Unknown);
        aggregate.saw_unknown = true;
        aggregate.all_read_only = false;
    }

    aggregate.capabilities.sort_unstable();
    aggregate.capabilities.dedup();
    aggregate.concrete_targets.sort();
    aggregate.concrete_targets.dedup();

    let confidence = if aggregate.saw_unknown {
        ShellAnalysisConfidence::Low
    } else if aggregate.dynamic || !aggregate.targets_concrete {
        ShellAnalysisConfidence::Medium
    } else {
        ShellAnalysisConfidence::High
    };
    let strictly_read_only = aggregate.all_read_only
        && !aggregate.dynamic
        && !has_pipeline
        && !has_redirection
        && !has_background_operator
        && !aggregate.saw_unknown
        && !aggregate.destructive;

    ShellCommandAnalysis {
        capabilities: aggregate.capabilities,
        destructive: aggregate.destructive,
        dynamic: aggregate.dynamic,
        concrete_targets: aggregate.concrete_targets,
        targets_concrete: aggregate.targets_concrete,
        confidence,
        has_pipeline,
        has_redirection,
        has_background_operator,
        destructive_targets_concrete: aggregate.destructive_targets_concrete,
        has_destructive_target: aggregate.saw_destructive_target,
        strictly_read_only,
    }
}

#[derive(Default)]
struct Aggregate {
    capabilities: Vec<ShellCapability>,
    destructive: bool,
    dynamic: bool,
    concrete_targets: Vec<String>,
    targets_concrete: bool,
    saw_unknown: bool,
    all_read_only: bool,
    destructive_targets_concrete: bool,
    saw_destructive_target: bool,
}

impl Aggregate {
    fn init_if_needed(&mut self) {
        if self.capabilities.is_empty() {
            self.targets_concrete = true;
            self.all_read_only = true;
            self.destructive_targets_concrete = true;
        }
    }

    fn add_targets<'a>(&mut self, words: impl IntoIterator<Item = &'a Word>) {
        let mut saw_target = false;
        for word in words {
            saw_target = true;
            if word.dynamic
                || word.wildcard
                || word.value.trim().is_empty()
                || looks_like_dynamic_target(&word.value)
            {
                self.targets_concrete = false;
            } else {
                self.concrete_targets.push(word.value.clone());
            }
        }
        if !saw_target {
            self.targets_concrete = false;
        }
    }

    fn add_destructive_targets<'a>(&mut self, words: impl IntoIterator<Item = &'a Word>) {
        let words = words.into_iter().collect::<Vec<_>>();
        self.saw_destructive_target |= !words.is_empty();
        if words.iter().any(|word| {
            word.dynamic
                || word.wildcard
                || word.value.trim().is_empty()
                || looks_like_dynamic_target(&word.value)
        }) {
            self.destructive_targets_concrete = false;
        }
        self.add_targets(words);
    }
}

fn looks_like_dynamic_target(value: &str) -> bool {
    let value = value.trim_start();
    value.starts_with('(')
        || value.starts_with("@(")
        || value.starts_with("${")
        || value.starts_with("$(")
        || value.starts_with('{')
}

fn classify_segment(words: &[Word], aggregate: &mut Aggregate) {
    aggregate.init_if_needed();
    let Some(command_word) = words.first() else {
        return;
    };
    if command_word.dynamic {
        mark_unknown_dynamic(aggregate);
        return;
    }

    let command_name = normalize_command_name(&command_word.value);
    if command_name.ends_with(".ps1")
        || command_name.ends_with(".sh")
        || command_name.ends_with(".bat")
        || command_name.ends_with(".cmd")
    {
        aggregate.capabilities.extend([
            ShellCapability::StartProcess,
            ShellCapability::DynamicExecution,
        ]);
        aggregate.dynamic = true;
        aggregate.all_read_only = false;
        aggregate.add_targets(words.get(0..1).unwrap_or_default());
        return;
    }

    let Some(family) = COMMAND_FAMILIES
        .iter()
        .find_map(|(name, family)| (*name == command_name).then_some(*family))
    else {
        mark_unknown(aggregate);
        return;
    };

    match family {
        CommandFamily::Ripgrep => classify_ripgrep(words, aggregate),
        CommandFamily::SelectString | CommandFamily::GetContent => {
            classify_read_command(words, aggregate)
        }
        CommandFamily::Git => classify_git(words, aggregate),
        CommandFamily::Delete => classify_delete(words, &command_name, aggregate),
        CommandFamily::SetContent | CommandFamily::OutFile => {
            aggregate.capabilities.push(ShellCapability::WorkspaceWrite);
            aggregate.all_read_only = false;
            aggregate.add_targets(extract_powershell_paths(
                words,
                &["-path", "-literalpath", "-filepath"],
            ));
        }
        CommandFamily::Network => classify_network(words, aggregate),
        CommandFamily::DynamicRuntime => classify_dynamic_runtime(words, aggregate),
        CommandFamily::PackageManager => classify_package_manager(words, aggregate),
        CommandFamily::Cargo => classify_cargo(words, aggregate),
        CommandFamily::Format => {
            aggregate.capabilities.push(ShellCapability::DeleteFiles);
            aggregate.destructive = true;
            aggregate.all_read_only = false;
            aggregate.add_destructive_targets(positional_arguments(words, 1));
        }
    }
}

fn classify_ripgrep(words: &[Word], aggregate: &mut Aggregate) {
    aggregate
        .capabilities
        .extend([ShellCapability::Observation, ShellCapability::ReadFiles]);
    if words.iter().any(|word| {
        matches!(
            word.value.to_ascii_lowercase().as_str(),
            "--pre" | "--hostname-bin"
        ) || word.value.to_ascii_lowercase().starts_with("--pre=")
            || word
                .value
                .to_ascii_lowercase()
                .starts_with("--hostname-bin=")
    }) {
        aggregate
            .capabilities
            .push(ShellCapability::DynamicExecution);
        aggregate.dynamic = true;
        aggregate.all_read_only = false;
    }
    aggregate.dynamic |= words.iter().any(|word| word.dynamic);
    if aggregate.dynamic {
        aggregate.all_read_only = false;
    }
    // Distinguishing every ripgrep option value from every path would require
    // mirroring its full CLI parser. Keep a truthful coarse scope instead;
    // this metadata is never used to widen filesystem authorization.
    aggregate
        .concrete_targets
        .push("workspace:command-scope".to_string());
}

fn classify_read_command(words: &[Word], aggregate: &mut Aggregate) {
    aggregate
        .capabilities
        .extend([ShellCapability::Observation, ShellCapability::ReadFiles]);
    aggregate.dynamic |= words.iter().any(|word| word.dynamic);
    if aggregate.dynamic {
        aggregate.all_read_only = false;
    }
    let paths = extract_powershell_paths(words, &["-path", "-literalpath"]);
    if !paths.is_empty() {
        aggregate.add_targets(paths);
    }
}

fn classify_git(words: &[Word], aggregate: &mut Aggregate) {
    let Some((subcommand_index, subcommand)) = git_subcommand(words) else {
        mark_unknown(aggregate);
        return;
    };
    if let Some(workdir) = git_workdir(words) {
        aggregate.add_targets(std::iter::once(workdir));
    } else {
        aggregate
            .concrete_targets
            .push("repository:current-workdir".to_string());
    }
    if GIT_READ_SUBCOMMANDS.contains(&subcommand.as_str()) {
        let tail = &words[subcommand_index + 1..];
        let writes_output = tail.iter().any(|word| {
            let lower = word.value.to_ascii_lowercase();
            lower == "--output" || lower.starts_with("--output=")
        });
        // These subcommands also have mutating forms. Be deliberately
        // conservative unless their listing form is explicit.
        let ambiguous_mutator = match subcommand.as_str() {
            "branch" => {
                let mutating_flag = tail.iter().any(|word| {
                    matches!(
                        word.value.as_str(),
                        "-d" | "-D"
                            | "-m"
                            | "-M"
                            | "-c"
                            | "-C"
                            | "--delete"
                            | "--move"
                            | "--copy"
                            | "--edit-description"
                            | "--set-upstream-to"
                            | "--unset-upstream"
                    )
                });
                mutating_flag
                    || (!tail.is_empty()
                        && !tail.iter().all(|word| {
                            word.value.starts_with('-')
                                || matches!(word.value.as_str(), "list" | "show")
                        }))
            }
            "tag" => {
                let mutating_flag = tail.iter().any(|word| {
                    matches!(
                        word.value.as_str(),
                        "-d" | "--delete"
                            | "-a"
                            | "--annotate"
                            | "-s"
                            | "--sign"
                            | "-f"
                            | "--force"
                    )
                });
                mutating_flag
                    || (!tail.is_empty()
                        && !tail.iter().all(|word| {
                            word.value.starts_with('-')
                                || matches!(word.value.as_str(), "list" | "show")
                        }))
            }
            "remote" => {
                let mutating_subcommand = tail.first().is_some_and(|word| {
                    matches!(
                        word.value.to_ascii_lowercase().as_str(),
                        "add"
                            | "remove"
                            | "rm"
                            | "rename"
                            | "set-head"
                            | "set-branches"
                            | "set-url"
                            | "prune"
                            | "update"
                    )
                });
                mutating_subcommand
                    || (!tail.is_empty()
                        && !tail.iter().all(|word| {
                            word.value.starts_with('-')
                                || matches!(word.value.as_str(), "list" | "show" | "get-url")
                        }))
            }
            "worktree" => tail
                .first()
                .is_some_and(|word| !word.value.eq_ignore_ascii_case("list")),
            _ => false,
        };
        if writes_output {
            aggregate
                .capabilities
                .extend([ShellCapability::GitRead, ShellCapability::WorkspaceWrite]);
            aggregate.all_read_only = false;
        } else if ambiguous_mutator {
            aggregate.capabilities.push(ShellCapability::GitMutation);
            aggregate.all_read_only = false;
        } else {
            aggregate
                .capabilities
                .extend([ShellCapability::Observation, ShellCapability::GitRead]);
        }
    } else if matches!(subcommand.as_str(), "reset" | "clean") {
        aggregate
            .capabilities
            .extend([ShellCapability::GitMutation, ShellCapability::DeleteFiles]);
        aggregate.destructive = true;
        aggregate.all_read_only = false;
        aggregate
            .concrete_targets
            .push("repository:index-and-worktree".to_string());
        aggregate.saw_destructive_target = true;
    } else if GIT_MUTATION_SUBCOMMANDS.contains(&subcommand.as_str()) {
        aggregate.capabilities.push(ShellCapability::GitMutation);
        aggregate.all_read_only = false;
    } else {
        mark_unknown(aggregate);
    }
    aggregate.dynamic |= words.iter().any(|word| word.dynamic);
    if aggregate.dynamic {
        aggregate.all_read_only = false;
    }
}

fn classify_delete(words: &[Word], command_name: &str, aggregate: &mut Aggregate) {
    aggregate.capabilities.extend([
        ShellCapability::WorkspaceWrite,
        ShellCapability::DeleteFiles,
    ]);
    aggregate.destructive = true;
    aggregate.all_read_only = false;
    let targets = if command_name == "remove-item" {
        extract_powershell_paths(words, &["-path", "-literalpath"])
    } else {
        positional_arguments(words, 1)
    };
    aggregate.add_destructive_targets(targets);
}

fn classify_network(words: &[Word], aggregate: &mut Aggregate) {
    aggregate
        .capabilities
        .extend([ShellCapability::Network, ShellCapability::StartProcess]);
    aggregate.all_read_only = false;
    let writes_file = words.iter().any(|word| {
        matches!(
            word.value.to_ascii_lowercase().as_str(),
            "-o" | "--output" | "--output-dir" | "-outfile"
        )
    });
    if writes_file {
        aggregate.capabilities.push(ShellCapability::WorkspaceWrite);
    }
    aggregate.dynamic |= words.iter().any(|word| word.dynamic);
}

fn classify_dynamic_runtime(words: &[Word], aggregate: &mut Aggregate) {
    aggregate.capabilities.extend([
        ShellCapability::StartProcess,
        ShellCapability::DynamicExecution,
    ]);
    aggregate.dynamic = true;
    aggregate.all_read_only = false;
    if let Some(script) = words.iter().skip(1).find(|word| {
        !word.value.starts_with('-')
            && !matches!(
                word.value.to_ascii_lowercase().as_str(),
                "-c" | "-command" | "/c"
            )
    }) {
        aggregate.add_targets(std::iter::once(script));
    }
}

fn classify_package_manager(words: &[Word], aggregate: &mut Aggregate) {
    aggregate.capabilities.extend([
        ShellCapability::StartProcess,
        ShellCapability::DynamicExecution,
    ]);
    aggregate.dynamic = true;
    aggregate.all_read_only = false;
    if words.get(1).is_some_and(|word| {
        PACKAGE_WRITE_SUBCOMMANDS.contains(&word.value.to_ascii_lowercase().as_str())
    }) {
        aggregate
            .capabilities
            .extend([ShellCapability::WorkspaceWrite, ShellCapability::Network]);
    }
}

fn classify_cargo(words: &[Word], aggregate: &mut Aggregate) {
    aggregate.capabilities.extend([
        ShellCapability::StartProcess,
        ShellCapability::DynamicExecution,
    ]);
    aggregate.dynamic = true;
    aggregate.all_read_only = false;
    if words.get(1).is_some_and(|word| {
        CARGO_WRITE_SUBCOMMANDS.contains(&word.value.to_ascii_lowercase().as_str())
    }) {
        aggregate
            .capabilities
            .extend([ShellCapability::WorkspaceWrite, ShellCapability::Network]);
    }
}

fn mark_unknown(aggregate: &mut Aggregate) {
    aggregate.capabilities.push(ShellCapability::Unknown);
    aggregate.saw_unknown = true;
    aggregate.all_read_only = false;
}

fn mark_unknown_dynamic(aggregate: &mut Aggregate) {
    mark_unknown(aggregate);
    aggregate
        .capabilities
        .push(ShellCapability::DynamicExecution);
    aggregate.dynamic = true;
}

fn git_subcommand(words: &[Word]) -> Option<(usize, String)> {
    let mut index = 1;
    while index < words.len() {
        let value = words[index].value.as_str();
        let lower = value.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "-c" | "--git-dir" | "--work-tree" | "--namespace"
        ) {
            index += 2;
            continue;
        }
        if lower.starts_with("-c=")
            || lower.starts_with("--git-dir=")
            || lower.starts_with("--work-tree=")
            || lower.starts_with("--namespace=")
            || lower == "--no-pager"
            || lower == "--paginate"
            || lower == "--literal-pathspecs"
        {
            index += 1;
            continue;
        }
        if value.starts_with('-') {
            index += 1;
            continue;
        }
        return Some((index, lower));
    }
    None
}

fn git_workdir(words: &[Word]) -> Option<&Word> {
    let index = words.iter().position(|word| word.value == "-C")?;
    words.get(index + 1)
}

fn extract_powershell_paths<'a>(words: &'a [Word], named: &[&str]) -> Vec<&'a Word> {
    let mut targets = Vec::new();
    let mut index = 1;
    while index < words.len() {
        let lower = words[index].value.to_ascii_lowercase();
        if named.contains(&lower.as_str()) {
            if let Some(target) = words.get(index + 1) {
                targets.push(target);
            }
            index += 2;
            continue;
        }
        index += 1;
    }
    if !targets.is_empty() {
        return targets;
    }

    let positionals = positional_arguments(words, 1);
    let positional_index = usize::from(
        words
            .first()
            .is_some_and(|word| normalize_command_name(&word.value) == "select-string"),
    );
    positionals
        .get(positional_index)
        .copied()
        .into_iter()
        .collect()
}

fn positional_arguments(words: &[Word], start: usize) -> Vec<&Word> {
    words
        .iter()
        .skip(start)
        .filter(|word| {
            let value = word.value.as_str();
            value == "-" || (!value.starts_with('-') && !is_cmd_switch(value))
        })
        .collect()
}

fn is_cmd_switch(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "/s" | "/q" | "/f" | "/a" | "/p"
    )
}

fn normalize_command_name(value: &str) -> String {
    let name = value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase();
    name.strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".com"))
        .unwrap_or(&name)
        .to_string()
}

fn lex(command: &str) -> Vec<Lexeme> {
    let chars = command.chars().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut word = String::new();
    let mut word_dynamic = false;
    let mut word_wildcard = false;
    let mut quote = None;
    let mut index = 0;

    let flush_word =
        |output: &mut Vec<Lexeme>, word: &mut String, dynamic: &mut bool, wildcard: &mut bool| {
            if !word.is_empty() {
                output.push(Lexeme::Word(Word {
                    value: std::mem::take(word),
                    dynamic: std::mem::take(dynamic),
                    wildcard: std::mem::take(wildcard),
                }));
            }
        };

    while index < chars.len() {
        let ch = chars[index];
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    word.push(ch);
                }
                index += 1;
            }
            Some('"') => {
                if ch == '"' {
                    quote = None;
                } else {
                    if ch == '$' || ch == '`' || (ch == '%' && percent_expansion_at(&chars, index))
                    {
                        word_dynamic = true;
                    }
                    if matches!(ch, '*' | '?' | '[' | ']') {
                        word_wildcard = true;
                    }
                    word.push(ch);
                }
                index += 1;
            }
            _ => {
                if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                    index += 1;
                    continue;
                }
                if ch.is_whitespace() {
                    flush_word(
                        &mut output,
                        &mut word,
                        &mut word_dynamic,
                        &mut word_wildcard,
                    );
                    if ch == '\n' || ch == '\r' {
                        output.push(Lexeme::Operator(Operator::Sequence));
                    }
                    index += 1;
                    continue;
                }
                let (operator, consumed) = match ch {
                    ';' => (Some(Operator::Sequence), 1),
                    '|' if chars.get(index + 1) == Some(&'|') => (Some(Operator::Sequence), 2),
                    '|' => (Some(Operator::Pipeline), 1),
                    '&' if chars.get(index + 1) == Some(&'&') => (Some(Operator::Sequence), 2),
                    '&' => (Some(Operator::Background), 1),
                    '>' | '<' => {
                        let consumed = usize::from(chars.get(index + 1) == Some(&ch)) + 1;
                        (Some(Operator::Redirection), consumed)
                    }
                    _ => (None, 0),
                };
                if let Some(operator) = operator {
                    flush_word(
                        &mut output,
                        &mut word,
                        &mut word_dynamic,
                        &mut word_wildcard,
                    );
                    output.push(Lexeme::Operator(operator));
                    index += consumed;
                    continue;
                }
                if ch == '$' || ch == '`' || (ch == '%' && percent_expansion_at(&chars, index)) {
                    word_dynamic = true;
                }
                if matches!(ch, '*' | '?' | '[' | ']') {
                    word_wildcard = true;
                }
                word.push(ch);
                index += 1;
            }
        }
    }
    flush_word(
        &mut output,
        &mut word,
        &mut word_dynamic,
        &mut word_wildcard,
    );
    output
}

fn percent_expansion_at(chars: &[char], start: usize) -> bool {
    if chars.get(start) != Some(&'%') {
        return false;
    }
    let mut index = start + 1;
    let mut saw_name = false;
    while let Some(ch) = chars.get(index) {
        if *ch == '%' {
            return saw_name;
        }
        if !(ch.is_ascii_alphanumeric() || *ch == '_') {
            return false;
        }
        saw_name = true;
        index += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_read_only_matrix_is_deliberately_narrow() {
        for command in [
            "rg -n TODO .",
            "Select-String -Path src/*.rs -Pattern TODO",
            "Get-Content -LiteralPath 'Cargo.toml'",
            "git status --short",
            "git diff --no-ext-diff --no-color --",
            "git log --oneline -5",
            "git log --format=%H -5",
            "git show --stat HEAD",
            "git -C repo status; git -C repo log -1",
        ] {
            let analysis = analyze_shell_command(command);
            assert!(
                analysis.is_strictly_read_only(),
                "expected strict read-only classification for {command}: {analysis:?}"
            );
            assert!(!analysis.destructive);
        }
    }

    #[test]
    fn pipelines_redirects_and_dynamic_read_targets_are_not_strict_observations() {
        for command in [
            "git status | Out-File status.txt",
            "git log > log.txt",
            "git log --output=log.txt",
            "Get-Content $path",
            "rg --pre 'python filter.py' TODO .",
            "rg TODO . | Select-String important",
        ] {
            let analysis = analyze_shell_command(command);
            assert!(
                !analysis.is_strictly_read_only(),
                "must remain a process effect: {command}: {analysis:?}"
            );
        }
    }

    #[test]
    fn common_mutations_and_dynamic_runtimes_are_not_mistaken_for_reads() {
        for command in [
            "git add src/lib.rs",
            "git commit -m test",
            "git checkout feature",
            "Set-Content -LiteralPath a.txt -Value changed",
            "python script.py",
            "node script.js",
            "pwsh -File script.ps1",
            "pnpm test",
            "npm install",
            "cargo test",
        ] {
            let analysis = analyze_shell_command(command);
            assert!(!analysis.is_strictly_read_only(), "{command}");
        }
    }

    #[test]
    fn destructive_command_matrix_includes_power_shell_posix_and_git() {
        for command in [
            "Remove-Item -Recurse -LiteralPath 'build'",
            "rm -rf build",
            "del /s build",
            "rmdir /s /q build",
            "git reset --hard HEAD~1",
            "git clean -fdx",
            "format C:",
        ] {
            let analysis = analyze_shell_command(command);
            assert!(analysis.destructive, "{command}: {analysis:?}");
            assert!(!analysis.is_strictly_read_only());
        }
    }

    #[test]
    fn destructive_dynamic_or_wildcard_targets_are_unreviewable() {
        for command in [
            "Remove-Item -Recurse $target",
            "rm -rf \"$target\"",
            "Remove-Item -Force *.tmp",
            "Remove-Item -Recurse (Join-Path C:\\Temp build)",
        ] {
            let analysis = analyze_shell_command(command);
            assert!(
                analysis.is_unreviewable_destructive_action(),
                "{command}: {analysis:?}"
            );
        }
        let concrete = analyze_shell_command("Remove-Item -Recurse -LiteralPath 'build'");
        assert!(!concrete.is_unreviewable_destructive_action());
        assert_eq!(concrete.concrete_targets, vec!["build"]);

        let mixed = analyze_shell_command(
            "Select-String -Path \"*.rs\" -Pattern TODO; Remove-Item -LiteralPath 'build'",
        );
        assert!(!mixed.is_unreviewable_destructive_action());
    }

    #[test]
    fn unknown_commands_remain_unknown_instead_of_becoming_denied_or_read_only() {
        let analysis = analyze_shell_command("custom-tool inspect project");
        assert_eq!(analysis.confidence, ShellAnalysisConfidence::Low);
        assert!(analysis.capabilities.contains(&ShellCapability::Unknown));
        assert!(!analysis.destructive);
        assert!(!analysis.is_strictly_read_only());
    }

    #[test]
    fn network_and_package_managers_expose_coarse_capabilities() {
        let curl = analyze_shell_command("curl -o artifact.zip https://example.test/a.zip");
        assert!(curl.capabilities.contains(&ShellCapability::Network));
        assert!(curl.capabilities.contains(&ShellCapability::WorkspaceWrite));

        let npm = analyze_shell_command("npm install package");
        assert!(npm
            .capabilities
            .contains(&ShellCapability::DynamicExecution));
        assert!(npm.capabilities.contains(&ShellCapability::Network));
        assert!(npm.capabilities.contains(&ShellCapability::WorkspaceWrite));
    }
}
