use std::collections::HashMap;

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
    PowerShellRead,
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
    // Keep filesystem observations in one declarative family. These commands
    // may be followed by an unrecognized PowerShell pipeline transform; the
    // exact read paths they expose are still safe to project into that call's
    // restricted sandbox.
    ("select-string", CommandFamily::PowerShellRead),
    ("sls", CommandFamily::PowerShellRead),
    ("get-content", CommandFamily::PowerShellRead),
    ("gc", CommandFamily::PowerShellRead),
    ("cat", CommandFamily::PowerShellRead),
    ("type", CommandFamily::PowerShellRead),
    ("get-childitem", CommandFamily::PowerShellRead),
    ("get-child-item", CommandFamily::PowerShellRead),
    ("gci", CommandFamily::PowerShellRead),
    ("dir", CommandFamily::PowerShellRead),
    ("ls", CommandFamily::PowerShellRead),
    ("get-item", CommandFamily::PowerShellRead),
    ("gi", CommandFamily::PowerShellRead),
    ("import-csv", CommandFamily::PowerShellRead),
    ("ipcsv", CommandFamily::PowerShellRead),
    ("test-path", CommandFamily::PowerShellRead),
    ("resolve-path", CommandFamily::PowerShellRead),
    ("get-acl", CommandFamily::PowerShellRead),
    ("get-filehash", CommandFamily::PowerShellRead),
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
    analyze_shell_command_nested(command, 0)
}

fn analyze_shell_command_nested(command: &str, nesting: usize) -> ShellCommandAnalysis {
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
    let mut static_values = HashMap::<String, Word>::new();
    for segment in &segments {
        if let Some((name, value)) = static_string_assignment(segment) {
            aggregate.init_if_needed();
            aggregate.capabilities.push(ShellCapability::Observation);
            static_values.insert(name, value);
            continue;
        }
        if let Some(name) = assigned_variable_name(segment) {
            // Never let an earlier literal survive a later dynamic assignment.
            // Destructive review in particular must see the final reference as
            // unresolved instead of authorizing a stale, safer-looking path.
            static_values.remove(&name);
        }
        classify_segment(segment, &static_values, &mut aggregate);
    }

    // PowerShell control flow and expression groups are executable containers,
    // not opaque arguments. Analyze their bodies compositionally so a known
    // capability nested under try/if/foreach, a script block, or $() is not
    // hidden by the outer control keyword.
    if nesting < 32 {
        for nested_command in nested_executable_regions(command) {
            let nested = analyze_shell_command_nested(&nested_command, nesting + 1);
            has_pipeline |= nested.has_pipeline;
            has_redirection |= nested.has_redirection;
            has_background_operator |= nested.has_background_operator;
            aggregate.merge_nested(&nested);
        }
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

    fn add_resolved_targets<'a>(
        &mut self,
        words: impl IntoIterator<Item = &'a Word>,
        static_values: &HashMap<String, Word>,
    ) {
        let resolved = words
            .into_iter()
            .map(|word| resolve_static_word(word, static_values))
            .collect::<Vec<_>>();
        self.add_targets(resolved.iter());
    }

    fn add_read_targets<'a>(&mut self, words: impl IntoIterator<Item = &'a Word>) {
        for word in words {
            if word.wildcard && !word.dynamic {
                self.targets_concrete = false;
                if let Some(scope) = wildcard_read_scope(&word.value) {
                    self.concrete_targets.push(scope);
                }
            } else {
                self.add_targets(std::iter::once(word));
            }
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

    fn add_resolved_destructive_targets<'a>(
        &mut self,
        words: impl IntoIterator<Item = &'a Word>,
        static_values: &HashMap<String, Word>,
    ) {
        let resolved = words
            .into_iter()
            .map(|word| resolve_static_word(word, static_values))
            .collect::<Vec<_>>();
        self.add_destructive_targets(resolved.iter());
    }

    fn merge_nested(&mut self, nested: &ShellCommandAnalysis) {
        let found_known_capability = nested.capabilities.iter().any(|capability| {
            !matches!(
                capability,
                ShellCapability::Observation | ShellCapability::Unknown
            )
        });
        if !found_known_capability && !nested.destructive {
            return;
        }

        self.init_if_needed();
        self.capabilities
            .extend(nested.capabilities.iter().copied());
        self.destructive |= nested.destructive;
        self.dynamic |= nested.dynamic;
        self.concrete_targets
            .extend(nested.concrete_targets.iter().cloned());
        self.targets_concrete &= nested.targets_concrete;
        self.saw_unknown |= nested.capabilities.contains(&ShellCapability::Unknown);
        self.all_read_only &= nested.is_strictly_read_only();
        self.destructive_targets_concrete &= nested.destructive_targets_concrete;
        self.saw_destructive_target |= nested.has_destructive_target;
    }
}

