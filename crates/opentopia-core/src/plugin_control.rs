use crate::capabilities::{ContributionKind, PluginPermissionKind};
use crate::plugins::PluginDescriptor;
use crate::store::{normalize_workspace_key, SqliteSessionStore};
use anyhow::{bail, Context};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MAX_PLUGIN_CONTROL_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginControlScopeType {
    Global,
    Workspace,
    Thread,
}

impl PluginControlScopeType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Workspace => "workspace",
            Self::Thread => "thread",
        }
    }

    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "global" => Ok(Self::Global),
            "workspace" => Ok(Self::Workspace),
            "thread" => Ok(Self::Thread),
            other => bail!("unknown plugin scope type: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginControlScope {
    pub scope_type: PluginControlScopeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
}

impl PluginControlScope {
    pub fn global() -> Self {
        Self {
            scope_type: PluginControlScopeType::Global,
            scope_id: None,
        }
    }

    pub fn workspace(path: &Path) -> anyhow::Result<Self> {
        let scope_id = normalize_workspace_key(path);
        if scope_id.is_empty() {
            bail!("workspace scope cannot be empty");
        }
        Ok(Self {
            scope_type: PluginControlScopeType::Workspace,
            scope_id: Some(scope_id),
        })
    }

    pub fn thread(thread_id: Uuid) -> Self {
        Self {
            scope_type: PluginControlScopeType::Thread,
            scope_id: Some(thread_id.to_string()),
        }
    }

    pub fn normalized(&self) -> anyhow::Result<Self> {
        let scope_id = match self.scope_type {
            PluginControlScopeType::Global => {
                if self
                    .scope_id
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                {
                    bail!("global plugin scope must not have a scopeId");
                }
                None
            }
            PluginControlScopeType::Workspace => {
                let value = self
                    .scope_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .context("workspace plugin scope requires scopeId")?;
                let normalized = normalize_workspace_key(Path::new(value));
                if normalized.is_empty() {
                    bail!("workspace plugin scope cannot be empty");
                }
                Some(normalized)
            }
            PluginControlScopeType::Thread => {
                let value = self
                    .scope_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .context("thread plugin scope requires scopeId")?;
                Some(
                    Uuid::parse_str(value)
                        .context("thread plugin scopeId must be a UUID")?
                        .to_string(),
                )
            }
        };
        Ok(Self {
            scope_type: self.scope_type,
            scope_id,
        })
    }

