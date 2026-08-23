use super::*;

fn schema_contains_reference(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key("$ref") || object.values().any(schema_contains_reference)
        }
        Value::Array(values) => values.iter().any(schema_contains_reference),
        _ => false,
    }
}

#[test]
fn every_static_builtin_uses_an_inline_derived_input_schema() {
    let registry = ToolRegistry::with_builtins();

    for name in registry.list() {
        let tool = registry
            .get(&name)
            .expect("every listed tool must remain resolvable");
        assert!(
            tool.has_derived_input_schema(),
            "static tool {name} bypasses the typed schema adapter"
        );
        let schema = tool.schema();
        assert!(schema.is_object(), "tool {name} schema is not an object");
        assert_eq!(
            schema.get("type").and_then(Value::as_str),
            Some("object"),
            "tool {name} must expose an object-root schema: {schema}"
        );
        assert!(
            !schema_contains_reference(&schema),
            "tool {name} schema contains a non-portable reference: {schema}"
        );
    }
}

#[test]
fn foreground_yield_schema_enforces_runtime_floor_and_allows_two_minutes() {
    let shell = ShellTool.schema();
    assert_eq!(
        shell["properties"]["yieldTimeMs"]["minimum"].as_f64(),
        Some(30_000.0)
    );
    assert_eq!(
        shell["properties"]["yieldTimeMs"]["maximum"].as_f64(),
        Some(120_000.0)
    );

    let browser = BrowserTool.schema();
    let download = &action_schema_branch(&browser, "download")["properties"];
    assert_eq!(download["yieldTimeMs"]["minimum"].as_f64(), Some(30_000.0));
    assert_eq!(download["yieldTimeMs"]["maximum"].as_f64(), Some(120_000.0));
}

fn schema_contains_object_matching(
    value: &Value,
    predicate: &impl Fn(&serde_json::Map<String, Value>) -> bool,
) -> bool {
    match value {
        Value::Object(object) => {
            predicate(object)
                || object
                    .values()
                    .any(|value| schema_contains_object_matching(value, predicate))
        }
        Value::Array(values) => values
            .iter()
            .any(|value| schema_contains_object_matching(value, predicate)),
        _ => false,
    }
}

fn action_schema_branches(schema: &Value) -> &[Value] {
    schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .expect("action tool must expose object branches")
}

pub(super) fn action_schema_branch<'a>(schema: &'a Value, action: &str) -> &'a Value {
    action_schema_branches(schema)
        .iter()
        .find(|branch| {
            branch["properties"]["action"]["enum"]
                .as_array()
                .is_some_and(|values| values.as_slice() == [json!(action)])
        })
        .unwrap_or_else(|| panic!("missing schema branch for action {action}"))
}

fn assert_discriminated_action_schema(tool: &dyn Tool, actions: &[&str]) {
    let schema = tool.schema();
    let branches = action_schema_branches(&schema);
    assert_eq!(branches.len(), actions.len(), "{}", tool.name());
    for action in actions {
        let branch = action_schema_branch(&schema, action);
        assert_eq!(branch["additionalProperties"], false, "{action}");
        assert!(branch["required"]
            .as_array()
            .is_some_and(|required| required.contains(&json!("action"))));
    }
}

#[test]
fn spawn_agent_fork_turns_schema_only_allows_labels_or_positive_counts() {
    let schema = Tool::schema(&SpawnAgentTool);
    let fork_turns = &schema["properties"]["fork_turns"];

    assert!(schema_contains_object_matching(fork_turns, &|object| {
        object.get("type") == Some(&json!("string"))
            && object.get("enum") == Some(&json!(["none", "all"]))
    }));
    assert!(schema_contains_object_matching(fork_turns, &|object| {
        object.get("type") == Some(&json!("integer"))
            && object.get("minimum").and_then(Value::as_f64) == Some(1.0)
    }));
    assert!(schema_contains_object_matching(fork_turns, &|object| {
        object.get("type") == Some(&json!("null"))
    }));

    for fork_turns in [
        json!("none"),
        json!("all"),
        json!(1),
        json!(12),
        Value::Null,
    ] {
        assert!(serde_json::from_value::<SpawnAgentInput>(json!({
            "task_name": "reviewer",
            "message": "review this change",
            "fork_turns": fork_turns,
        }))
        .is_ok());
    }
    for fork_turns in [json!("recent"), json!("0"), json!(0), json!(-1), json!(1.5)] {
        assert!(serde_json::from_value::<SpawnAgentInput>(json!({
            "task_name": "reviewer",
            "message": "review this change",
            "fork_turns": fork_turns,
        }))
        .is_err());
    }
}

