use super::*;
use crate::effect_journal::{EffectKind, EffectSideEffectClass};
use crate::enterprise::{
    AgentBudgetV1, AgentInstanceV1, AgentModelBindingV1, AgentModelPolicyV1, AgentRiskClassV1,
    AgentTemplateError, AgentTemplateSpecV1, CapabilityProjection, ExperienceSurfaceProfile,
};
use crate::mcp::{McpServerConfig, McpToolDescriptor};
use crate::model::{
    MessageRole, TerminalCommandStatus, ToolResult, TurnChangeSetStatus, TurnFileChange,
    TurnFileChangeKind,
};
use crate::settings::AppSettings;
use crate::store_migrations::{
    self, CURRENT_DATABASE_SCHEMA_VERSION, LEGACY_DATABASE_SCHEMA_VERSION,
};
use crate::work_form::{WorkFormStatus, WorkItemStatus};
use legacy_schema::turns_table_supports_waiting_boundaries;
use project_repository::table_has_column;
use std::collections::{BTreeSet, HashMap};

include!("tests/setup_and_plugins.rs");
include!("tests/migrations.rs");
include!("tests/events_and_effects.rs");
include!("tests/projects_and_flows.rs");