    fn database_id(&self) -> anyhow::Result<String> {
        Ok(self.normalized()?.scope_id.unwrap_or_default())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginActivationRecord {
    pub plugin_id: String,
    pub scope: PluginControlScope,
    pub enabled: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingsRecord {
    pub plugin_id: String,
    pub scope: PluginControlScope,
    pub settings: Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginSecretBindingRecord {
    pub plugin_id: String,
    pub scope: PluginControlScope,
    pub setting_key: String,
    pub binding_id: String,
    #[serde(default)]
    pub metadata: Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginPermissionGrantStatus {
    Granted,
    Revoked,
}

impl PluginPermissionGrantStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Revoked => "revoked",
        }
    }

    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "granted" => Ok(Self::Granted),
            "revoked" => Ok(Self::Revoked),
            other => bail!("unknown plugin permission grant status: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginPermissionGrantRecord {
    pub plugin_id: String,
    pub scope: PluginControlScope,
    pub permission: String,
    #[serde(default)]
    pub constraint: Value,
    pub status: PluginPermissionGrantStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginContributionRecord {
    pub plugin_id: String,
    pub contribution_id: String,
    pub kind: String,
    pub local_id: String,
    #[serde(default)]
    pub descriptor: Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeHealthStatus {
    Unknown,
    Ready,
    Degraded,
    Error,
    Stopped,
}

impl PluginRuntimeHealthStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Error => "error",
            Self::Stopped => "stopped",
        }
    }

    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "unknown" => Ok(Self::Unknown),
            "ready" => Ok(Self::Ready),
            "degraded" => Ok(Self::Degraded),
            "error" => Ok(Self::Error),
            "stopped" => Ok(Self::Stopped),
            other => bail!("unknown plugin runtime health status: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginRuntimeHealthRecord {
    pub plugin_id: String,
    pub contribution_id: String,
    pub status: PluginRuntimeHealthStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub last_checked_at: DateTime<Utc>,
    pub restart_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginPermissionRequest {
    pub category: String,
    pub value: String,
    pub permission: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginControlManifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(default)]
    pub host_capabilities: Vec<String>,
    #[serde(default)]
    pub permission_requests: Vec<PluginPermissionRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_schema: Option<Value>,
    #[serde(default)]
    pub secret_setting_keys: Vec<String>,
    #[serde(default)]
    pub required_secret_setting_keys: Vec<String>,
    #[serde(default)]
    pub contributions: Vec<PluginContributionRecord>,
}

pub(crate) fn migrate_plugin_control(conn: &mut Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS plugin_activations (
            plugin_id TEXT NOT NULL,
            scope_type TEXT NOT NULL CHECK(scope_type IN ('global', 'workspace', 'thread')),
            scope_id TEXT NOT NULL DEFAULT '',
            enabled INTEGER NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(plugin_id, scope_type, scope_id)
        );
        CREATE TABLE IF NOT EXISTS plugin_settings (
            plugin_id TEXT NOT NULL,
            scope_type TEXT NOT NULL CHECK(scope_type IN ('global', 'workspace', 'thread')),
            scope_id TEXT NOT NULL DEFAULT '',
            settings_json TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(plugin_id, scope_type, scope_id)
        );
        CREATE TABLE IF NOT EXISTS plugin_secret_bindings (
            plugin_id TEXT NOT NULL,
            scope_type TEXT NOT NULL CHECK(scope_type IN ('global', 'workspace', 'thread')),
            scope_id TEXT NOT NULL DEFAULT '',
            setting_key TEXT NOT NULL,
            binding_id TEXT NOT NULL,
            metadata_json TEXT NOT NULL DEFAULT '{}',
            updated_at TEXT NOT NULL,
            PRIMARY KEY(plugin_id, scope_type, scope_id, setting_key)
        );
        CREATE TABLE IF NOT EXISTS plugin_permission_grants (
            plugin_id TEXT NOT NULL,
            scope_type TEXT NOT NULL CHECK(scope_type IN ('global', 'workspace', 'thread')),
            scope_id TEXT NOT NULL DEFAULT '',
            permission TEXT NOT NULL,
            constraint_json TEXT NOT NULL DEFAULT '{}',
            status TEXT NOT NULL CHECK(status IN ('granted', 'revoked')),
            granted_at TEXT,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(plugin_id, scope_type, scope_id, permission)
        );
        CREATE TABLE IF NOT EXISTS plugin_contributions (
            plugin_id TEXT NOT NULL,
            contribution_id TEXT NOT NULL PRIMARY KEY,
            kind TEXT NOT NULL,
            local_id TEXT NOT NULL,
            descriptor_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS plugin_runtime_health (
            contribution_id TEXT NOT NULL PRIMARY KEY,
            plugin_id TEXT NOT NULL,
            status TEXT NOT NULL CHECK(status IN ('unknown', 'ready', 'degraded', 'error', 'stopped')),
            last_error TEXT,
            last_checked_at TEXT NOT NULL,
            restart_count INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_plugin_activations_scope
            ON plugin_activations(scope_type, scope_id, plugin_id);
        CREATE INDEX IF NOT EXISTS idx_plugin_permissions_scope
            ON plugin_permission_grants(scope_type, scope_id, plugin_id);
        CREATE INDEX IF NOT EXISTS idx_plugin_contributions_plugin
            ON plugin_contributions(plugin_id, kind, contribution_id);
        CREATE INDEX IF NOT EXISTS idx_plugin_health_plugin
            ON plugin_runtime_health(plugin_id, contribution_id);
        "#,
    )?;
    Ok(())
}

impl SqliteSessionStore {
    pub fn set_plugin_activation(
        &self,
        plugin_id: &str,
        scope: &PluginControlScope,
        enabled: bool,
    ) -> anyhow::Result<PluginActivationRecord> {
        let scope = scope.normalized()?;
        let scope_id = scope.database_id()?;
        let updated_at = Utc::now();
        self.with_connection(|conn| {
            conn.execute(
                r#"INSERT INTO plugin_activations
                   (plugin_id, scope_type, scope_id, enabled, updated_at)
                   VALUES (?1, ?2, ?3, ?4, ?5)
                   ON CONFLICT(plugin_id, scope_type, scope_id) DO UPDATE SET
                     enabled = excluded.enabled, updated_at = excluded.updated_at"#,
                params![
                    plugin_id,
                    scope.scope_type.as_str(),
                    scope_id,
                    enabled,
                    updated_at.to_rfc3339()
                ],
            )?;
            Ok(())
        })?;
        Ok(PluginActivationRecord {
            plugin_id: plugin_id.to_string(),
            scope,
            enabled,
            updated_at,
        })
    }

    pub fn list_plugin_activations(
        &self,
        plugin_id: &str,
    ) -> anyhow::Result<Vec<PluginActivationRecord>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT scope_type, scope_id, enabled, updated_at FROM plugin_activations WHERE plugin_id = ?1 ORDER BY scope_type, scope_id",
            )?;
            let rows = stmt.query_map(params![plugin_id], |row| {
                let scope_type: String = row.get(0)?;
                let scope_id: String = row.get(1)?;
                let updated_at: String = row.get(3)?;
                Ok((scope_type, scope_id, row.get::<_, bool>(2)?, updated_at))
            })?;
            rows.map(|row| {
                let (scope_type, scope_id, enabled, updated_at) = row?;
                Ok(PluginActivationRecord {
                    plugin_id: plugin_id.to_string(),
                    scope: scope_from_database(&scope_type, &scope_id)?,
                    enabled,
                    updated_at: parse_datetime(&updated_at)?,
                })
            })
            .collect()
        })
    }

