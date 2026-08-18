use super::*;

#[test]
fn literal_word_matching_respects_identifier_boundaries() {
    assert_eq!(find_literal_match("load();", "load", true), Some(0));
    assert_eq!(find_literal_match("service.load();", "load", true), Some(8));
    assert_eq!(find_literal_match("preload();", "load", true), None);
    assert_eq!(find_literal_match("load_more();", "load", true), None);
    assert_eq!(find_literal_match("preload();", "load", false), Some(3));
}

#[tokio::test]
async fn search_tool_finds_exact_symbol_definitions_and_references_across_files() {
    let id = Uuid::new_v4();
    let workspace_root = std::env::temp_dir().join(format!("opentopia-symbol-search-{id}"));
    let source_root = workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    fs::write(
        source_root.join("definition.rs"),
        "pub fn load() {}\npub fn preload() {}\n",
    )
    .unwrap();
    fs::write(
        source_root.join("caller.rs"),
        "fn run() {\n    load();\n    preload();\n}\n",
    )
    .unwrap();
    fs::write(
        workspace_root.join("literal.txt"),
        "service.load\nserviceXload\n",
    )
    .unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let mut sandbox = LocalSandboxConfig::enforce();
    sandbox.network = crate::sandbox::NetworkPolicy::Allow;
    let context =
        ToolInvocationContext::local_with_sandbox_config(workspace_root.clone(), policy, sandbox);

    let searched = WorkspaceSearchTool
        .execute(
            ToolCall::new(
                "workspace_search",
                json!({
                    "query": "load",
                    "path": "src",
                    "fixedStrings": true,
                    "wordMatch": true
                }),
            ),
            context.clone(),
        )
        .await
        .unwrap();

    assert!(
        searched.output.contains("definition.rs"),
        "unexpected search output: {:?}; metadata: {}",
        searched.output,
        searched.metadata
    );
    assert!(searched.output.contains("caller.rs"));
    assert!(!searched.output.contains("preload"));
    assert_eq!(searched.metadata["matches"], 2);
    assert_eq!(searched.metadata["fixedStrings"], true);
    assert_eq!(searched.metadata["wordMatch"], true);

    let literal = WorkspaceSearchTool
        .execute(
            ToolCall::new(
                "workspace_search",
                json!({
                    "query": "service.load",
                    "path": "literal.txt",
                    "fixedStrings": true
                }),
            ),
            context,
        )
        .await
        .unwrap();
    assert!(literal.output.contains("service.load"));
    assert!(!literal.output.contains("serviceXload"));
    assert_eq!(literal.metadata["matches"], 1);

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn search_tool_returns_numbered_utf8_context_and_structured_location() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-search-context-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::write(
        workspace_root.join("context.txt"),
        "before\r\n🙂目标 value\r\nafter\r\nlast\r\n",
    )
    .unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local(workspace_root.clone(), policy);

    let searched = WorkspaceSearchTool
        .execute(
            ToolCall::new(
                "workspace_search",
                json!({
                    "query": "目标",
                    "path": "context.txt",
                    "fixedStrings": true,
                    "contextLines": 1
                }),
            ),
            context,
        )
        .await
        .unwrap();

    assert!(searched.output.contains("context.txt:2:2"));
    assert!(searched.output.contains("  1 | before"));
    assert!(searched.output.contains("> 2 | 🙂目标 value"));
    assert!(searched.output.contains("  3 | after"));
    assert_eq!(searched.metadata["contextLines"], 1);
    assert_eq!(searched.metadata["locations"][0]["line"], 2);
    assert_eq!(searched.metadata["locations"][0]["column"], 2);

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn fallback_search_returns_the_same_context_contract() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-fallback-context-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let target = workspace_root.join("fallback.txt");
    fs::write(&target, "first\nneedle🙂\nthird\n").unwrap();
    let policy = BasicPolicyEngine::new(workspace_root.clone(), PermissionMode::FullAccess);

    let result = run_fallback_search(
        workspace_root.clone(),
        target,
        Arc::new(policy),
        "needle".to_string(),
        10,
        false,
        1,
    )
    .await
    .unwrap();

    assert!(result.output.contains("  1 | first"));
    assert!(result.output.contains("> 2 | needle🙂"));
    assert!(result.output.contains("  3 | third"));
    assert_eq!(result.locations[0]["line"], 2);
    assert_eq!(result.locations[0]["column"], 1);

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn file_observation_tools_reject_parent_traversal_but_allow_absolute_host_paths() {
    let id = Uuid::new_v4();
    let workspace_root = std::env::temp_dir().join(format!("opentopia-tools-root-{id}"));
    let outside = std::env::temp_dir().join(format!("opentopia-tools-outside-{id}"));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "outside marker").unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local(workspace_root.clone(), policy);

    let traversal = ListFilesTool
        .execute(
            ToolCall::new("list_files", json!({ "path": "../.." })),
            context.clone(),
        )
        .await
        .unwrap_err();
    assert!(traversal.to_string().contains("cannot contain '..'"));

    let outside_path = outside.display().to_string();
    let read = ReadFileTool
        .execute(
            ToolCall::new(
                "read_file",
                json!({ "path": outside.join("secret.txt").display().to_string() }),
            ),
            context.clone(),
        )
        .await
        .expect("read absolute host file");
    assert!(read.output.contains("outside marker"));

    let list = ListFilesTool
        .execute(
            ToolCall::new("list_files", json!({ "path": outside_path })),
            context.clone(),
        )
        .await
        .expect("list absolute host directory");
    assert!(list.output.contains("secret.txt"));

    let read_again = ReadFileTool
        .execute(
            ToolCall::new(
                "read_file",
                json!({ "path": outside.join("secret.txt").display().to_string() }),
            ),
            context.clone(),
        )
        .await
        .expect("repeat absolute host read without approval");
    assert!(read_again.output.contains("outside marker"));

    let search = WorkspaceSearchTool
        .execute(
            ToolCall::new(
                "workspace_search",
                json!({ "query": "marker", "path": outside.display().to_string() }),
            ),
            context,
        )
        .await
        .expect("search absolute host directory");
    assert!(search.output.contains("outside marker"));

    fs::remove_dir_all(workspace_root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

/// Before windowing, everything past the first 16000 characters of a file
/// was unreachable through `read_file`, and a truncated read looked the same
/// to the model as a short file.
#[tokio::test]
async fn read_file_windows_reach_the_end_of_a_long_file() {
    let id = Uuid::new_v4();
    let workspace_root = std::env::temp_dir().join(format!("opentopia-read-window-{id}"));
    fs::create_dir_all(&workspace_root).unwrap();
    let contents = format!("{}TAIL", "z".repeat(READ_FILE_WINDOW_CHARS + 500));
    fs::write(workspace_root.join("long.txt"), &contents).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::Auto,
    ));
    let context = ToolInvocationContext::local(workspace_root.clone(), policy);

    let first = ReadFileTool
        .execute(
            ToolCall::new("read_file", json!({ "path": "long.txt" })),
            context.clone(),
        )
        .await
        .unwrap();
    assert_eq!(first.metadata["offset"], 0);
    assert_eq!(first.metadata["nextOffset"], READ_FILE_WINDOW_CHARS);
    assert_eq!(first.metadata["totalChars"], contents.chars().count());
    assert!(!first.output.contains("TAIL"));
    assert!(first.output.contains("\"mode\":\"characters\""));

    let next = first.metadata["nextOffset"].as_u64().unwrap();
    let second = ReadFileTool
        .execute(
            ToolCall::new("read_file", json!({ "path": "long.txt", "offset": next })),
            context.clone(),
        )
        .await
        .unwrap();
    assert!(second.output.contains("TAIL"), "the tail must be reachable");
    assert!(second.metadata["nextOffset"].is_null());

    let bounded = ReadFileTool
        .execute(
            ToolCall::new("read_file", json!({ "path": "long.txt", "limit": 10 })),
            context,
        )
        .await
        .unwrap();
    assert_eq!(bounded.metadata["nextOffset"], 10);

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn read_artifact_windows_reach_full_ingress_output() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-read-artifact-window-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
    let thread = store
        .create_thread(Some("artifact window".to_string()), workspace_root.clone())
        .expect("create task");
    let contents = format!("{}TAIL", "a".repeat(READ_ARTIFACT_WINDOW_CHARS + 25));
    let artifact = store
        .insert_artifact(Artifact::inline(
            thread.id,
            "tool_output",
            "text/plain; charset=utf-8",
            contents,
            json!({}),
        ))
        .expect("insert artifact");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::Auto,
    ));
    let mut context = ToolInvocationContext::local(workspace_root.clone(), policy);
    context.state = Some(ToolStateStore::new(store));
    context.thread_id = Some(thread.id);

    let first = ReadArtifactTool
        .execute(
            ToolCall::new("read_artifact", json!({ "artifactId": artifact.id })),
            context.clone(),
        )
        .await
        .expect("read first artifact window");
    assert!(!first.output.contains("TAIL"));
    let next_offset = first.metadata["nextOffset"].as_u64().expect("next offset");
    let second = ReadArtifactTool
        .execute(
            ToolCall::new(
                "read_artifact",
                json!({ "artifactId": artifact.id, "offset": next_offset }),
            ),
            context,
        )
        .await
        .expect("read final artifact window");
    assert!(second.output.contains("TAIL"));
    assert!(second.metadata["nextOffset"].is_null());
    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn read_file_reads_one_based_utf8_line_ranges_and_preserves_crlf() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-read-lines-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let contents = "第一行\r\nsecond🙂\r\n第三行\nlast";
    fs::write(workspace_root.join("lines.txt"), contents).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::Auto,
    ));
    let context = ToolInvocationContext::local(workspace_root.clone(), policy);

    let result = ReadFileTool
        .execute(
            ToolCall::new(
                "read_file",
                json!({
                    "path": "lines.txt",
                    "window": { "mode": "lines", "startLine": 2, "endLine": 3 }
                }),
            ),
            context,
        )
        .await
        .unwrap();

    assert_eq!(result.output, "second🙂\r\n第三行\n");
    assert_eq!(result.metadata["mode"], "lines");
    assert_eq!(result.metadata["startLine"], 2);
    assert_eq!(result.metadata["endLine"], 3);
    assert_eq!(result.metadata["totalLines"], 4);
    assert_eq!(result.metadata["startOffset"], 5);
    assert!(result.metadata["nextLine"].is_null());

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn read_file_rejects_invalid_or_mixed_line_windows() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-read-lines-invalid-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::write(workspace_root.join("lines.txt"), "one\ntwo\n").unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::Auto,
    ));
    let context = ToolInvocationContext::local(workspace_root.clone(), policy);

    for (input, expected) in [
        (
            json!({ "path": "lines.txt", "offset": 0, "startLine": 1 }),
            "cannot be combined",
        ),
        (
            json!({ "path": "lines.txt", "endLine": 2 }),
            "requires startLine",
        ),
        (json!({ "path": "lines.txt", "startLine": 0 }), "at least 1"),
        (
            json!({ "path": "lines.txt", "startLine": 2, "endLine": 1 }),
            "greater than or equal",
        ),
        (
            json!({ "path": "lines.txt", "startLine": 3 }),
            "exceeds total lines",
        ),
    ] {
        let error = ReadFileTool
            .execute(ToolCall::new("read_file", input), context.clone())
            .await
            .unwrap_err();
        let error_chain = format!("{error:#}");
        assert!(
            error_chain.contains(expected),
            "unexpected error: {error:#}"
        );
    }

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn read_file_line_mode_paginates_only_at_line_boundaries() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-read-line-page-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::write(workspace_root.join("lines.txt"), "one\ntwo\nthree\n").unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::Auto,
    ));
    let context = ToolInvocationContext::local(workspace_root.clone(), policy);

    let result = execute_read_file_with_cap(
        Uuid::new_v4(),
        FileReadInput {
            path: "lines.txt".to_string(),
            window: Some(FileReadWindow::Lines {
                start_line: 1,
                end_line: Some(3),
            }),
        },
        context,
        8,
    )
    .await
    .unwrap();

    assert!(result.output.starts_with("one\ntwo\n"));
    assert!(!result.output.starts_with("one\ntwo\nt"));
    assert_eq!(result.metadata["endLine"], 2);
    assert_eq!(result.metadata["nextLine"], 3);
    assert_eq!(result.metadata["nextOffset"], 8);

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn file_observation_tools_preserve_explicit_additional_readable_roots() {
    let id = Uuid::new_v4();
    let workspace_root = std::env::temp_dir().join(format!("opentopia-tools-root-{id}"));
    let outside = std::env::temp_dir().join(format!("opentopia-tools-readable-{id}"));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("allowed.txt"), "configured marker").unwrap();
    let mut config = LocalSandboxConfig::default();
    config.read_paths = vec![outside.clone()];
    let policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
        workspace_root.clone(),
        PermissionMode::Auto,
        &config,
    ));
    let context =
        ToolInvocationContext::local_with_sandbox_config(workspace_root.clone(), policy, config);

    let listed = ListFilesTool
        .execute(
            ToolCall::new(
                "list_files",
                json!({ "path": outside.display().to_string() }),
            ),
            context.clone(),
        )
        .await
        .unwrap();
    assert!(listed.output.contains("allowed.txt"));

    let read = ReadFileTool
        .execute(
            ToolCall::new(
                "read_file",
                json!({ "path": outside.join("allowed.txt").display().to_string() }),
            ),
            context.clone(),
        )
        .await
        .unwrap();
    assert!(read.output.contains("configured marker"));

    let searched = WorkspaceSearchTool
        .execute(
            ToolCall::new(
                "workspace_search",
                json!({ "query": "configured marker", "path": outside.display().to_string() }),
            ),
            context,
        )
        .await
        .unwrap();
    assert!(searched.output.contains("configured marker"));

    fs::remove_dir_all(workspace_root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[tokio::test]
async fn write_file_preserves_explicit_additional_writable_roots() {
    let id = Uuid::new_v4();
    let workspace_root = std::env::temp_dir().join(format!("opentopia-tools-root-{id}"));
    let outside = std::env::temp_dir().join(format!("opentopia-tools-writable-{id}"));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let mut config = LocalSandboxConfig::default();
    config.writable_roots = vec![outside.clone()];
    let policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
        workspace_root.clone(),
        PermissionMode::Auto,
        &config,
    ));
    let context =
        ToolInvocationContext::local_with_sandbox_config(workspace_root.clone(), policy, config);
    let target = outside.join("dependency-cache.txt");

    WriteFileTool
        .execute(
            ToolCall::new(
                "write_file",
                json!({
                    "path": target.display().to_string(),
                    "content": "configured writable root"
                }),
            ),
            context,
        )
        .await
        .expect("configured writable root should not require approval");
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "configured writable root"
    );

    fs::remove_dir_all(workspace_root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[tokio::test]
async fn write_file_rejects_a_hash_from_a_stale_model_read() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-stale-write-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let target = workspace_root.join("shared.txt");
    fs::write(&target, "version one").unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local(workspace_root.clone(), policy);

    let read = ReadFileTool
        .execute(
            ToolCall::new("read_file", json!({ "path": "shared.txt" })),
            context.clone(),
        )
        .await
        .unwrap();
    let expected_hash = read.metadata["contentHash"].as_str().unwrap().to_string();
    fs::write(&target, "version from another conversation").unwrap();

    let error = WriteFileTool
        .execute(
            ToolCall::new(
                "write_file",
                json!({
                    "path": "shared.txt",
                    "content": "stale replacement",
                    "expectedHash": expected_hash
                }),
            ),
            context,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("precondition failed"));
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "version from another conversation"
    );
    fs::remove_dir_all(workspace_root).unwrap();
}