fn nested_executable_regions(command: &str) -> Vec<String> {
    let chars = command.chars().collect::<Vec<_>>();
    let mut regions = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\'' => index = skip_single_quoted(&chars, index),
            '"' => index = scan_double_quoted(&chars, index, &mut regions),
            '#' => index = skip_line_comment(&chars, index),
            '<' if chars.get(index + 1) == Some(&'#') => index = skip_block_comment(&chars, index),
            '{' => {
                if let Some(end) = find_matching_group(&chars, index, '{', '}') {
                    regions.push(chars[index + 1..end].iter().collect());
                    index = end + 1;
                } else {
                    index += 1;
                }
            }
            '(' => {
                if let Some(end) = find_matching_group(&chars, index, '(', ')') {
                    regions.push(chars[index + 1..end].iter().collect());
                    index = end + 1;
                } else {
                    index += 1;
                }
            }
            '`' => index = (index + 2).min(chars.len()),
            _ => index += 1,
        }
    }
    regions
}

fn scan_double_quoted(chars: &[char], start: usize, regions: &mut Vec<String>) -> usize {
    let mut index = start + 1;
    while index < chars.len() {
        match chars[index] {
            '`' => index = (index + 2).min(chars.len()),
            '"' => return index + 1,
            '$' if chars.get(index + 1) == Some(&'(') => {
                let open = index + 1;
                if let Some(end) = find_matching_group(chars, open, '(', ')') {
                    regions.push(chars[open + 1..end].iter().collect());
                    index = end + 1;
                } else {
                    index += 2;
                }
            }
            _ => index += 1,
        }
    }
    index
}