    pub fn plugin_effectively_enabled(
        &self,
        plugin_id: &str,
        default_enabled: bool,
        workspace_root: Option<&Path>,
        thread_id: Option<Uuid>,
    ) -> anyhow::Result<bool> {
        let records = self.list_plugin_activations(plugin_id)?;
        let global = PluginControlScope::global();
        let workspace = workspace_root
            .map(PluginControlScope::workspace)
            .transpose()?;
        let thread = thread_id.map(PluginControlScope::thread);
        let value_for = |target: &PluginControlScope| {
            records
                .iter()
                .find(|record| record.scope == *target)
                .map(|record| record.enabled)
        };
        Ok(value_for(&global).unwrap_or(default_enabled)
            && workspace.as_ref().and_then(value_for).unwrap_or(true)
            && thread.as_ref().and_then(value_for).unwrap_or(true))
    }

    pub fn get_plugin_settings(
        &self,
        plugin_id: &str,
        scope: &PluginControlScope,
    ) -> anyhow::Result<Option<PluginSettingsRecord>> {
        let scope = scope.normalized()?;
        let scope_id = scope.database_id()?;
        self.with_connection(|conn| {
            conn.query_row(
                "SELECT settings_json, updated_at FROM plugin_settings WHERE plugin_id = ?1 AND scope_type = ?2 AND scope_id = ?3",
                params![plugin_id, scope.scope_type.as_str(), scope_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .map(|(settings, updated_at)| {
                Ok(PluginSettingsRecord {
                    plugin_id: plugin_id.to_string(),
                    scope: scope.clone(),
                    settings: serde_json::from_str(&settings)?,
                    updated_at: parse_datetime(&updated_at)?,
                })
            })
            .transpose()
        })
    }

    pub fn put_plugin_settings(
        &self,
        plugin_id: &str,
        scope: &PluginControlScope,
        settings: &Value,
    ) -> anyhow::Result<PluginSettingsRecord> {
        if !settings.is_object() {
            bail!("plugin settings must be a JSON object");
        }
        let scope = scope.normalized()?;
        let scope_id = scope.database_id()?;
        let updated_at = Utc::now();
        self.with_connection(|conn| {
            conn.execute(
                r#"INSERT INTO plugin_settings
                   (plugin_id, scope_type, scope_id, settings_json, updated_at)
                   VALUES (?1, ?2, ?3, ?4, ?5)
                   ON CONFLICT(plugin_id, scope_type, scope_id) DO UPDATE SET
                     settings_json = excluded.settings_json, updated_at = excluded.updated_at"#,
                params![
                    plugin_id,
                    scope.scope_type.as_str(),
                    scope_id,
                    serde_json::to_string(settings)?,
                    updated_at.to_rfc3339()
                ],
            )?;
            Ok(())
        })?;
        Ok(PluginSettingsRecord {
            plugin_id: plugin_id.to_string(),
            scope,
            settings: settings.clone(),
            updated_at,
        })
    }

    pub fn list_plugin_secret_bindings(
        &self,
        plugin_id: &str,
        scope: &PluginControlScope,
    ) -> anyhow::Result<Vec<PluginSecretBindingRecord>> {
        let scope = scope.normalized()?;
        let scope_id = scope.database_id()?;
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT setting_key, binding_id, metadata_json, updated_at FROM plugin_secret_bindings WHERE plugin_id = ?1 AND scope_type = ?2 AND scope_id = ?3 ORDER BY setting_key",
            )?;
            let rows = stmt.query_map(params![plugin_id, scope.scope_type.as_str(), scope_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
            })?;
            rows.map(|row| {
                let (setting_key, binding_id, metadata, updated_at) = row?;
                Ok(PluginSecretBindingRecord {
                    plugin_id: plugin_id.to_string(),
                    scope: scope.clone(),
                    setting_key,
                    binding_id,
                    metadata: serde_json::from_str(&metadata)?,
                    updated_at: parse_datetime(&updated_at)?,
                })
            }).collect()
        })
    }

    pub fn put_plugin_secret_binding(
        &self,
        plugin_id: &str,
        scope: &PluginControlScope,
        setting_key: &str,
        binding_id: &str,
        metadata: &Value,
    ) -> anyhow::Result<PluginSecretBindingRecord> {
        let setting_key = non_empty(setting_key, "secret setting key")?;
        let binding_id = non_empty(binding_id, "secret binding ID")?;
        let scope = scope.normalized()?;
        let scope_id = scope.database_id()?;
        let updated_at = Utc::now();
        self.with_connection(|conn| {
            conn.execute(
                r#"INSERT INTO plugin_secret_bindings
                   (plugin_id, scope_type, scope_id, setting_key, binding_id, metadata_json, updated_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                   ON CONFLICT(plugin_id, scope_type, scope_id, setting_key) DO UPDATE SET
                     binding_id = excluded.binding_id, metadata_json = excluded.metadata_json,
                     updated_at = excluded.updated_at"#,
                params![plugin_id, scope.scope_type.as_str(), scope_id, setting_key, binding_id, serde_json::to_string(metadata)?, updated_at.to_rfc3339()],
            )?;
            Ok(())
        })?;
        Ok(PluginSecretBindingRecord {
            plugin_id: plugin_id.to_string(),
            scope,
            setting_key,
            binding_id,
            metadata: metadata.clone(),
            updated_at,
        })
    }

    pub fn delete_plugin_secret_binding(
        &self,
        plugin_id: &str,
        scope: &PluginControlScope,
        setting_key: &str,
    ) -> anyhow::Result<bool> {
        let scope = scope.normalized()?;
        let scope_id = scope.database_id()?;
        self.with_connection(|conn| {
            Ok(conn.execute(
                "DELETE FROM plugin_secret_bindings WHERE plugin_id = ?1 AND scope_type = ?2 AND scope_id = ?3 AND setting_key = ?4",
                params![plugin_id, scope.scope_type.as_str(), scope_id, setting_key],
            )? > 0)
        })
    }

    pub fn set_manifest_plugin_permission_grant(
        &self,
        plugin_id: &str,
        manifest: &PluginControlManifest,
        scope: &PluginControlScope,
        permission: &str,
        constraint: &Value,
        status: PluginPermissionGrantStatus,
    ) -> anyhow::Result<PluginPermissionGrantRecord> {
        if !permission_requested(manifest, permission) {
            bail!("plugin manifest does not request permission `{permission}`");
        }
        self.set_plugin_permission_grant(plugin_id, scope, permission, constraint, status)
    }

    fn set_plugin_permission_grant(
        &self,
        plugin_id: &str,
        scope: &PluginControlScope,
        permission: &str,
        constraint: &Value,
        status: PluginPermissionGrantStatus,
    ) -> anyhow::Result<PluginPermissionGrantRecord> {
        let permission = non_empty(permission, "plugin permission")?;
        let scope = scope.normalized()?;
        let scope_id = scope.database_id()?;
        let updated_at = Utc::now();
        let granted_at = (status == PluginPermissionGrantStatus::Granted).then_some(updated_at);
        self.with_connection(|conn| {
            conn.execute(
                r#"INSERT INTO plugin_permission_grants
                   (plugin_id, scope_type, scope_id, permission, constraint_json, status, granted_at, updated_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                   ON CONFLICT(plugin_id, scope_type, scope_id, permission) DO UPDATE SET
                     constraint_json = excluded.constraint_json, status = excluded.status,
                     granted_at = CASE WHEN excluded.status = 'granted' THEN excluded.granted_at ELSE plugin_permission_grants.granted_at END,
                     updated_at = excluded.updated_at"#,
                params![plugin_id, scope.scope_type.as_str(), scope_id, permission, serde_json::to_string(constraint)?, status.as_str(), granted_at.map(|value| value.to_rfc3339()), updated_at.to_rfc3339()],
            )?;
            Ok(())
        })?;
        Ok(PluginPermissionGrantRecord {
            plugin_id: plugin_id.to_string(),
            scope,
            permission,
            constraint: constraint.clone(),
            status,
            granted_at,
            updated_at,
        })
    }

    pub fn list_plugin_permission_grants(
        &self,
        plugin_id: &str,
    ) -> anyhow::Result<Vec<PluginPermissionGrantRecord>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT scope_type, scope_id, permission, constraint_json, status, granted_at, updated_at FROM plugin_permission_grants WHERE plugin_id = ?1 ORDER BY scope_type, scope_id, permission",
            )?;
            let rows = stmt.query_map(params![plugin_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, String>(6)?))
            })?;
            rows.map(|row| {
                let (scope_type, scope_id, permission, constraint, status, granted_at, updated_at) = row?;
                Ok(PluginPermissionGrantRecord {
                    plugin_id: plugin_id.to_string(),
                    scope: scope_from_database(&scope_type, &scope_id)?,
                    permission,
                    constraint: serde_json::from_str(&constraint)?,
                    status: PluginPermissionGrantStatus::parse(&status)?,
                    granted_at: granted_at.as_deref().map(parse_datetime).transpose()?,
                    updated_at: parse_datetime(&updated_at)?,
                })
            }).collect()
        })
    }

    pub fn replace_plugin_contributions(
        &self,
        plugin_id: &str,
        contributions: &[PluginContributionRecord],
    ) -> anyhow::Result<()> {
        self.with_connection(|conn| {
            let transaction = conn.transaction()?;
            transaction.execute("DELETE FROM plugin_contributions WHERE plugin_id = ?1", params![plugin_id])?;
            for contribution in contributions {
                if contribution.plugin_id != plugin_id {
                    bail!("plugin contribution belongs to a different plugin");
                }
                transaction.execute(
                    "INSERT INTO plugin_contributions (plugin_id, contribution_id, kind, local_id, descriptor_json, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![plugin_id, contribution.contribution_id, contribution.kind, contribution.local_id, serde_json::to_string(&contribution.descriptor)?, contribution.updated_at.to_rfc3339()],
                )?;
                transaction.execute(
                    "INSERT OR IGNORE INTO plugin_runtime_health (contribution_id, plugin_id, status, last_checked_at, restart_count) VALUES (?1, ?2, 'unknown', ?3, 0)",
                    params![contribution.contribution_id, plugin_id, contribution.updated_at.to_rfc3339()],
                )?;
            }
            transaction.execute(
                "DELETE FROM plugin_runtime_health WHERE plugin_id = ?1 AND contribution_id NOT IN (SELECT contribution_id FROM plugin_contributions WHERE plugin_id = ?1)",
                params![plugin_id],
            )?;
            transaction.commit()?;
            Ok(())
        })
    }

    pub fn list_plugin_contributions(
        &self,
        plugin_id: &str,
    ) -> anyhow::Result<Vec<PluginContributionRecord>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT contribution_id, kind, local_id, descriptor_json, updated_at FROM plugin_contributions WHERE plugin_id = ?1 ORDER BY kind, contribution_id",
            )?;
            let rows = stmt.query_map(params![plugin_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?))
            })?;
            rows.map(|row| {
                let (contribution_id, kind, local_id, descriptor, updated_at) = row?;
                Ok(PluginContributionRecord {
                    plugin_id: plugin_id.to_string(),
                    contribution_id,
                    kind,
                    local_id,
                    descriptor: serde_json::from_str(&descriptor)?,
                    updated_at: parse_datetime(&updated_at)?,
                })
            }).collect()
        })
    }

    pub fn put_plugin_runtime_health(
        &self,
        record: &PluginRuntimeHealthRecord,
    ) -> anyhow::Result<()> {
        self.with_connection(|conn| {
            conn.execute(
                r#"INSERT INTO plugin_runtime_health
                   (contribution_id, plugin_id, status, last_error, last_checked_at, restart_count)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                   ON CONFLICT(contribution_id) DO UPDATE SET plugin_id = excluded.plugin_id,
                     status = excluded.status, last_error = excluded.last_error,
                     last_checked_at = excluded.last_checked_at, restart_count = excluded.restart_count"#,
                params![record.contribution_id, record.plugin_id, record.status.as_str(), record.last_error, record.last_checked_at.to_rfc3339(), record.restart_count],
            )?;
            Ok(())
        })
    }

    pub fn list_plugin_runtime_health(
        &self,
        plugin_id: &str,
    ) -> anyhow::Result<Vec<PluginRuntimeHealthRecord>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT contribution_id, status, last_error, last_checked_at, restart_count FROM plugin_runtime_health WHERE plugin_id = ?1 ORDER BY contribution_id",
            )?;
            let rows = stmt.query_map(params![plugin_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?, row.get::<_, u64>(4)?))
            })?;
            rows.map(|row| {
                let (contribution_id, status, last_error, last_checked_at, restart_count) = row?;
                Ok(PluginRuntimeHealthRecord {
                    plugin_id: plugin_id.to_string(),
                    contribution_id,
                    status: PluginRuntimeHealthStatus::parse(&status)?,
                    last_error,
                    last_checked_at: parse_datetime(&last_checked_at)?,
                    restart_count,
                })
            }).collect()
        })
    }
}

