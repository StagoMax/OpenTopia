use super::{
    decode_typed_tool_input, derived_tool_schema, enforce_policy_decision, enforce_read_policy,
    normalize_workspace_path, tool_resource_key, Tool, ToolExecutionPolicy, ToolInvocationContext,
    TypedTool,
};
use crate::execution::{ExecRequest, ExecutionContext, ExecutionEnvironment};
use crate::execution_authorization::{ProcessLifetime, ToolExecutionIntent};
use crate::model::{ToolCall, ToolResult};
use crate::policy::{PolicyDecision, PolicyEngine};
use crate::tool_output_truncation::{truncate_tool_result_at_source, ToolOutputSourceKind};
use anyhow::Context;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

pub struct WorkspaceSearchTool;

const DEFAULT_SEARCH_MAX_RESULTS: usize = 100;
const SEARCH_MAX_RESULTS_LIMIT: usize = 1_000;
const FALLBACK_MAX_FILE_BYTES: u64 = 1_048_576;

fn search_path(path: Option<&str>) -> &str {
    path.map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or(".")
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SearchInput {
    /// Search pattern passed to rg, or substring for fallback search.
    query: String,
    /// Optional file or directory path relative to workspace.
    #[serde(default)]
    path: Option<String>,
    /// Treat the query as literal text instead of a regular expression.
    #[serde(default)]
    fixed_strings: bool,
    /// Return only matches bounded by non-word characters.
    #[serde(default)]
    word_match: bool,
    /// Maximum matching lines to return.
    #[serde(default)]
    #[schemars(range(min = 1, max = 1000))]
    max_results: Option<usize>,
    /// Number of source lines before and after each match to include.
    #[serde(default)]
    #[schemars(range(min = 0, max = 20))]
    context_lines: Option<usize>,
}

#[async_trait]
impl TypedTool for WorkspaceSearchTool {
    type Input = SearchInput;

    fn name(&self) -> &str {
        "workspace_search"
    }

    fn description(&self) -> &str {
        "Recursively search workspace text for candidate definitions and references with ripgrep, falling back to a literal scan. Set context_lines (0-20) to include numbered surrounding source lines and structured match locations that can be passed to filesystem read. Text matches are evidence to confirm by reading code, not semantic symbol resolution."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![tool_resource_key(
            "tree",
            search_path(input.path.as_deref()),
        )])
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        ToolExecutionIntent::observation([PathBuf::from(search_path(input.path.as_deref()))])
            .with_process_lifetime(ProcessLifetime::OneShot)
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let query = input.query.trim();
        anyhow::ensure!(!query.is_empty(), "search requires a query");
        let relative = search_path(input.path.as_deref());
        let max_results = input
            .max_results
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_SEARCH_MAX_RESULTS)
            .min(SEARCH_MAX_RESULTS_LIMIT);
        let fixed_strings = input.fixed_strings;
        let word_match = input.word_match;
        let context_lines = input.context_lines.unwrap_or(0).min(20);

        let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_read_policy(&ctx, &logical_path)?;
        let path = ctx.environment.resolve_read_path(&logical_path)?;

        let search_arg = search_command_path(relative, &path);
        let result = match run_rg_search(
            ctx.environment.as_ref(),
            &ctx.workspace_root,
            &search_arg,
            query,
            max_results,
            fixed_strings,
            word_match,
            context_lines,
        )
        .await?
        {
            Some(result) => result,
            None => {
                run_fallback_search(
                    ctx.workspace_root.clone(),
                    path.clone(),
                    ctx.policy.clone(),
                    query.to_string(),
                    max_results,
                    word_match,
                    context_lines,
                )
                .await?
            }
        };

        let metadata = json!({
            "query": query,
            "path": path.display().to_string(),
            "engine": result.engine,
            "matches": result.matches,
            "returnedMatches": result.returned_matches,
            "maxResults": max_results,
            "fixedStrings": fixed_strings,
            "wordMatch": word_match,
            "contextLines": context_lines,
            "locations": result.locations,
            "truncated": result.truncated,
            "originalBytes": result.original_bytes,
            "outputBytes": result.output_bytes,
            "fallback": result.fallback
        });

        let tool_result = ToolResult {
            call_id,
            output: result.output,
            content: Vec::new(),
            metadata,
        };
        Ok(truncate_tool_result_at_source(
            "workspace_search",
            tool_result,
            ToolOutputSourceKind::WorkspaceSearch,
            ctx.state.as_ref(),
            ctx.thread_id,
        ))
    }
}

impl_typed_tool!(WorkspaceSearchTool);