#[test]
fn derived_schema_and_typed_decoder_reject_the_same_invalid_shapes() {
    assert_eq!(
        ListFilesTool.input_error(&json!({})).as_deref(),
        Some("arguments.path is required")
    );
    assert_eq!(
        ListFilesTool
            .input_error(&json!({ "path": ".", "unexpected": true }))
            .as_deref(),
        Some("arguments.unexpected is not allowed")
    );
    assert!(WorkspaceSearchTool
        .input_error(&json!({
            "query": "TypedTool",
            "fixedStrings": true,
            "maxResults": 10
        }))
        .is_none());
    assert!(ApplyPatchTool
        .input_error(&json!({
            "patch": "diff --git a/a b/a",
            "operation": { "type": "delete_file", "path": "a" }
        }))
        .is_some());
}
#[test]
fn list_files_requires_an_explicit_workspace_relative_path() {
    let schema = ListFilesTool.schema();

    assert_eq!(schema["required"], json!(["path"]));
    assert_eq!(schema["properties"]["path"]["type"], "string");
}

#[test]
fn detects_common_cross_platform_sandbox_denials() {
    assert!(looks_like_sandbox_denial("Access is denied."));
    assert!(looks_like_sandbox_denial(
        "Access to the path 'C:\\\\outside.txt' is denied."
    ));
    assert!(looks_like_sandbox_denial("CategoryInfo: PermissionDenied"));
    assert!(looks_like_sandbox_denial("bash: Permission denied"));
    assert!(looks_like_sandbox_denial("Operation not permitted"));
    assert!(looks_like_sandbox_denial("Network is unreachable"));
    assert!(!looks_like_sandbox_denial("cargo test failed"));
}

#[test]
fn search_tool_exposes_exact_symbol_controls() {
    let schema = WorkspaceSearchTool.schema();
    let properties = schema["properties"]
        .as_object()
        .expect("search schema properties");

    assert_eq!(properties["fixedStrings"]["type"], "boolean");
    assert_eq!(properties["wordMatch"]["type"], "boolean");
    assert_eq!(properties["contextLines"]["minimum"].as_f64(), Some(0.0));
    assert_eq!(properties["contextLines"]["maximum"].as_f64(), Some(20.0));
    assert!(Tool::description(&WorkspaceSearchTool).contains("not semantic symbol resolution"));
}

#[test]
fn background_read_schema_exposes_a_bounded_wait() {
    let schema = BackgroundOutputTool.schema();
    let read = action_schema_branch(&schema, "read");
    let timeout = &read["properties"]["timeoutMs"];
    assert!(
        schema_contains_object_matching(timeout, &|object| {
            object.get("minimum").and_then(Value::as_f64) == Some(0.0)
                && object.get("maximum").and_then(Value::as_f64) == Some(3_600_000.0)
        }),
        "timeout schema: {timeout}"
    );
    assert!(read["required"]
        .as_array()
        .is_some_and(|required| required.contains(&json!("jobId"))));
    assert!(Tool::description(&BackgroundOutputTool).contains("cancellable wait"));
}