pub fn inspect_plugin_control_manifest(
    plugin: &PluginDescriptor,
) -> anyhow::Result<PluginControlManifest> {
    let capability_manifest = &plugin.capability_manifest;
    let api_version = capability_manifest.api_version.clone();
    let host_capabilities = capability_manifest.required_host_capabilities.clone();
    let mut permission_requests = capability_manifest
        .permissions
        .requirements()
        .into_iter()
        .map(|permission| {
            let category = match permission.kind {
                PluginPermissionKind::Filesystem => "filesystem",
                PluginPermissionKind::Network => "network",
                PluginPermissionKind::Secret => "secrets",
                PluginPermissionKind::Desktop => "desktop",
            };
            PluginPermissionRequest {
                category: category.to_string(),
                permission: format!("{category}:{}", permission.value),
                value: permission.value,
            }
        })
        .collect::<Vec<_>>();
    permission_requests.sort_by(|left, right| left.permission.cmp(&right.permission));

    let configuration_schema = capability_manifest
        .configuration_schema
        .as_deref()
        .map(|path| resolve_plugin_file(&plugin.path, path))
        .transpose()?
        .map(|path| read_json_file(&path))
        .transpose()?;
    let secret_setting_keys = configuration_schema
        .as_ref()
        .map(secret_setting_keys)
        .unwrap_or_default();
    let required_secret_setting_keys = configuration_schema
        .as_ref()
        .map(required_secret_setting_keys)
        .unwrap_or_default();
    let now = Utc::now();
    let contributions = capability_manifest
        .contributions
        .iter()
        .map(|contribution| PluginContributionRecord {
            plugin_id: plugin.id.clone(),
            contribution_id: contribution.id.clone(),
            kind: contribution_kind_name(contribution.kind).to_string(),
            local_id: contribution.local_id.clone(),
            descriptor: serde_json::to_value(contribution).unwrap_or_else(|_| Value::Null),
            updated_at: now,
        })
        .collect();
    Ok(PluginControlManifest {
        api_version,
        host_capabilities,
        permission_requests,
        configuration_schema,
        secret_setting_keys,
        required_secret_setting_keys,
        contributions,
    })
}