pub(super) struct SearchRun {
    engine: &'static str,
    pub(super) output: String,
    matches: usize,
    returned_matches: usize,
    pub(super) locations: Vec<Value>,
    truncated: bool,
    original_bytes: usize,
    output_bytes: usize,
    fallback: Value,
}

struct RgCommandOutput {
    success: bool,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    sandbox: Value,
}

struct FallbackCollector {
    lines: Vec<String>,
    locations: Vec<Value>,
    matches: usize,
    original_bytes: usize,
    files_scanned: usize,
    files_skipped: usize,
    policy_skipped: usize,
    max_results: usize,
    context_lines: usize,
}

impl FallbackCollector {
    fn new(max_results: usize, context_lines: usize) -> Self {
        Self {
            lines: Vec::new(),
            locations: Vec::new(),
            matches: 0,
            original_bytes: 0,
            files_scanned: 0,
            files_skipped: 0,
            policy_skipped: 0,
            max_results,
            context_lines,
        }
    }

    fn push_match(&mut self, line: String, location: Value) {
        self.matches += 1;
        self.original_bytes += line.len() + 1;
        if self.lines.len() < self.max_results {
            self.lines.push(line);
            self.locations.push(location);
        }
    }
}

async fn run_rg_search(
    environment: &dyn ExecutionEnvironment,
    workspace_root: &Path,
    search_path: &Path,
    query: &str,
    max_results: usize,
    fixed_strings: bool,
    word_match: bool,
    context_lines: usize,
) -> anyhow::Result<Option<SearchRun>> {
    let mut args = vec![
        "--line-number".to_string(),
        "--column".to_string(),
        "--color".to_string(),
        "never".to_string(),
        "--no-heading".to_string(),
        "--no-messages".to_string(),
        "--max-count".to_string(),
        max_results.to_string(),
    ];
    if context_lines > 0 {
        args.extend([
            "--json".to_string(),
            "--context".to_string(),
            context_lines.to_string(),
        ]);
    }
    if fixed_strings {
        args.push("--fixed-strings".to_string());
    }
    if word_match {
        args.push("--word-regexp".to_string());
    }
    args.extend([
        "--".to_string(),
        query.to_string(),
        search_path.to_string_lossy().into_owned(),
    ]);

    let output = if cfg!(windows) {
        // The search path and read policy were already resolved above. Running
        // this read-only executable through the Windows process sandbox can
        // spend its entire timeout applying ACLs to a large dirty workspace;
        // invoke rg directly so search latency is independent of workspace
        // size. No shell is involved and rg receives only bounded arguments.
        let mut command = tokio::process::Command::new("rg");
        command
            .current_dir(workspace_root)
            .args(&args)
            .kill_on_drop(true);
        match tokio::time::timeout(Duration::from_secs(15), command.output()).await {
            Ok(Ok(output)) => RgCommandOutput {
                success: output.status.success(),
                exit_code: output.status.code(),
                stdout: output.stdout,
                stderr: output.stderr,
                sandbox: json!({ "mode": "host_read_only" }),
            },
            Ok(Err(error)) if error.kind() == ErrorKind::NotFound || fixed_strings => {
                return Ok(None);
            }
            Ok(Err(error)) => return Err(error).context("failed to run host rg search"),
            Err(_) if fixed_strings => return Ok(None),
            Err(_) => anyhow::bail!("host rg search timed out after 15s"),
        }
    } else {
        match environment
            .exec(
                ExecRequest::new("rg").args(args),
                ExecutionContext::with_timeout(Duration::from_secs(30)),
            )
            .await
        {
            Ok(output) => RgCommandOutput {
                success: output.success,
                exit_code: output.exit_code,
                stdout: output.stdout,
                stderr: output.stderr,
                sandbox: serde_json::to_value(output.sandbox).unwrap_or(Value::Null),
            },
            Err(err) if is_not_found_error(&err) || fixed_strings => return Ok(None),
            Err(err) => return Err(err).context("failed to run rg search"),
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.success && output.exit_code != Some(1) && fixed_strings {
        return Ok(None);
    }
    if !output.success && output.exit_code != Some(1) {
        anyhow::bail!(
            "rg search failed ({:?})\n{}",
            output.exit_code,
            truncate(&stderr, 12_000)
        );
    }

    let fallback = json!({ "used": false, "sandbox": output.sandbox });
    if context_lines > 0 {
        return parse_rg_json_context(&stdout, max_results, context_lines, fallback).map(Some);
    }

    Ok(Some(finalize_search_run(
        "rg",
        stdout.lines().map(str::to_string).collect(),
        stdout.lines().count(),
        stdout.len(),
        max_results,
        Vec::new(),
        fallback,
    )))
}

pub(super) async fn run_fallback_search(
    workspace_root: PathBuf,
    search_path: PathBuf,
    policy: Arc<dyn PolicyEngine>,
    query: String,
    max_results: usize,
    word_match: bool,
    context_lines: usize,
) -> anyhow::Result<SearchRun> {
    tokio::task::spawn_blocking(move || {
        let mut collector = FallbackCollector::new(max_results, context_lines);
        collect_fallback_search(
            &workspace_root,
            &search_path,
            policy.as_ref(),
            &query,
            word_match,
            &mut collector,
        )?;
        let fallback = json!({
            "used": true,
            "mode": if word_match { "literal-word" } else { "substring" },
            "maxFileBytes": FALLBACK_MAX_FILE_BYTES,
            "filesScanned": collector.files_scanned,
            "filesSkipped": collector.files_skipped,
            "policySkipped": collector.policy_skipped
        });
        Ok(finalize_search_run(
            "fallback-substring",
            collector.lines,
            collector.matches,
            collector.original_bytes,
            max_results,
            collector.locations,
            fallback,
        ))
    })
    .await
    .context("fallback search task failed")?
}

fn collect_fallback_search(
    workspace_root: &Path,
    path: &Path,
    policy: &dyn PolicyEngine,
    query: &str,
    word_match: bool,
    collector: &mut FallbackCollector,
) -> anyhow::Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        collector.files_skipped += 1;
        return Ok(());
    }

    if metadata.is_dir() {
        let mut entries = std::fs::read_dir(path)
            .with_context(|| format!("failed to list {}", path.display()))?
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            collect_fallback_search(
                workspace_root,
                &entry.path(),
                policy,
                query,
                word_match,
                collector,
            )?;
        }
        return Ok(());
    }

    if !metadata.is_file() {
        collector.files_skipped += 1;
        return Ok(());
    }

    match policy.inspect_read(path) {
        PolicyDecision::Allow => {}
        PolicyDecision::Deny { .. } | PolicyDecision::Ask { .. } => {
            collector.policy_skipped += 1;
            return Ok(());
        }
    }

    if metadata.len() > FALLBACK_MAX_FILE_BYTES {
        collector.files_skipped += 1;
        return Ok(());
    }

    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => {
            collector.files_skipped += 1;
            return Ok(());
        }
    };
    collector.files_scanned += 1;

    let display_path = display_workspace_path(workspace_root, path);
    let source_lines = contents.lines().collect::<Vec<_>>();
    for (line_index, line) in source_lines.iter().enumerate() {
        if let Some(byte_index) = find_literal_match(line, query, word_match) {
            let column = line[..byte_index].chars().count() + 1;
            let line_number = line_index + 1;
            let rendered = if collector.context_lines == 0 {
                format!("{display_path}:{line_number}:{column}:{line}")
            } else {
                render_search_context(
                    &display_path,
                    line_number,
                    column,
                    &source_lines,
                    collector.context_lines,
                )
            };
            collector.push_match(
                rendered,
                json!({
                    "path": display_path,
                    "line": line_number,
                    "column": column
                }),
            );
        }
    }

    Ok(())
}

