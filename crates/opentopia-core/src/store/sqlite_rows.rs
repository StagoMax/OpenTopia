use super::sqlite_codec::{
    parse_artifact_storage, parse_artifact_storage_metadata, parse_datetime, parse_u64, parse_uuid,
};
use crate::effect_journal::{EffectJournalRecord, EffectKind, EffectSideEffectClass, EffectStatus};
use crate::mcp::{McpServerConfig, McpToolDescriptor, ThreadMcpServer};
use crate::model::{
    AgentEvent, AgentEventPayload, Approval, ApprovalStatus, Artifact, ArtifactMetadata,
    ExperienceMode, GoalRecord, Message, MessagePart, MessageRole, Project, TerminalCommandHistory,
    TerminalCommandStatus, Thread, ThreadModelSelection, TurnChangeSet, TurnChangeSetStatus,
    TurnRecord, TurnStatus, UserInputRecord, UserInputRequest, UserInputStatus,
};
use rusqlite::types::Type;
use std::path::PathBuf;

pub(super) fn map_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<Thread> {
    let project_id: Option<String> = row.get(3)?;
    let archived_at: Option<String> = row.get(4)?;
    let experience_mode_value: String = row.get(5)?;
    let experience_mode = ExperienceMode::from_str(&experience_mode_value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    let model_selection: Option<String> = row.get(6)?;
    let model_selection = model_selection
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            serde_json::from_str::<ThreadModelSelection>(value).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    Type::Text,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        err.to_string(),
                    )),
                )
            })
        })
        .transpose()?;
    Ok(Thread {
        id: parse_uuid(row.get(0)?, 0)?,
        title: row.get(1)?,
        workspace_root: PathBuf::from(row.get::<_, String>(2)?),
        project_id: project_id.map(|value| parse_uuid(value, 3)).transpose()?,
        experience_mode,
        model_selection,
        archived_at: archived_at
            .map(|value| parse_datetime(value, 4))
            .transpose()?,
        created_at: parse_datetime(row.get(7)?, 7)?,
        updated_at: parse_datetime(row.get(8)?, 8)?,
    })
}

pub(super) fn encode_model_selection(
    selection: Option<&ThreadModelSelection>,
) -> anyhow::Result<Option<String>> {
    selection
        .map(serde_json::to_string)
        .transpose()
        .map_err(Into::into)
}

pub(super) fn map_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: parse_uuid(row.get(0)?, 0)?,
        name: row.get(1)?,
        workspace_root: row.get::<_, Option<String>>(2)?.map(PathBuf::from),
        pinned: row.get::<_, i64>(3)? != 0,
        sort_order: row.get(4)?,
        created_at: parse_datetime(row.get(5)?, 5)?,
        updated_at: parse_datetime(row.get(6)?, 6)?,
    })
}

pub(super) fn map_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    let parts_json: String = row.get(3)?;
    let parts: Vec<MessagePart> = serde_json::from_str(&parts_json)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(err)))?;
    let role_value: String = row.get(2)?;
    let role = MessageRole::from_str(&role_value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    Ok(Message {
        id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        role,
        parts,
        created_at: parse_datetime(row.get(4)?, 4)?,
    })
}

pub(super) fn map_turn(row: &rusqlite::Row<'_>) -> rusqlite::Result<TurnRecord> {
    let status_value: String = row.get(4)?;
    let status = TurnStatus::from_str(&status_value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    let completed_at: Option<String> = row.get(7)?;
    Ok(TurnRecord {
        turn_id: parse_uuid(row.get(0)?, 0)?,
        invocation_id: row.get::<_, i64>(1)?.max(1) as u64,
        thread_id: parse_uuid(row.get(2)?, 2)?,
        user_message_id: parse_uuid(row.get(3)?, 3)?,
        status,
        started_at: parse_datetime(row.get(5)?, 5)?,
        updated_at: parse_datetime(row.get(6)?, 6)?,
        completed_at: completed_at
            .map(|value| parse_datetime(value, 7))
            .transpose()?,
        error: row.get(8)?,
    })
}

pub(super) fn map_goal(row: &rusqlite::Row<'_>) -> rusqlite::Result<GoalRecord> {
    let token_budget = row
        .get::<_, Option<i64>>(3)?
        .map(u64::try_from)
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(3, Type::Integer, Box::new(error))
        })?;
    Ok(GoalRecord {
        id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        objective: row.get(2)?,
        token_budget,
        tokens_used: parse_u64(row.get(4)?, 4)?,
        time_used_seconds: parse_u64(row.get(5)?, 5)?,
        version: parse_u64(row.get(6)?, 6)?,
        created_at: parse_datetime(row.get(7)?, 7)?,
        updated_at: parse_datetime(row.get(8)?, 8)?,
    })
}