pub fn validate_plugin_settings(schema: Option<&Value>, settings: &Value) -> anyhow::Result<()> {
    if !settings.is_object() {
        bail!("plugin settings must be a JSON object");
    }
    if let Some(schema) = schema {
        validate_schema_value(schema, settings, "settings")?;
        let secrets = secret_setting_keys(schema)
            .into_iter()
            .collect::<HashSet<_>>();
        for key in settings.as_object().into_iter().flat_map(Map::keys) {
            if secrets.contains(key) {
                bail!("secret setting `{key}` must use an opaque secret binding ID");
            }
        }
    } else if settings
        .as_object()
        .is_some_and(|settings| !settings.is_empty())
    {
        bail!("plugin does not declare a configuration schema");
    }
    Ok(())
}

pub fn permission_requested(manifest: &PluginControlManifest, permission: &str) -> bool {
    manifest
        .permission_requests
        .iter()
        .any(|request| request.permission == permission)
}

fn read_json_file(path: &Path) -> anyhow::Result<Value> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to inspect {}", path.display()))?;
    if metadata.len() > MAX_PLUGIN_CONTROL_FILE_BYTES {
        bail!("plugin control file is too large: {}", path.display());
    }
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("invalid JSON in {}", path.display()))
}

fn resolve_plugin_file(root: &Path, declared: &str) -> anyhow::Result<PathBuf> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", root.display()))?;
    let path = root
        .join(declared.trim_start_matches("./"))
        .canonicalize()
        .with_context(|| format!("failed to resolve plugin path `{declared}`"))?;
    if !path.starts_with(&root) || !path.is_file() {
        bail!("plugin path escapes its package root: {declared}");
    }
    Ok(path)
}