pub(super) fn find_literal_match(line: &str, query: &str, word_match: bool) -> Option<usize> {
    if !word_match {
        return line.find(query);
    }

    line.match_indices(query).find_map(|(byte_index, _)| {
        let before = line[..byte_index].chars().next_back();
        let after = line[byte_index + query.len()..].chars().next();
        let bounded_before = before.is_none_or(|character| !is_word_character(character));
        let bounded_after = after.is_none_or(|character| !is_word_character(character));
        (bounded_before && bounded_after).then_some(byte_index)
    })
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn render_search_context(
    path: &str,
    match_line: usize,
    column: usize,
    lines: &[&str],
    context_lines: usize,
) -> String {
    let start = match_line.saturating_sub(context_lines).max(1);
    let end = match_line.saturating_add(context_lines).min(lines.len());
    let width = end.to_string().len();
    let mut rendered = format!("{path}:{match_line}:{column}");
    for line_number in start..=end {
        let marker = if line_number == match_line { '>' } else { ' ' };
        rendered.push_str(&format!(
            "\n{marker} {line_number:>width$} | {}",
            lines[line_number - 1]
        ));
    }
    rendered
}

fn parse_rg_json_context(
    stdout: &str,
    max_results: usize,
    context_lines: usize,
    fallback: Value,
) -> anyhow::Result<SearchRun> {
    #[derive(Clone)]
    struct MatchLocation {
        path: String,
        line: usize,
        column: usize,
    }

    let mut source_lines = HashMap::<String, BTreeMap<usize, String>>::new();
    let mut matches = Vec::<MatchLocation>::new();
    for raw in stdout.lines() {
        let event: Value = serde_json::from_str(raw).context("failed to parse rg JSON output")?;
        let event_type = event.get("type").and_then(Value::as_str);
        if !matches!(event_type, Some("match" | "context")) {
            continue;
        }
        let data = &event["data"];
        let Some(path) = data["path"]["text"].as_str() else {
            continue;
        };
        let Some(line_number) = data["line_number"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
        else {
            continue;
        };
        let Some(text) = data["lines"]["text"].as_str() else {
            continue;
        };
        let text = text.trim_end_matches(['\r', '\n']).to_string();
        source_lines
            .entry(path.to_string())
            .or_default()
            .insert(line_number, text.clone());

        if event_type == Some("match") {
            let byte_column = data["submatches"]
                .as_array()
                .and_then(|submatches| submatches.first())
                .and_then(|submatch| submatch["start"].as_u64())
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(0)
                .min(text.len());
            let byte_column = (0..=byte_column)
                .rev()
                .find(|index| text.is_char_boundary(*index))
                .unwrap_or(0);
            let column = text[..byte_column].chars().count() + 1;
            matches.push(MatchLocation {
                path: path.to_string(),
                line: line_number,
                column,
            });
        }
    }

    let match_count = matches.len();
    let returned_matches = match_count.min(max_results);
    let locations = matches
        .iter()
        .take(max_results)
        .map(|location| {
            json!({
                "path": location.path,
                "line": location.line,
                "column": location.column
            })
        })
        .collect::<Vec<_>>();
    let rendered = matches
        .into_iter()
        .take(max_results)
        .map(|location| {
            let available = source_lines
                .get(&location.path)
                .expect("rg match must have a source line");
            let first = location.line.saturating_sub(context_lines).max(1);
            let last = location.line.saturating_add(context_lines);
            let selected = (first..=last)
                .filter_map(|line_number| {
                    available
                        .get(&line_number)
                        .map(|line| (line_number, line.as_str()))
                })
                .collect::<Vec<_>>();
            let width = selected
                .last()
                .map(|(line_number, _)| line_number.to_string().len())
                .unwrap_or(1);
            let mut block = format!("{}:{}:{}", location.path, location.line, location.column);
            for (line_number, line) in selected {
                let marker = if line_number == location.line {
                    '>'
                } else {
                    ' '
                };
                block.push_str(&format!("\n{marker} {line_number:>width$} | {line}"));
            }
            block
        })
        .collect::<Vec<_>>()
        .join("\n--\n");
    let text = if rendered.is_empty() {
        "(no matches)".to_string()
    } else {
        rendered
    };
    let original_bytes = text.len();
    let output_bytes = text.len();
    Ok(SearchRun {
        engine: "rg",
        output: text,
        matches: match_count,
        returned_matches,
        locations,
        truncated: match_count > max_results,
        original_bytes,
        output_bytes,
        fallback,
    })
}

fn finalize_search_run(
    engine: &'static str,
    lines: Vec<String>,
    matches: usize,
    original_bytes: usize,
    max_results: usize,
    locations: Vec<Value>,
    fallback: Value,
) -> SearchRun {
    let returned_matches = lines.len().min(max_results);
    let text = if lines.is_empty() {
        "(no matches)".to_string()
    } else {
        lines
            .into_iter()
            .take(max_results)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let line_truncated = matches > max_results;
    let output_bytes = text.len();
    SearchRun {
        engine,
        output: text,
        matches,
        returned_matches,
        locations,
        truncated: line_truncated,
        original_bytes,
        output_bytes,
        fallback,
    }
}

fn search_command_path(relative: &str, normalized: &Path) -> PathBuf {
    let candidate = PathBuf::from(relative);
    if candidate.is_absolute() {
        normalized.to_path_buf()
    } else {
        candidate
    }
}

fn display_workspace_path(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn is_not_found_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|err| err.kind() == ErrorKind::NotFound)
    })
}

fn truncate(value: &str, max_chars: usize) -> String {
    let total_chars = value.chars().count();
    if total_chars <= max_chars {
        return value.to_string();
    }

    let head_chars = max_chars / 2;
    let tail_chars = max_chars.saturating_sub(head_chars);
    let mut truncated: String = value.chars().take(head_chars).collect();
    truncated.push_str(&format!(
        "\n\n[{} characters omitted]\n\n",
        total_chars.saturating_sub(max_chars)
    ));
    truncated.extend(value.chars().skip(total_chars.saturating_sub(tail_chars)));
    truncated
}