#[test]
fn action_driven_tools_expose_only_action_specific_fields() {
    assert_discriminated_action_schema(
        &SpreadsheetTool,
        &[
            "inspect_delimited",
            "inspect",
            "list_sheets",
            "read_range",
            "read_ranges",
            "read_rows",
            "read_columns",
            "find",
            "filter_rows",
            "validate",
            "fill_template",
            "export_delimited",
            "write",
            "write_rows",
            "write_columns",
            "copy_rows",
            "copy_columns",
            "batch",
        ],
    );
    assert_discriminated_action_schema(
        &BrowserTool,
        &[
            "navigate",
            "observe",
            "screenshot",
            "click",
            "type",
            "select",
            "hover",
            "scroll",
            "switch_target",
            "wait",
            "download",
            "close",
        ],
    );
    assert_discriminated_action_schema(
        &ComputerTool,
        &[
            "list_windows",
            "observe",
            "click",
            "type",
            "keypress",
            "scroll",
            "drag",
            "wait",
            "close",
        ],
    );
    assert_discriminated_action_schema(&PdfTool, &["inspect", "extract", "render", "validate"]);
    assert_discriminated_action_schema(&DocumentTool, &["inspect", "extract", "validate"]);
    assert_discriminated_action_schema(&BackgroundOutputTool, &["read", "list", "write", "stop"]);

    let spreadsheet = SpreadsheetTool.schema();
    let fill_template = &action_schema_branch(&spreadsheet, "fill_template")["properties"];
    assert!(fill_template.get("dataPath").is_some());
    assert!(fill_template.get("templatePath").is_some());
    assert!(fill_template.get("mappings").is_some());
    assert!(fill_template.get("rows").is_none());
    let export_delimited = &action_schema_branch(&spreadsheet, "export_delimited")["properties"];
    assert!(export_delimited.get("outputPath").is_some());
    assert!(export_delimited.get("format").is_some());
    assert!(export_delimited.get("sheets").is_none());
    let read_range = &action_schema_branch(&spreadsheet, "read_range")["properties"];
    assert!(read_range.get("range").is_some());
    assert!(read_range.get("rows").is_none());
    let write_rows = &action_schema_branch(&spreadsheet, "write_rows")["properties"];
    assert!(write_rows.get("rows").is_some());
    assert!(write_rows.get("query").is_none());
    assert!(write_rows.get("operations").is_none());
    let batch = &action_schema_branch(&spreadsheet, "batch")["properties"];
    assert!(batch.get("operations").is_some());
    assert!(batch.get("rows").is_none());

    let browser = BrowserTool.schema();
    let navigate = &action_schema_branch(&browser, "navigate")["properties"];
    assert!(navigate.get("url").is_some());
    assert!(navigate.get("nodeRef").is_none());
    let click = &action_schema_branch(&browser, "click")["properties"];
    assert!(click.get("observationId").is_some());
    assert!(click.get("nodeRef").is_some());
    assert!(click.get("url").is_none());

    let computer = ComputerTool.schema();
    let drag = &action_schema_branch(&computer, "drag")["properties"];
    assert!(drag.get("endX").is_some());
    assert!(drag.get("text").is_none());

    let pdf = PdfTool.schema();
    let extract = &action_schema_branch(&pdf, "extract")["properties"];
    assert!(extract.get("maxCharacters").is_some());
    assert!(extract.get("dpi").is_none());
    let render = &action_schema_branch(&pdf, "render")["properties"];
    assert!(render.get("dpi").is_some());
    assert!(render.get("maxCharacters").is_none());

    let document = DocumentTool.schema();
    let inspect = &action_schema_branch(&document, "inspect")["properties"];
    assert!(inspect.get("includeRelatedParts").is_none());
    let extract = &action_schema_branch(&document, "extract")["properties"];
    assert!(extract.get("includeRelatedParts").is_some());

    let background = BackgroundOutputTool.schema();
    let list = &action_schema_branch(&background, "list")["properties"];
    assert_eq!(list.as_object().map(serde_json::Map::len), Some(1));
    let write = &action_schema_branch(&background, "write")["properties"];
    assert!(write.get("data").is_some());
    assert!(write.get("timeoutMs").is_none());
}