fn secret_setting_keys(schema: &Value) -> Vec<String> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(Map::iter)
        .filter(|(_, property)| {
            property.get("writeOnly").and_then(Value::as_bool) == Some(true)
                || property.get("secret").and_then(Value::as_bool) == Some(true)
                || property.get("opentopiaSecret").and_then(Value::as_bool) == Some(true)
                || matches!(
                    property.get("format").and_then(Value::as_str),
                    Some("password" | "secret")
                )
        })
        .map(|(key, _)| key.clone())
        .collect()
}

fn required_secret_setting_keys(schema: &Value) -> Vec<String> {
    let secrets = secret_setting_keys(schema)
        .into_iter()
        .collect::<HashSet<_>>();
    schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|key| secrets.contains(*key))
        .map(str::to_string)
        .collect()
}

fn validate_schema_value(schema: &Value, value: &Value, path: &str) -> anyhow::Result<()> {
    for keyword in [
        "$ref",
        "allOf",
        "anyOf",
        "oneOf",
        "not",
        "if",
        "then",
        "else",
        "pattern",
        "patternProperties",
        "dependentRequired",
        "unevaluatedProperties",
    ] {
        if schema.get(keyword).is_some() {
            bail!("{path} uses unsupported schema keyword `{keyword}`");
        }
    }
    if let Some(constant) = schema.get("const") {
        if value != constant {
            bail!("{path} does not match the schema const value");
        }
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        if !values.contains(value) {
            bail!("{path} is not one of the allowed values");
        }
    }
    if let Some(types) = schema.get("type") {
        let valid = match types {
            Value::String(kind) => value_matches_type(value, kind),
            Value::Array(kinds) => kinds
                .iter()
                .filter_map(Value::as_str)
                .any(|kind| value_matches_type(value, kind)),
            _ => false,
        };
        if !valid {
            bail!("{path} has the wrong JSON type");
        }
    }
    if let Some(object) = value.as_object() {
        let properties = schema.get("properties").and_then(Value::as_object);
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            let secrets = secret_setting_keys(schema)
                .into_iter()
                .collect::<HashSet<_>>();
            for key in required.iter().filter_map(Value::as_str) {
                if !secrets.contains(key) && !object.contains_key(key) {
                    bail!("{path}.{key} is required");
                }
            }
        }
        for (key, item) in object {
            match properties.and_then(|properties| properties.get(key)) {
                Some(property) => validate_schema_value(property, item, &format!("{path}.{key}"))?,
                None if schema.get("additionalProperties").and_then(Value::as_bool)
                    == Some(false) =>
                {
                    bail!("{path}.{key} is not declared by the plugin schema")
                }
                None => {
                    if let Some(additional) = schema
                        .get("additionalProperties")
                        .filter(|value| value.is_object())
                    {
                        validate_schema_value(additional, item, &format!("{path}.{key}"))?;
                    }
                }
            }
        }
    }
    if let Some(array) = value.as_array() {
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64) {
            if array.len() < min as usize {
                bail!("{path} contains fewer than {min} items");
            }
        }
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64) {
            if array.len() > max as usize {
                bail!("{path} contains more than {max} items");
            }
        }
        if schema.get("uniqueItems").and_then(Value::as_bool) == Some(true) {
            for (index, item) in array.iter().enumerate() {
                if array[..index].contains(item) {
                    bail!("{path} contains duplicate items");
                }
            }
        }
        if let Some(items) = schema.get("items") {
            for (index, item) in array.iter().enumerate() {
                validate_schema_value(items, item, &format!("{path}[{index}]"))?;
            }
        }
    }
    if let Some(string) = value.as_str() {
        if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
            if string.chars().count() < min as usize {
                bail!("{path} is shorter than {min} characters");
            }
        }
        if let Some(max) = schema.get("maxLength").and_then(Value::as_u64) {
            if string.chars().count() > max as usize {
                bail!("{path} is longer than {max} characters");
            }
        }
    }
    if let Some(number) = value.as_f64() {
        if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64) {
            if number < minimum {
                bail!("{path} is less than the minimum {minimum}");
            }
        }
        if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64) {
            if number > maximum {
                bail!("{path} is greater than the maximum {maximum}");
            }
        }
    }
    Ok(())
}