fn find_matching_group(chars: &[char], start: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 1usize;
    let mut index = start + 1;
    while index < chars.len() {
        match chars[index] {
            '\'' => index = skip_single_quoted(chars, index),
            '"' => {
                let mut ignored_regions = Vec::new();
                index = scan_double_quoted(chars, index, &mut ignored_regions);
            }
            '#' => index = skip_line_comment(chars, index),
            '<' if chars.get(index + 1) == Some(&'#') => index = skip_block_comment(chars, index),
            '`' => index = (index + 2).min(chars.len()),
            current if current == open => {
                depth += 1;
                index += 1;
            }
            current if current == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn skip_single_quoted(chars: &[char], start: usize) -> usize {
    let mut index = start + 1;
    while index < chars.len() {
        if chars[index] == '\'' {
            if chars.get(index + 1) == Some(&'\'') {
                index += 2;
            } else {
                return index + 1;
            }
        } else {
            index += 1;
        }
    }
    index
}

fn skip_line_comment(chars: &[char], start: usize) -> usize {
    chars[start..]
        .iter()
        .position(|ch| matches!(ch, '\r' | '\n'))
        .map_or(chars.len(), |offset| start + offset + 1)
}

fn skip_block_comment(chars: &[char], start: usize) -> usize {
    let mut index = start + 2;
    while index + 1 < chars.len() {
        if chars[index] == '#' && chars[index + 1] == '>' {
            return index + 2;
        }
        index += 1;
    }
    chars.len()
}

fn static_string_assignment(words: &[Word]) -> Option<(String, Word)> {
    let (name, value) = match words {
        [name, equals, value] if equals.value == "=" => {
            (variable_reference(&name.value)?, value.clone())
        }
        [assignment] => {
            let (name, value) = assignment.value.split_once('=')?;
            let name = variable_reference(name)?;
            let value = value.trim();
            if value.is_empty() {
                return None;
            }
            (
                name,
                Word {
                    value: value.to_string(),
                    dynamic: value.contains('$')
                        || value.contains('`')
                        || value.starts_with('(')
                        || value.starts_with('{'),
                    wildcard: value.chars().any(|ch| matches!(ch, '*' | '?' | '[' | ']')),
                },
            )
        }
        _ => return None,
    };
    if value.dynamic
        || value.value.contains("::")
        || !looks_like_static_path_literal(&value.value)
        || looks_like_dynamic_target(&value.value)
    {
        return None;
    }
    Some((name, value))
}

fn assigned_variable_name(words: &[Word]) -> Option<String> {
    match words {
        [name, equals, ..] if equals.value == "=" => variable_reference(&name.value),
        [assignment, ..] => assignment
            .value
            .split_once('=')
            .and_then(|(name, _)| variable_reference(name)),
        _ => None,
    }
}

fn looks_like_static_path_literal(value: &str) -> bool {
    value.contains(['/', '\\'])
        || value.starts_with('.')
        || value.as_bytes().get(1) == Some(&b':')
        || value.rsplit_once('.').is_some_and(|(stem, extension)| {
            !stem.is_empty()
                && !extension.is_empty()
                && extension
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        })
}

fn variable_reference(value: &str) -> Option<String> {
    let value = value.trim();
    let name = value
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .or_else(|| value.strip_prefix('$'))?;
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }
    Some(name.to_ascii_lowercase())
}

fn resolve_static_word(word: &Word, static_values: &HashMap<String, Word>) -> Word {
    variable_reference(&word.value)
        .and_then(|name| static_values.get(&name))
        .cloned()
        .unwrap_or_else(|| word.clone())
}

const DOTNET_READ_INVOCATIONS: &[&str] = &[
    "[system.io.file]::readallbytes(",
    "[io.file]::readallbytes(",
    "[system.io.file]::readalltext(",
    "[io.file]::readalltext(",
    "[system.io.file]::readalllines(",
    "[io.file]::readalllines(",
    "[system.io.file]::openread(",
    "[io.file]::openread(",
    "[system.io.file]::opentext(",
    "[io.file]::opentext(",
    "[system.io.directory]::enumeratefiles(",
    "[io.directory]::enumeratefiles(",
    "[system.io.directory]::enumeratedirectories(",
    "[io.directory]::enumeratedirectories(",
    "[system.io.directory]::enumeratefilesystementries(",
    "[io.directory]::enumeratefilesystementries(",
    "[system.io.directory]::getfiles(",
    "[io.directory]::getfiles(",
    "[system.io.directory]::getdirectories(",
    "[io.directory]::getdirectories(",
    "[system.io.directory]::getfilesystementries(",
    "[io.directory]::getfilesystementries(",
    "[system.io.streamreader]::new(",
    "[io.streamreader]::new(",
    "[microsoft.visualbasic.fileio.textfieldparser]::new(",
];

fn classify_known_dotnet_read(
    words: &[Word],
    static_values: &HashMap<String, Word>,
    aggregate: &mut Aggregate,
) -> bool {
    let argument = direct_dotnet_read_argument(words).or_else(|| new_object_reader_argument(words));
    let Some(argument) = argument else {
        return false;
    };

    aggregate
        .capabilities
        .extend([ShellCapability::Observation, ShellCapability::ReadFiles]);
    let mut argument = resolve_static_word(&argument, static_values);
    // .NET APIs receive literal strings rather than PowerShell provider globs.
    argument.wildcard = false;
    aggregate.dynamic |= argument.dynamic;
    if argument.dynamic {
        aggregate.all_read_only = false;
    }
    aggregate.add_read_targets(std::iter::once(&argument));
    true
}

fn direct_dotnet_read_argument(words: &[Word]) -> Option<Word> {
    if let Some(argument) = words
        .first()
        .and_then(|word| dotnet_read_argument(&word.value))
    {
        return Some(argument);
    }
    match words {
        [name, equals, expression, ..]
            if equals.value == "=" && variable_reference(&name.value).is_some() =>
        {
            dotnet_read_argument(&expression.value)
        }
        _ => None,
    }
}

fn dotnet_read_argument(value: &str) -> Option<Word> {
    let lower = value.to_ascii_lowercase();
    for invocation in DOTNET_READ_INVOCATIONS {
        let Some(start) = lower.find(invocation) else {
            continue;
        };
        let start = start + invocation.len();
        let tail = value.get(start..)?;
        let end = tail.find(')')?;
        let argument = tail[..end].split(',').next()?.trim();
        if argument.is_empty() {
            return None;
        }
        return Some(inline_argument_word(argument));
    }
    None
}

fn new_object_reader_argument(words: &[Word]) -> Option<Word> {
    let command = words.first()?;
    if normalize_command_name(&command.value) != "new-object" {
        return None;
    }
    let type_index = words.iter().position(|word| {
        matches!(
            word.value.to_ascii_lowercase().as_str(),
            "system.io.streamreader"
                | "io.streamreader"
                | "microsoft.visualbasic.fileio.textfieldparser"
        )
    })?;
    let argument_index = words
        .iter()
        .position(|word| word.value.eq_ignore_ascii_case("-argumentlist"))
        .map(|index| index + 1)
        .unwrap_or(type_index + 1);
    words.get(argument_index).cloned()
}

fn inline_argument_word(value: &str) -> Word {
    let value = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .or_else(|| {
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
        })
        .unwrap_or(value)
        .trim()
        .to_string();
    Word {
        dynamic: value.contains('$')
            || value.contains('`')
            || value.starts_with('(')
            || value.starts_with('{'),
        wildcard: false,
        value,
    }
}

fn wildcard_read_scope(value: &str) -> Option<String> {
    let wildcard = value.find(|ch| matches!(ch, '*' | '?' | '[' | ']'))?;
    let prefix = &value[..wildcard];
    let separator = prefix.rfind(['/', '\\']);
    let scope = match separator {
        Some(0) => &prefix[..1],
        Some(2) if prefix.as_bytes().get(1) == Some(&b':') => &prefix[..3],
        Some(index) => &prefix[..index],
        None => ".",
    };
    (!scope.is_empty()).then(|| scope.to_string())
}

fn looks_like_dynamic_target(value: &str) -> bool {
    let value = value.trim_start();
    value.starts_with('(')
        || value.starts_with("@(")
        || value.starts_with("${")
        || value.starts_with("$(")
        || value.starts_with('{')
}

fn classify_segment(
    words: &[Word],
    static_values: &HashMap<String, Word>,
    aggregate: &mut Aggregate,
) {
    aggregate.init_if_needed();
    if classify_known_dotnet_read(words, static_values, aggregate) {
        return;
    }
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
        aggregate.add_resolved_targets(words.get(0..1).unwrap_or_default(), static_values);
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
        CommandFamily::PowerShellRead => classify_read_command(words, static_values, aggregate),
        CommandFamily::Git => classify_git(words, static_values, aggregate),
        CommandFamily::Delete => classify_delete(words, &command_name, static_values, aggregate),
        CommandFamily::SetContent | CommandFamily::OutFile => {
            aggregate.capabilities.push(ShellCapability::WorkspaceWrite);
            aggregate.all_read_only = false;
            aggregate.add_resolved_targets(
                extract_powershell_paths(words, &["-path", "-literalpath", "-filepath"]),
                static_values,
            );
        }
        CommandFamily::Network => classify_network(words, aggregate),
        CommandFamily::DynamicRuntime => classify_dynamic_runtime(words, aggregate),
        CommandFamily::PackageManager => classify_package_manager(words, aggregate),
        CommandFamily::Cargo => classify_cargo(words, aggregate),
        CommandFamily::Format => {
            aggregate.capabilities.push(ShellCapability::DeleteFiles);
            aggregate.destructive = true;
            aggregate.all_read_only = false;
            aggregate
                .add_resolved_destructive_targets(positional_arguments(words, 1), static_values);
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

fn classify_read_command(
    words: &[Word],
    static_values: &HashMap<String, Word>,
    aggregate: &mut Aggregate,
) {
    aggregate
        .capabilities
        .extend([ShellCapability::Observation, ShellCapability::ReadFiles]);
    let paths = extract_powershell_paths(words, &["-path", "-literalpath"]);
    let literal_path = words
        .iter()
        .any(|word| word.value.eq_ignore_ascii_case("-literalpath"));
    let mut resolved_paths = paths
        .iter()
        .map(|word| resolve_static_word(word, static_values))
        .collect::<Vec<_>>();
    if !literal_path {
        for path in &mut resolved_paths {
            path.wildcard |= path
                .value
                .chars()
                .any(|ch| matches!(ch, '*' | '?' | '[' | ']'));
        }
    }
    let has_unresolved_dynamic_path = resolved_paths.iter().any(|word| word.dynamic);
    aggregate.dynamic |= has_unresolved_dynamic_path;
    if has_unresolved_dynamic_path {
        aggregate.all_read_only = false;
    }
    if !resolved_paths.is_empty() {
        aggregate.add_read_targets(resolved_paths.iter());
    }
}

fn classify_git(words: &[Word], static_values: &HashMap<String, Word>, aggregate: &mut Aggregate) {
    let Some((subcommand_index, subcommand)) = git_subcommand(words) else {
        mark_unknown(aggregate);
        return;
    };
    if let Some(workdir) = git_workdir(words) {
        aggregate.add_resolved_targets(std::iter::once(workdir), static_values);
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

fn classify_delete(
    words: &[Word],
    command_name: &str,
    static_values: &HashMap<String, Word>,
    aggregate: &mut Aggregate,
) {
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
    aggregate.add_resolved_destructive_targets(targets, static_values);
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

    let command_name = words
        .first()
        .map(|word| normalize_command_name(&word.value))
        .unwrap_or_default();
    let positionals = powershell_positional_arguments(words);
    let select_string = matches!(command_name.as_str(), "select-string" | "sls");
    let pattern_is_named = words
        .iter()
        .any(|word| word.value.eq_ignore_ascii_case("-pattern"));
    let positional_index = usize::from(select_string && !pattern_is_named);
    positionals
        .get(positional_index)
        .copied()
        .into_iter()
        .collect()
}

fn powershell_positional_arguments(words: &[Word]) -> Vec<&Word> {
    let mut positionals = Vec::new();
    let mut index = 1;
    while index < words.len() {
        let value = words[index].value.as_str();
        if powershell_parameter_takes_value(value) {
            index += 2;
            continue;
        }
        if value.starts_with('-') {
            index += 1;
            continue;
        }
        positionals.push(&words[index]);
        index += 1;
    }
    positionals
}

fn powershell_parameter_takes_value(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "-algorithm"
            | "-credential"
            | "-delimiter"
            | "-depth"
            | "-encoding"
            | "-erroraction"
            | "-errorvariable"
            | "-exclude"
            | "-filter"
            | "-header"
            | "-include"
            | "-informationaction"
            | "-informationvariable"
            | "-newerthan"
            | "-olderthan"
            | "-outbuffer"
            | "-outvariable"
            | "-parameter"
            | "-pathtype"
            | "-pattern"
            | "-pipelinevariable"
            | "-readcount"
            | "-relativebasepath"
            | "-stream"
            | "-tail"
            | "-totalcount"
            | "-warningaction"
            | "-warningvariable"
    )
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
    fn common_powershell_filesystem_observations_expose_exact_read_targets() {
        for command in [
            "Get-ChildItem -LiteralPath 'C:\\Users\\me\\Downloads'",
            "Import-Csv -LiteralPath 'C:\\Users\\me\\Downloads\\orders.csv'",
            "Test-Path -LiteralPath 'C:\\Users\\me\\Downloads\\orders.csv'",
            "Get-Item -LiteralPath 'C:\\Users\\me\\Downloads\\orders.csv'",
            "Get-FileHash -LiteralPath 'C:\\Users\\me\\Downloads\\orders.csv'",
            "Import-Csv -LiteralPath 'C:\\Users\\me\\Downloads\\货物贸易B2C-订单模板.csv'",
        ] {
            let analysis = analyze_shell_command(command);
            assert!(
                analysis.capabilities.contains(&ShellCapability::ReadFiles),
                "expected filesystem read classification for {command}: {analysis:?}"
            );
            assert!(
                !analysis.capabilities.contains(&ShellCapability::Unknown),
                "known read command must not remain unknown: {command}: {analysis:?}"
            );
            assert!(
                analysis
                    .concrete_targets
                    .iter()
                    .any(|target| target.starts_with("C:\\Users\\me\\Downloads")),
                "expected exact external target for {command}: {analysis:?}"
            );
            assert!(analysis.is_strictly_read_only(), "{command}: {analysis:?}");
        }

        for (command, expected) in [
            (
                "Import-Csv -Encoding UTF8 'C:\\Users\\me\\Downloads\\orders.csv'",
                "C:\\Users\\me\\Downloads\\orders.csv",
            ),
            (
                "Get-FileHash -Algorithm SHA256 'C:\\Users\\me\\Downloads\\orders.csv'",
                "C:\\Users\\me\\Downloads\\orders.csv",
            ),
            (
                "sls -Pattern order 'C:\\Users\\me\\Downloads\\orders.csv'",
                "C:\\Users\\me\\Downloads\\orders.csv",
            ),
        ] {
            let analysis = analyze_shell_command(command);
            assert_eq!(analysis.concrete_targets, vec![expected], "{command}");
        }

        let wildcard =
            analyze_shell_command("Get-ChildItem -Path 'C:\\Users\\me\\Downloads\\*.xlsx' -File");
        assert_eq!(wildcard.concrete_targets, vec!["C:\\Users\\me\\Downloads"]);
        assert!(!wildcard.targets_concrete);
        assert!(wildcard.is_strictly_read_only());
    }

    #[test]
    fn literal_path_variables_are_resolved_before_read_intent_is_built() {
        for command in [
            "$path = 'C:\\Users\\me\\Downloads\\orders.csv'; Import-Csv -LiteralPath $path",
            "$path='C:\\Users\\me\\Downloads\\orders.csv'; Get-Content -LiteralPath $path",
        ] {
            let analysis = analyze_shell_command(command);
            assert_eq!(
                analysis.concrete_targets,
                vec!["C:\\Users\\me\\Downloads\\orders.csv"]
            );
            assert!(!analysis.dynamic, "{command}: {analysis:?}");
            assert!(analysis.is_strictly_read_only(), "{command}: {analysis:?}");
        }

        let command_expression = analyze_shell_command("$value = Remove-Item");
        assert!(command_expression
            .capabilities
            .contains(&ShellCapability::Unknown));
        assert!(command_expression
            .capabilities
            .contains(&ShellCapability::DynamicExecution));

        let reassigned = analyze_shell_command(
            "$target = 'C:\\safe'; $target = $env:TEMP; Remove-Item -Recurse -LiteralPath $target",
        );
        assert!(reassigned.is_unreviewable_destructive_action());
        assert!(!reassigned
            .concrete_targets
            .contains(&"C:\\safe".to_string()));
    }

    #[test]
    fn known_read_paths_survive_unclassified_pipeline_transforms() {
        let analysis = analyze_shell_command(
            "Import-Csv -LiteralPath 'C:\\Users\\me\\Downloads\\orders.csv' | Where-Object Status -eq open",
        );
        assert!(analysis.capabilities.contains(&ShellCapability::ReadFiles));
        assert!(analysis.capabilities.contains(&ShellCapability::Unknown));
        assert_eq!(
            analysis.concrete_targets,
            vec!["C:\\Users\\me\\Downloads\\orders.csv"]
        );
        assert!(!analysis.is_strictly_read_only());
    }

    #[test]
    fn known_dotnet_read_apis_expose_exact_targets_without_allowing_writes() {
        for command in [
            "[System.IO.File]::ReadAllBytes('C:\\Users\\me\\Downloads\\orders.xlsx')",
            "$reader = [System.IO.StreamReader]::new('C:\\Users\\me\\Downloads\\orders.csv'); $reader.ReadToEnd()",
            "New-Object System.IO.StreamReader -ArgumentList 'C:\\Users\\me\\Downloads\\orders.csv'",
            "[Microsoft.VisualBasic.FileIO.TextFieldParser]::new('C:\\Users\\me\\Downloads\\orders.csv')",
        ] {
            let analysis = analyze_shell_command(command);
            assert!(
                analysis.capabilities.contains(&ShellCapability::ReadFiles),
                "expected .NET read classification for {command}: {analysis:?}"
            );
            assert!(
                analysis
                    .concrete_targets
                    .iter()
                    .any(|target| target.starts_with("C:\\Users\\me\\Downloads")),
                "expected exact .NET read target for {command}: {analysis:?}"
            );
            assert!(!analysis
                .capabilities
                .contains(&ShellCapability::WorkspaceWrite));
        }

        let write = analyze_shell_command(
            "[System.IO.File]::WriteAllText('C:\\Users\\me\\Downloads\\orders.txt', 'changed')",
        );
        assert!(write.capabilities.contains(&ShellCapability::Unknown));
        assert!(!write.capabilities.contains(&ShellCapability::ReadFiles));

        let wrapped_delete = analyze_shell_command(
            "Remove-Item ([System.IO.File]::ReadAllText('C:\\Users\\me\\Downloads\\target.txt'))",
        );
        assert!(wrapped_delete.destructive);
        assert!(wrapped_delete
            .capabilities
            .contains(&ShellCapability::DeleteFiles));

        let dynamic_wrapper = analyze_shell_command(
            "Invoke-Expression \"[System.IO.File]::ReadAllText('C:\\Users\\me\\Downloads\\orders.txt')\"",
        );
        assert!(dynamic_wrapper
            .capabilities
            .contains(&ShellCapability::Unknown));
        assert!(!dynamic_wrapper
            .capabilities
            .contains(&ShellCapability::ReadFiles));
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

    #[test]
    fn nested_powershell_commands_contribute_their_capabilities() {
        for command in [
            "$uri = 'https://example.test'; try { Invoke-WebRequest -Uri $uri } catch { Write-Error $_ }",
            "if ($ready) { Invoke-RestMethod -Uri 'https://example.test' }",
            "foreach ($uri in $uris) { curl $uri }",
            "& { wget 'https://example.test' }",
            "$response = $(Invoke-WebRequest -Uri 'https://example.test')",
            "(Invoke-RestMethod -Uri 'https://example.test')",
            "\"result: $(Invoke-WebRequest -Uri 'https://example.test')\"",
        ] {
            let analysis = analyze_shell_command(command);
            assert!(
                analysis.capabilities.contains(&ShellCapability::Network),
                "nested network command was missed: {command}: {analysis:?}"
            );
        }
    }

    #[test]
    fn quoted_or_commented_command_names_are_not_executable_regions() {
        for command in [
            "Write-Output 'Invoke-WebRequest https://example.test'",
            "# $(Invoke-WebRequest -Uri 'https://example.test')",
            "<# { Invoke-RestMethod -Uri 'https://example.test' } #>",
        ] {
            let analysis = analyze_shell_command(command);
            assert!(
                !analysis.capabilities.contains(&ShellCapability::Network),
                "non-executable text was classified as network: {command}: {analysis:?}"
            );
        }
    }
}