pub(super) fn map_turn_change_set(row: &rusqlite::Row<'_>) -> rusqlite::Result<TurnChangeSet> {
    let status_value: String = row.get(7)?;
    let status = TurnChangeSetStatus::from_str(&status_value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            7,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    let files_json: String = row.get(8)?;
    let additions: i64 = row.get(9)?;
    let deletions: i64 = row.get(10)?;
    let finalized_at: Option<String> = row.get(13)?;
    let reverted_at: Option<String> = row.get(14)?;
    Ok(TurnChangeSet {
        turn_id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        workspace_root: PathBuf::from(row.get::<_, String>(2)?),
        repo_root: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
        workspace_prefix: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
        before_tree: row.get(5)?,
        after_tree: row.get(6)?,
        status,
        files: serde_json::from_str(&files_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(err))
        })?,
        additions: u64::try_from(additions).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(9, Type::Integer, Box::new(err))
        })?,
        deletions: u64::try_from(deletions).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(10, Type::Integer, Box::new(err))
        })?,
        error: row.get(11)?,
        created_at: parse_datetime(row.get(12)?, 12)?,
        finalized_at: finalized_at
            .map(|value| parse_datetime(value, 13))
            .transpose()?,
        reverted_at: reverted_at
            .map(|value| parse_datetime(value, 14))
            .transpose()?,
    })
}

pub(super) fn map_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentEvent> {
    let turn_id: Option<String> = row.get(2)?;
    let payload_json: String = row.get(4)?;
    let payload: AgentEventPayload = serde_json::from_str(&payload_json)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(err)))?;
    Ok(AgentEvent {
        id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        turn_id: turn_id.map(|value| parse_uuid(value, 2)).transpose()?,
        seq: row.get(3)?,
        payload,
        created_at: parse_datetime(row.get(5)?, 5)?,
    })
}

pub(super) fn map_effect(row: &rusqlite::Row<'_>) -> rusqlite::Result<EffectJournalRecord> {
    let kind_text: String = row.get(5)?;
    let kind = EffectKind::from_str(&kind_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, Type::Text, boxed_invalid_data(error))
    })?;
    let input_json: String = row.get(8)?;
    let input = serde_json::from_str(&input_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(error))
    })?;
    let result_json: Option<String> = row.get(9)?;
    let result = result_json
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(9, Type::Text, Box::new(error))
        })?;
    let status_text: String = row.get(10)?;
    let status = EffectStatus::from_str(&status_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(10, Type::Text, boxed_invalid_data(error))
    })?;
    let side_effect_text: String = row.get(11)?;
    let side_effect_class =
        EffectSideEffectClass::from_str(&side_effect_text).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(11, Type::Text, boxed_invalid_data(error))
        })?;
    let attempt_value: i64 = row.get(13)?;
    let attempt = u32::try_from(attempt_value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(13, Type::Integer, Box::new(error))
    })?;
    let started_at: Option<String> = row.get(16)?;
    let completed_at: Option<String> = row.get(17)?;
    Ok(EffectJournalRecord {
        effect_id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        turn_id: parse_uuid(row.get(2)?, 2)?,
        agent_path: row.get(3)?,
        idempotency_key: row.get(4)?,
        kind,
        operation: row.get(6)?,
        input_hash: row.get(7)?,
        input,
        result,
        status,
        side_effect_class,
        idempotent: row.get(12)?,
        attempt,
        error: row.get(14)?,
        created_at: parse_datetime(row.get(15)?, 15)?,
        started_at: started_at
            .map(|value| parse_datetime(value, 16))
            .transpose()?,
        completed_at: completed_at
            .map(|value| parse_datetime(value, 17))
            .transpose()?,
        updated_at: parse_datetime(row.get(18)?, 18)?,
    })
}

pub(super) fn boxed_invalid_data(error: impl std::fmt::Display) -> Box<std::io::Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        error.to_string(),
    ))
}