#[test]
fn builtin_action_discriminators_never_use_a_flat_optional_field_bag() {
    let registry = ToolRegistry::with_builtins();
    for name in registry.list() {
        let schema = registry.get(&name).expect("registered tool").schema();
        let flat_actions = schema["properties"]["action"]["enum"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default();
        assert!(
            flat_actions <= 1,
            "tool {name} exposes {flat_actions} actions through one flat property bag"
        );
    }
}

#[test]
fn action_driven_tools_reject_cross_action_or_incomplete_inputs() {
    assert!(SpreadsheetTool
        .input_error(&json!({
            "action": "read_range",
            "path": "book.xlsx",
            "sheet": "Sheet1"
        }))
        .is_some());
    assert!(BrowserTool
        .input_error(&json!({ "action": "navigate" }))
        .is_some());
    assert!(BrowserTool
        .input_error(&json!({
            "action": "click",
            "observationId": "obs",
            "nodeRef": "n1",
            "url": "https://example.com"
        }))
        .is_some());
    assert!(ComputerTool
        .input_error(&json!({
            "action": "drag",
            "observationId": "obs",
            "x": 1,
            "y": 2
        }))
        .is_some());
    assert!(PdfTool
        .input_error(&json!({
            "action": "inspect",
            "path": "report.pdf",
            "dpi": 96
        }))
        .is_some());
    assert!(DocumentTool
        .input_error(&json!({
            "action": "validate",
            "path": "report.docx",
            "maxCharacters": 100
        }))
        .is_some());
    assert!(BackgroundOutputTool.input_error(&json!({})).is_some());
    assert!(BackgroundOutputTool
        .input_error(&json!({ "action": "read" }))
        .is_some());

    assert!(SpreadsheetTool
        .input_error(&json!({
            "action": "read_range",
            "path": "book.xlsx",
            "sheet": "Sheet1",
            "range": {
                "start": { "row": 0, "column": 0 },
                "end": { "row": 1, "column": 1 }
            }
        }))
        .is_none());
    assert!(BackgroundOutputTool
        .input_error(&json!({ "action": "list" }))
        .is_none());
    assert!(BrowserTool
        .input_error(&json!({
            "action": "click",
            "observationId": "obs",
            "nodeRef": "n1"
        }))
        .is_none());
    assert!(ComputerTool
        .input_error(&json!({
            "action": "drag",
            "observationId": "obs",
            "x": 1,
            "y": 2,
            "endX": 3,
            "endY": 4
        }))
        .is_none());
    assert!(PdfTool
        .input_error(&json!({
            "action": "render",
            "path": "report.pdf",
            "pages": [1],
            "dpi": 96
        }))
        .is_none());
    assert!(DocumentTool
        .input_error(&json!({
            "action": "extract",
            "path": "report.docx",
            "includeRelatedParts": true,
            "maxCharacters": 100
        }))
        .is_none());
    assert!(BackgroundOutputTool
        .input_error(&json!({
            "action": "write",
            "jobId": Uuid::new_v4(),
            "data": "hello",
            "appendNewline": true
        }))
        .is_none());
}

#[test]
fn read_file_schema_exposes_mutually_exclusive_line_coordinates() {
    let schema = ReadFileTool.schema();
    let properties = schema["properties"]
        .as_object()
        .expect("read_file schema properties");
    let branches = properties["window"]["anyOf"]
        .as_array()
        .expect("optional typed window branches");
    let tagged_union = branches
        .iter()
        .find_map(|branch| branch.get("oneOf").and_then(Value::as_array))
        .expect("window tagged union");
    assert_eq!(tagged_union.len(), 2);
    assert!(tagged_union.iter().all(|branch| branch["required"]
        .as_array()
        .is_some_and(|required| required.contains(&json!("mode")))));
    assert!(!properties.contains_key("startLine"));
    assert!(!properties.contains_key("offset"));
    assert!(Tool::description(&ReadFileTool).contains("typed window"));
    assert!(ReadFileTool
        .input_error(&json!({
            "path": "src/lib.rs",
            "window": { "mode": "lines", "startLine": 10, "endLine": 20 }
        }))
        .is_none());
    assert!(ReadFileTool
        .input_error(&json!({
            "path": "src/lib.rs",
            "window": { "mode": "characters", "offset": 100, "limit": 500 }
        }))
        .is_none());
    assert!(ReadFileTool
        .input_error(&json!({
            "path": "src/lib.rs",
            "window": {
                "mode": "lines",
                "startLine": 10,
                "offset": 100
            }
        }))
        .is_some());
}