fn contribution_kind_name(kind: ContributionKind) -> &'static str {
    match kind {
        ContributionKind::Skill => "skills",
        ContributionKind::McpServer => "mcp_servers",
        ContributionKind::NativeTool => "native_tools",
        ContributionKind::Previewer => "previewers",
        ContributionKind::ContextLoader => "context_loaders",
        ContributionKind::AgentProfile => "agent_profiles",
        ContributionKind::ScmConnector => "scm_connectors",
        ContributionKind::App => "apps",
    }
}

fn value_matches_type(value: &Value, kind: &str) -> bool {
    match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "string" => value.is_string(),
        _ => false,
    }
}

fn scope_from_database(scope_type: &str, scope_id: &str) -> anyhow::Result<PluginControlScope> {
    let scope_type = PluginControlScopeType::parse(scope_type)?;
    PluginControlScope {
        scope_type,
        scope_id: (scope_type != PluginControlScopeType::Global).then(|| scope_id.to_string()),
    }
    .normalized()
}

fn parse_datetime(value: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

fn non_empty(value: &str, name: &str) -> anyhow::Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{name} cannot be empty");
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_scopes_are_persistent_and_monotonic() {
        let store = SqliteSessionStore::open(":memory:").unwrap();
        let workspace = Path::new("C:/work/demo");
        let thread_id = Uuid::new_v4();
        store
            .set_plugin_activation("plugin", &PluginControlScope::global(), false)
            .unwrap();
        store
            .set_plugin_activation(
                "plugin",
                &PluginControlScope::workspace(workspace).unwrap(),
                true,
            )
            .unwrap();
        store
            .set_plugin_activation("plugin", &PluginControlScope::thread(thread_id), true)
            .unwrap();
        assert!(!store
            .plugin_effectively_enabled("plugin", true, Some(workspace), Some(thread_id))
            .unwrap());
        assert_eq!(store.list_plugin_activations("plugin").unwrap().len(), 3);
    }

    #[test]
    fn explicit_global_activation_can_enable_a_default_disabled_plugin() {
        let store = SqliteSessionStore::open(":memory:").unwrap();
        store
            .set_plugin_activation("plugin", &PluginControlScope::global(), true)
            .unwrap();
        assert!(store
            .plugin_effectively_enabled("plugin", false, None, None)
            .unwrap());
    }

    #[test]
    fn settings_validation_rejects_unknown_and_secret_values() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["token"],
            "properties": {
                "mode": { "type": "string", "enum": ["fast", "safe"] },
                "token": { "type": "string", "writeOnly": true }
            },
            "additionalProperties": false
        });
        validate_plugin_settings(Some(&schema), &serde_json::json!({ "mode": "safe" })).unwrap();
        assert!(
            validate_plugin_settings(Some(&schema), &serde_json::json!({ "extra": true })).is_err()
        );
        assert!(validate_plugin_settings(
            Some(&schema),
            &serde_json::json!({ "token": "plaintext" })
        )
        .is_err());
        assert_eq!(required_secret_setting_keys(&schema), vec!["token"]);
        assert!(validate_plugin_settings(
            Some(&serde_json::json!({
                "type": "object",
                "properties": { "name": { "type": "string", "pattern": "^[a-z]+$" } }
            })),
            &serde_json::json!({ "name": "valid" })
        )
        .is_err());
    }

    #[test]
    fn permission_and_runtime_records_round_trip() {
        let store = SqliteSessionStore::open(":memory:").unwrap();
        let scope = PluginControlScope::global();
        store
            .set_plugin_permission_grant(
                "plugin",
                &scope,
                "network:api.example.com",
                &serde_json::json!({ "ports": [443] }),
                PluginPermissionGrantStatus::Granted,
            )
            .unwrap();
        assert_eq!(
            store.list_plugin_permission_grants("plugin").unwrap().len(),
            1
        );

        let contribution = PluginContributionRecord {
            plugin_id: "plugin".to_string(),
            contribution_id: "plugin/tool".to_string(),
            kind: "nativeTools".to_string(),
            local_id: "tool".to_string(),
            descriptor: serde_json::json!({ "id": "tool" }),
            updated_at: Utc::now(),
        };
        store
            .replace_plugin_contributions("plugin", &[contribution])
            .unwrap();
        let mut health = store
            .list_plugin_runtime_health("plugin")
            .unwrap()
            .remove(0);
        assert_eq!(health.status, PluginRuntimeHealthStatus::Unknown);
        health.status = PluginRuntimeHealthStatus::Ready;
        health.restart_count = 2;
        store.put_plugin_runtime_health(&health).unwrap();
        assert_eq!(
            store.list_plugin_runtime_health("plugin").unwrap()[0].restart_count,
            2
        );
    }
}