pub(super) fn map_terminal_history(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<TerminalCommandHistory> {
    let cwd: Option<String> = row.get(5)?;
    let status_value: String = row.get(9)?;
    let status = TerminalCommandStatus::from_str(&status_value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            9,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    Ok(TerminalCommandHistory {
        command_id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        seq_start: parse_u64(row.get(2)?, 2)?,
        seq_end: parse_u64(row.get(3)?, 3)?,
        command: row.get(4)?,
        cwd: cwd.map(PathBuf::from),
        stdout: row.get(6)?,
        stderr: row.get(7)?,
        exit_code: row.get(8)?,
        status,
        message: row.get(10)?,
        started_at: parse_datetime(row.get(11)?, 11)?,
        completed_at: parse_datetime(row.get(12)?, 12)?,
    })
}

pub(super) fn map_artifact_metadata(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactMetadata> {
    let storage_kind: String = row.get(4)?;
    let path: Option<String> = row.get(5)?;
    let metadata_json: String = row.get(7)?;
    Ok(ArtifactMetadata {
        id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        kind: row.get(2)?,
        content_type: row.get(3)?,
        storage: parse_artifact_storage_metadata(&storage_kind, path, 4)?,
        bytes: parse_u64(row.get(6)?, 6)?,
        metadata: serde_json::from_str(&metadata_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(7, Type::Text, Box::new(err))
        })?,
        created_at: parse_datetime(row.get(8)?, 8)?,
    })
}

pub(super) fn map_artifact(row: &rusqlite::Row<'_>) -> rusqlite::Result<Artifact> {
    let storage_kind: String = row.get(4)?;
    let path: Option<String> = row.get(5)?;
    let inline_content: Option<String> = row.get(6)?;
    let metadata_json: String = row.get(8)?;
    Ok(Artifact {
        id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        kind: row.get(2)?,
        content_type: row.get(3)?,
        storage: parse_artifact_storage(&storage_kind, path, inline_content, 4)?,
        bytes: parse_u64(row.get(7)?, 7)?,
        metadata: serde_json::from_str(&metadata_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(err))
        })?,
        created_at: parse_datetime(row.get(9)?, 9)?,
    })
}

pub(super) fn map_approval(row: &rusqlite::Row<'_>) -> rusqlite::Result<Approval> {
    let status_value: String = row.get(4)?;
    let status = ApprovalStatus::from_str(&status_value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    let decided_at: Option<String> = row.get(6)?;
    Ok(Approval {
        approval_id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        action: row.get(2)?,
        reason: row.get(3)?,
        status,
        created_at: parse_datetime(row.get(5)?, 5)?,
        decided_at: decided_at
            .map(|value| parse_datetime(value, 6))
            .transpose()?,
    })
}

pub(super) fn map_user_input_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserInputRecord> {
    let request_id = parse_uuid(row.get(0)?, 0)?;
    let thread_id = parse_uuid(row.get(1)?, 1)?;
    let request_json: String = row.get(2)?;
    let mut request: UserInputRequest = serde_json::from_str(&request_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(error))
    })?;
    request.request_id = request_id;
    let status_value: String = row.get(3)?;
    let status = UserInputStatus::from_str(&status_value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                error.to_string(),
            )),
        )
    })?;
    let response_json: Option<String> = row.get(4)?;
    let response = response_json
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
            })
        })
        .transpose()?;
    let answered_at: Option<String> = row.get(6)?;
    Ok(UserInputRecord {
        thread_id,
        request,
        status,
        response,
        created_at: parse_datetime(row.get(5)?, 5)?,
        answered_at: answered_at
            .map(|value| parse_datetime(value, 6))
            .transpose()?,
    })
}

pub(super) fn map_mcp_server(row: &rusqlite::Row<'_>) -> rusqlite::Result<McpServerConfig> {
    let args_json: String = row.get(3)?;
    let env_keys_json: String = row.get(5)?;
    let cwd: Option<String> = row.get(4)?;
    Ok(McpServerConfig {
        server_id: parse_uuid(row.get(0)?, 0)?,
        name: row.get(1)?,
        command: row.get(2)?,
        args: serde_json::from_str(&args_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(err))
        })?,
        cwd: cwd.map(PathBuf::from),
        env_keys: serde_json::from_str(&env_keys_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(err))
        })?,
        timeout_ms: row.get::<_, i64>(6)? as u64,
        enabled: row.get::<_, i64>(7)? != 0,
        plugin_id: row.get(8)?,
        plugin_server_name: row.get(9)?,
        created_at: parse_datetime(row.get(10)?, 10)?,
        updated_at: parse_datetime(row.get(11)?, 11)?,
    })
}

pub(super) fn map_mcp_server_tool(row: &rusqlite::Row<'_>) -> rusqlite::Result<McpToolDescriptor> {
    let input_schema_json: String = row.get(4)?;
    let annotations_json: String = row.get(5)?;
    let meta_json: String = row.get(6)?;
    let permission_labels_json: String = row.get(7)?;
    Ok(McpToolDescriptor {
        server_id: parse_uuid(row.get(0)?, 0)?,
        public_name: row.get(1)?,
        tool_name: row.get(2)?,
        description: row.get(3)?,
        input_schema: serde_json::from_str(&input_schema_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(err))
        })?,
        annotations: serde_json::from_str(&annotations_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(err))
        })?,
        meta: serde_json::from_str(&meta_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(err))
        })?,
        permission_labels: serde_json::from_str(&permission_labels_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(7, Type::Text, Box::new(err))
        })?,
    })
}

pub(super) fn map_thread_mcp_server(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadMcpServer> {
    Ok(ThreadMcpServer {
        thread_id: parse_uuid(row.get(0)?, 0)?,
        server_id: parse_uuid(row.get(1)?, 1)?,
        enabled: row.get::<_, i64>(2)? != 0,
        updated_at: parse_datetime(row.get(3)?, 3)?,
    })
}
