use super::sqlite_codec::{parse_datetime, parse_uuid};
use chrono::{DateTime, Utc};
use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderContextStateKind {
    StoredResponse,
    CompactionItems,
    TranscriptItems,
    Hybrid,
}

impl Default for ProviderContextStateKind {
    fn default() -> Self {
        Self::StoredResponse
    }
}

impl ProviderContextStateKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StoredResponse => "stored_response",
            Self::CompactionItems => "compaction_items",
            Self::TranscriptItems => "transcript_items",
            Self::Hybrid => "hybrid",
        }
    }

    fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "stored_response" => Ok(Self::StoredResponse),
            "compaction_items" => Ok(Self::CompactionItems),
            "transcript_items" => Ok(Self::TranscriptItems),
            "hybrid" => Ok(Self::Hybrid),
            other => anyhow::bail!("unknown provider context state kind: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConversationState {
    pub thread_id: Uuid,
    pub agent_path: String,
    pub provider_id: String,
    pub model: String,
    #[serde(default)]
    pub adapter_identity: String,
    pub response_id: String,
    pub compatibility_hash: String,
    #[serde(default)]
    pub response_items: Vec<Value>,
    #[serde(default)]
    pub state_kind: ProviderContextStateKind,
    #[serde(default)]
    pub compaction_item_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<Uuid>,
    pub updated_at: DateTime<Utc>,
}

pub(super) fn save(conn: &Connection, state: &ProviderConversationState) -> anyhow::Result<()> {
    let response_items_json = serde_json::to_string(&state.response_items)?;
    conn.execute(
        r#"
        INSERT INTO provider_conversation_states (
            thread_id, agent_path, provider_id, model, adapter_identity, response_id,
            compatibility_hash, response_items_json, state_kind,
            compaction_item_count, checkpoint_id, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        ON CONFLICT(thread_id, agent_path) DO UPDATE SET
            provider_id = excluded.provider_id,
            model = excluded.model,
            adapter_identity = excluded.adapter_identity,
            response_id = excluded.response_id,
            compatibility_hash = excluded.compatibility_hash,
            response_items_json = excluded.response_items_json,
            state_kind = excluded.state_kind,
            compaction_item_count = excluded.compaction_item_count,
            checkpoint_id = excluded.checkpoint_id,
            updated_at = excluded.updated_at
        "#,
        params![
            state.thread_id.to_string(),
            &state.agent_path,
            &state.provider_id,
            &state.model,
            &state.adapter_identity,
            &state.response_id,
            &state.compatibility_hash,
            response_items_json,
            state.state_kind.as_str(),
            state.compaction_item_count as i64,
            state.checkpoint_id.map(|id| id.to_string()),
            state.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub(super) fn take(
    conn: &mut Connection,
    thread_id: Uuid,
    agent_path: &str,
) -> anyhow::Result<Option<ProviderConversationState>> {
    let transaction = conn.transaction()?;
    let state = load(&transaction, thread_id, agent_path)?;
    transaction.execute(
        "DELETE FROM provider_conversation_states WHERE thread_id = ?1 AND agent_path = ?2",
        params![thread_id.to_string(), agent_path],
    )?;
    transaction.commit()?;
    Ok(state)
}

pub(super) fn clear(conn: &Connection, thread_id: Uuid, agent_path: &str) -> anyhow::Result<bool> {
    Ok(conn.execute(
        "DELETE FROM provider_conversation_states WHERE thread_id = ?1 AND agent_path = ?2",
        params![thread_id.to_string(), agent_path],
    )? > 0)
}

pub(super) fn load(
    conn: &Connection,
    thread_id: Uuid,
    agent_path: &str,
) -> anyhow::Result<Option<ProviderConversationState>> {
    Ok(conn
        .query_row(
            r#"
            SELECT provider_id, model, adapter_identity, response_id, compatibility_hash,
                   response_items_json, state_kind, compaction_item_count,
                   checkpoint_id, updated_at
            FROM provider_conversation_states
            WHERE thread_id = ?1 AND agent_path = ?2
            "#,
            params![thread_id.to_string(), agent_path],
            |row| {
                let response_items_json = row.get::<_, String>(5)?;
                let response_items =
                    serde_json::from_str(&response_items_json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(error))
                    })?;
                let state_kind_text = row.get::<_, String>(6)?;
                let state_kind =
                    ProviderContextStateKind::from_str(&state_kind_text).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            Type::Text,
                            error.into_boxed_dyn_error(),
                        )
                    })?;
                let checkpoint_id = row
                    .get::<_, Option<String>>(8)?
                    .map(|value| parse_uuid(value, 8))
                    .transpose()?;
                Ok(ProviderConversationState {
                    thread_id,
                    agent_path: agent_path.to_string(),
                    provider_id: row.get(0)?,
                    model: row.get(1)?,
                    adapter_identity: row.get(2)?,
                    response_id: row.get(3)?,
                    compatibility_hash: row.get(4)?,
                    response_items,
                    state_kind,
                    compaction_item_count: row.get::<_, i64>(7)?.max(0) as usize,
                    checkpoint_id,
                    updated_at: parse_datetime(row.get::<_, String>(9)?, 9)?,
                })
            },
        )
        .optional()?)
}
