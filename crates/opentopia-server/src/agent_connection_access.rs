use chrono::Utc;
use opentopia_core::{
    connection_auth_is_runtime_ready, connection_model_tool_name,
    mcp_operation_fingerprint_from_capability, AgentTemplateSpecV1, ConnectionAuthVerificationV1,
    ConnectionBindingV1, ConnectionCapabilityKindV1, ConnectionCapabilityRevisionV1,
    ConnectionStatusV1, ExecutionConnectionOperationV1, ResolvedConnectionBindingV1,
    SqliteSessionStore,
};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ConnectionAccessMode {
    None,
    Legacy,
    Structured,
}

#[derive(Debug, Clone, Copy, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ConnectionAccessIssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConnectionAccessIssueView {
    pub(super) severity: ConnectionAccessIssueSeverity,
    pub(super) code: String,
    pub(super) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) connection_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) operation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConnectionAccessOperationView {
    pub(super) operation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) kind: Option<ConnectionCapabilityKindV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) provider_public_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) model_tool_name: Option<String>,
    pub(super) permission_labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConnectionAccessBindingView {
    pub(super) connection_id: Uuid,
    pub(super) capability_revision: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) connection_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) status: Option<ConnectionStatusV1>,
    pub(super) valid: bool,
    pub(super) operations: Vec<ConnectionAccessOperationView>,
    pub(super) issues: Vec<ConnectionAccessIssueView>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentTemplateConnectionAccessView {
    pub(super) valid: bool,
    pub(super) mode: ConnectionAccessMode,
    pub(super) bindings: Vec<ConnectionAccessBindingView>,
    pub(super) issues: Vec<ConnectionAccessIssueView>,
    pub(super) effective_mcp_server_ids: BTreeSet<Uuid>,
    pub(super) effective_model_tool_names: BTreeSet<String>,
}

pub(super) struct ResolvedAgentTemplateConnectionAccess {
    pub(super) view: AgentTemplateConnectionAccessView,
    pub(super) bindings: Vec<ResolvedConnectionBindingV1>,
}

impl ResolvedAgentTemplateConnectionAccess {
    pub(super) fn require_valid(self) -> Result<Vec<ResolvedConnectionBindingV1>, String> {
        if self.view.valid {
            return Ok(self.bindings);
        }
        let summary = self
            .view
            .issues
            .iter()
            .filter(|issue| issue.severity == ConnectionAccessIssueSeverity::Error)
            .take(3)
            .map(|issue| issue.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        Err(if summary.is_empty() {
            "Agent template Connection access is invalid".to_string()
        } else {
            summary
        })
    }
}

pub(super) fn resolve_agent_template_connection_access(
    store: &SqliteSessionStore,
    spec: &AgentTemplateSpecV1,
) -> anyhow::Result<ResolvedAgentTemplateConnectionAccess> {
    if spec.connection_bindings.is_empty() {
        let legacy =
            spec.capabilities.allow_all_mcp_servers || !spec.capabilities.mcp_servers.is_empty();
        let issues = legacy
            .then(|| ConnectionAccessIssueView {
                severity: ConnectionAccessIssueSeverity::Warning,
                code: "legacy_mcp_server_grants".to_string(),
                message: "This template uses legacy mcpServers grants; migrate it to pinned Connection operations when editing its access policy.".to_string(),
                connection_id: None,
                operation_id: None,
            })
            .into_iter()
            .collect::<Vec<_>>();
        return Ok(ResolvedAgentTemplateConnectionAccess {
            view: AgentTemplateConnectionAccessView {
                valid: true,
                mode: if legacy {
                    ConnectionAccessMode::Legacy
                } else {
                    ConnectionAccessMode::None
                },
                bindings: Vec::new(),
                issues,
                effective_mcp_server_ids: BTreeSet::new(),
                effective_model_tool_names: BTreeSet::new(),
            },
            bindings: Vec::new(),
        });
    }

    let mut views = Vec::with_capacity(spec.connection_bindings.len());
    let mut resolved_bindings = Vec::with_capacity(spec.connection_bindings.len());
    let mut all_issues = Vec::new();
    let mut effective_mcp_server_ids = BTreeSet::new();
    let mut effective_model_tool_names = BTreeSet::new();
    let mut model_name_owners = BTreeMap::new();

    for binding in &spec.connection_bindings {
        let (view, resolved) = resolve_binding(store, binding)?;
        all_issues.extend(view.issues.iter().cloned());
        if let Some(resolved) = resolved {
            effective_mcp_server_ids.extend(resolved.mcp_server_ids());
            for operation in resolved.operations.values() {
                if let Some((existing_connection_id, existing_operation_id)) = model_name_owners
                    .insert(
                        operation.model_tool_name.clone(),
                        (operation.connection_id, operation.operation_id.clone()),
                    )
                {
                    all_issues.push(ConnectionAccessIssueView {
                        severity: ConnectionAccessIssueSeverity::Error,
                        code: "model_tool_name_collision".to_string(),
                        message: format!(
                            "Model tool alias collision between {existing_connection_id}/{existing_operation_id} and {}/{}.",
                            operation.connection_id, operation.operation_id
                        ),
                        connection_id: Some(operation.connection_id),
                        operation_id: Some(operation.operation_id.clone()),
                    });
                }
                effective_model_tool_names.insert(operation.model_tool_name.clone());
            }
            resolved_bindings.push(resolved);
        }
        views.push(view);
    }
    let valid = !all_issues
        .iter()
        .any(|issue| issue.severity == ConnectionAccessIssueSeverity::Error)
        && resolved_bindings.len() == spec.connection_bindings.len();
    Ok(ResolvedAgentTemplateConnectionAccess {
        view: AgentTemplateConnectionAccessView {
            valid,
            mode: ConnectionAccessMode::Structured,
            bindings: views,
            issues: all_issues,
            effective_mcp_server_ids,
            effective_model_tool_names,
        },
        bindings: resolved_bindings,
    })
}

fn resolve_binding(
    store: &SqliteSessionStore,
    binding: &ConnectionBindingV1,
) -> anyhow::Result<(
    ConnectionAccessBindingView,
    Option<ResolvedConnectionBindingV1>,
)> {
    let mut issues = Vec::new();
    let Some(connection) = store.get_connection(binding.connection_id)? else {
        issues.push(error(
            "connection_not_found",
            "The selected Connection no longer exists.",
            binding.connection_id,
            None,
        ));
        return Ok((missing_binding_view(binding, issues), None));
    };
    let definition = store.get_integration_definition(connection.integration_definition_id)?;
    match definition.as_ref() {
        None => issues.push(error(
            "integration_definition_not_found",
            "The Connection's Integration definition no longer exists.",
            binding.connection_id,
            None,
        )),
        Some(definition) if !definition.enabled => issues.push(error(
            "integration_definition_disabled",
            "The Connection's Integration definition is disabled.",
            binding.connection_id,
            None,
        )),
        Some(_) => {}
    }
    if !connection.enabled {
        issues.push(error(
            "connection_disabled",
            "The selected Connection is disabled.",
            binding.connection_id,
            None,
        ));
    }
    if connection.status != ConnectionStatusV1::Ready {
        issues.push(error(
            "connection_not_ready",
            "The selected Connection must pass its health test before use.",
            binding.connection_id,
            None,
        ));
    }
    if let Some(definition) = definition.as_ref() {
        let now = Utc::now();
        if connection_auth_is_runtime_ready(definition, &connection, now) {
            if connection.auth_context.verification
                == ConnectionAuthVerificationV1::LegacyUnverified
            {
                issues.push(warning(
                    "legacy_auth_unverified",
                    "This migrated MCP Connection uses legacy environment credentials that cannot be independently verified.",
                    binding.connection_id,
                    None,
                ));
            }
        } else if connection
            .auth_context
            .expires_at
            .as_ref()
            .is_some_and(|expires_at| expires_at <= &now)
        {
            issues.push(error(
                "connection_auth_expired",
                "The selected Connection account credential has expired.",
                binding.connection_id,
                None,
            ));
        } else {
            issues.push(error(
                "connection_auth_unverified",
                "The selected Connection account is not verified.",
                binding.connection_id,
                None,
            ));
        }
    }

    let pinned = store
        .get_connection_capability_revision(binding.connection_id, binding.capability_revision)?;
    let active = connection
        .active_capability_revision
        .map(|revision| store.get_connection_capability_revision(binding.connection_id, revision))
        .transpose()?
        .flatten();
    if pinned.is_none() {
        issues.push(error(
            "capability_revision_not_found",
            "The reviewed capability revision no longer exists.",
            binding.connection_id,
            None,
        ));
    }
    if connection.active_capability_revision.is_none() || active.is_none() {
        issues.push(error(
            "active_capability_revision_not_found",
            "The Connection has no active capability revision.",
            binding.connection_id,
            None,
        ));
    }

    let pinned_index = pinned.as_ref().map(capability_index).unwrap_or_default();
    let active_index = active.as_ref().map(capability_index).unwrap_or_default();
    let mut resolved_operations = BTreeMap::new();
    let mcp_server_id = connection.runtime_binding.mcp_server_id();
    let mut operations = Vec::with_capacity(binding.operation_grants.len());
    for grant in &binding.operation_grants {
        let pinned_operation = pinned_index.get(grant.operation_id.as_str()).copied();
        let active_operation = active_index.get(grant.operation_id.as_str()).copied();
        if pinned_operation.is_none() {
            issues.push(error(
                "operation_not_in_pinned_revision",
                "The granted operation is absent from the reviewed capability revision.",
                binding.connection_id,
                Some(&grant.operation_id),
            ));
        }
        match (pinned_operation, active_operation) {
            (Some(_), None) => issues.push(error(
                "operation_removed",
                "The granted operation is no longer available from the active Connection revision.",
                binding.connection_id,
                Some(&grant.operation_id),
            )),
            (Some(pinned), Some(active))
                if mcp_operation_fingerprint_from_capability(pinned)
                    != mcp_operation_fingerprint_from_capability(active) =>
            {
                issues.push(error(
                    "operation_descriptor_changed",
                    "The granted operation changed after approval and must be reviewed again.",
                    binding.connection_id,
                    Some(&grant.operation_id),
                ))
            }
            (Some(pinned), Some(_)) => {
                if pinned.provider_metadata.server_id != mcp_server_id {
                    issues.push(error(
                        "operation_runtime_mismatch",
                        "The granted operation does not belong to the Connection's current MCP runtime.",
                        binding.connection_id,
                        Some(&grant.operation_id),
                    ));
                }
                resolved_operations.insert(
                    grant.operation_id.clone(),
                    ExecutionConnectionOperationV1 {
                        connection_id: binding.connection_id,
                        capability_revision: binding.capability_revision,
                        operation_id: grant.operation_id.clone(),
                        mcp_server_id: connection.runtime_binding.mcp_server_id(),
                        provider_tool_name: pinned.provider_metadata.tool_name.clone(),
                        model_tool_name: connection_model_tool_name(
                            binding.connection_id,
                            &pinned.provider_metadata.tool_name,
                        ),
                        pinned_operation_fingerprint: mcp_operation_fingerprint_from_capability(
                            pinned,
                        ),
                    },
                );
            }
            _ => {}
        }
        operations.push(ConnectionAccessOperationView {
            operation_id: grant.operation_id.clone(),
            name: pinned_operation.map(|operation| operation.name.clone()),
            display_name: pinned_operation.map(|operation| operation.display_name.clone()),
            kind: pinned_operation.map(|operation| operation.kind),
            provider_public_name: pinned_operation
                .map(|operation| operation.provider_metadata.public_name.clone()),
            model_tool_name: pinned_operation.map(|operation| {
                connection_model_tool_name(
                    binding.connection_id,
                    &operation.provider_metadata.tool_name,
                )
            }),
            permission_labels: pinned_operation
                .map(|operation| operation.permission_labels.clone())
                .unwrap_or_default(),
        });
    }

    match store.get_mcp_server(mcp_server_id)? {
        None => issues.push(error(
            "mcp_runtime_not_found",
            "The Connection's MCP runtime no longer exists.",
            binding.connection_id,
            None,
        )),
        Some(server) if !server.enabled => issues.push(error(
            "mcp_runtime_disabled",
            "The Connection's MCP runtime is disabled.",
            binding.connection_id,
            None,
        )),
        Some(_) => {}
    }

    let valid = !issues
        .iter()
        .any(|issue| issue.severity == ConnectionAccessIssueSeverity::Error);
    let resolved = valid.then(|| ResolvedConnectionBindingV1 {
        binding: binding.clone(),
        operations: resolved_operations,
    });
    Ok((
        ConnectionAccessBindingView {
            connection_id: binding.connection_id,
            capability_revision: binding.capability_revision,
            connection_name: Some(connection.name),
            status: Some(connection.status),
            valid,
            operations,
            issues,
        },
        resolved,
    ))
}

fn capability_index(
    revision: &ConnectionCapabilityRevisionV1,
) -> BTreeMap<&str, &opentopia_core::ConnectionCapabilityV1> {
    revision
        .capabilities
        .iter()
        .map(|capability| (capability.capability_id.as_str(), capability))
        .collect()
}

fn missing_binding_view(
    binding: &ConnectionBindingV1,
    issues: Vec<ConnectionAccessIssueView>,
) -> ConnectionAccessBindingView {
    ConnectionAccessBindingView {
        connection_id: binding.connection_id,
        capability_revision: binding.capability_revision,
        connection_name: None,
        status: None,
        valid: false,
        operations: binding
            .operation_grants
            .iter()
            .map(|grant| ConnectionAccessOperationView {
                operation_id: grant.operation_id.clone(),
                name: None,
                display_name: None,
                kind: None,
                provider_public_name: None,
                model_tool_name: None,
                permission_labels: Vec::new(),
            })
            .collect(),
        issues,
    }
}

fn error(
    code: &str,
    message: &str,
    connection_id: Uuid,
    operation_id: Option<&str>,
) -> ConnectionAccessIssueView {
    issue(
        ConnectionAccessIssueSeverity::Error,
        code,
        message,
        connection_id,
        operation_id,
    )
}

fn warning(
    code: &str,
    message: &str,
    connection_id: Uuid,
    operation_id: Option<&str>,
) -> ConnectionAccessIssueView {
    issue(
        ConnectionAccessIssueSeverity::Warning,
        code,
        message,
        connection_id,
        operation_id,
    )
}

fn issue(
    severity: ConnectionAccessIssueSeverity,
    code: &str,
    message: &str,
    connection_id: Uuid,
    operation_id: Option<&str>,
) -> ConnectionAccessIssueView {
    ConnectionAccessIssueView {
        severity,
        code: code.to_string(),
        message: message.to_string(),
        connection_id: Some(connection_id),
        operation_id: operation_id.map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentopia_core::{
        AgentBudgetV1, AgentModelPolicyV1, AgentRiskClassV1, CapabilityDiscoveryKindV1,
        CapabilityProjection, ConnectionAccountV1, ConnectionAuthContextV1,
        ConnectionCapabilityRevisionV1, ConnectionOwnerTypeV1, ConnectionRuntimeBindingV1,
        ConnectionV1, IntegrationAuthSchemeV1, IntegrationDefinitionV1, IntegrationKindV1,
        McpServerConfig, McpToolDescriptor, OperationGrantV1,
    };
    use serde_json::json;
    use std::collections::BTreeSet;

    fn template_spec(binding: ConnectionBindingV1) -> AgentTemplateSpecV1 {
        AgentTemplateSpecV1 {
            description: String::new(),
            instructions: "Use only approved CRM operations.".to_string(),
            capabilities: CapabilityProjection::deny_all(),
            resource_grants: Vec::new(),
            model_policy: AgentModelPolicyV1::deny_all(),
            state_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
            allow_all_delegates: false,
            delegate_template_ids: BTreeSet::new(),
            budget: AgentBudgetV1::default(),
            risk_class: AgentRiskClassV1::Low,
            connection_bindings: vec![binding],
        }
    }

    fn tool(server_id: Uuid, name: &str, description: &str) -> McpToolDescriptor {
        McpToolDescriptor {
            public_name: format!("crm__{name}"),
            server_id,
            tool_name: name.to_string(),
            description: Some(description.to_string()),
            input_schema: json!({"type": "object"}),
            annotations: json!({"readOnlyHint": true}),
            meta: json!({}),
            permission_labels: vec!["read".to_string()],
        }
    }

    fn publish_capabilities(
        store: &SqliteSessionStore,
        connection: &ConnectionV1,
        revision: u32,
        tools: &[McpToolDescriptor],
    ) -> ConnectionV1 {
        let capability =
            ConnectionCapabilityRevisionV1::from_mcp_tools(connection.id, revision, tools)
                .expect("build capability revision");
        let mut next = connection.clone();
        next.touch();
        next.status = ConnectionStatusV1::Ready;
        next.active_capability_revision = Some(revision);
        store
            .publish_connection_capability_revision(&next, connection.revision, &capability)
            .expect("publish capability revision")
            .0
    }

    #[test]
    fn unrelated_additions_preserve_access_but_selected_changes_and_removals_fail_closed() {
        let store = SqliteSessionStore::open(":memory:").expect("open store");
        let server = store
            .insert_mcp_server(McpServerConfig::new(
                "CRM runtime".to_string(),
                "crm-mcp".to_string(),
            ))
            .expect("insert runtime");
        let definition = store
            .insert_integration_definition(&IntegrationDefinitionV1::new(
                "crm".to_string(),
                "CRM".to_string(),
                IntegrationKindV1::Mcp,
                IntegrationAuthSchemeV1::None,
                CapabilityDiscoveryKindV1::McpToolsList,
            ))
            .expect("insert definition");
        let connection = store
            .insert_connection(&ConnectionV1::new(
                definition.id,
                "CRM production".to_string(),
                ConnectionOwnerTypeV1::OrgShared,
                "production".to_string(),
                ConnectionRuntimeBindingV1::McpServer {
                    server_id: server.server_id,
                },
                ConnectionAuthContextV1 {
                    credential_ref: None,
                    verification: ConnectionAuthVerificationV1::NotRequired,
                    account: ConnectionAccountV1::default(),
                    granted_scopes: Vec::new(),
                    expires_at: None,
                },
            ))
            .expect("insert connection");
        let selected = tool(server.server_id, "customers_get", "Read customer");
        let connection = publish_capabilities(&store, &connection, 1, &[selected.clone()]);
        let operation_id = format!("connection:{}:tool:customers_get", connection.id);
        let spec = template_spec(ConnectionBindingV1 {
            connection_id: connection.id,
            capability_revision: 1,
            operation_grants: vec![OperationGrantV1 {
                operation_id: operation_id.clone(),
            }],
        });

        let extra = tool(server.server_id, "customers_list", "List customers");
        let connection =
            publish_capabilities(&store, &connection, 2, &[selected.clone(), extra.clone()]);
        let added = resolve_agent_template_connection_access(&store, &spec)
            .expect("resolve after addition");
        assert!(
            added.view.valid,
            "unrelated additions must not expand grants"
        );

        let mut unverified = connection.clone();
        unverified.touch();
        unverified.auth_context.verification = ConnectionAuthVerificationV1::Unverified;
        let unverified = store
            .update_connection(&unverified, connection.revision)
            .expect("mark auth unverified");
        let auth_blocked = resolve_agent_template_connection_access(&store, &spec)
            .expect("resolve unverified Connection");
        assert!(!auth_blocked.view.valid);
        assert!(auth_blocked
            .view
            .issues
            .iter()
            .any(|issue| issue.code == "connection_auth_unverified"));

        let mut degraded = unverified.clone();
        degraded.touch();
        degraded.auth_context.verification = ConnectionAuthVerificationV1::NotRequired;
        degraded.status = ConnectionStatusV1::Degraded;
        let degraded = store
            .update_connection(&degraded, unverified.revision)
            .expect("mark Connection degraded");
        let status_blocked = resolve_agent_template_connection_access(&store, &spec)
            .expect("resolve degraded Connection");
        assert!(!status_blocked.view.valid);
        assert!(status_blocked
            .view
            .issues
            .iter()
            .any(|issue| issue.code == "connection_not_ready"));

        let mut connection = degraded.clone();
        connection.touch();
        connection.status = ConnectionStatusV1::Ready;
        connection.auth_context.verification = ConnectionAuthVerificationV1::Verified;
        connection.auth_context.expires_at = Some(Utc::now() - chrono::Duration::minutes(1));
        let expired = store
            .update_connection(&connection, degraded.revision)
            .expect("expire ready Connection");
        let expiry_blocked = resolve_agent_template_connection_access(&store, &spec)
            .expect("resolve expired Connection");
        assert!(!expiry_blocked.view.valid);
        assert!(expiry_blocked
            .view
            .issues
            .iter()
            .any(|issue| issue.code == "connection_auth_expired"));

        let mut connection = expired.clone();
        connection.touch();
        connection.auth_context.expires_at = None;
        let connection = store
            .update_connection(&connection, expired.revision)
            .expect("restore Connection credential");

        let changed_selected = tool(server.server_id, "customers_get", "Changed descriptor");
        let connection =
            publish_capabilities(&store, &connection, 3, &[changed_selected, extra.clone()]);
        let changed = resolve_agent_template_connection_access(&store, &spec)
            .expect("resolve after selected operation change");
        assert!(!changed.view.valid);
        assert!(changed
            .view
            .issues
            .iter()
            .any(|issue| issue.code == "operation_descriptor_changed"));

        publish_capabilities(&store, &connection, 4, &[extra]);
        let removed = resolve_agent_template_connection_access(&store, &spec)
            .expect("resolve after selected operation removal");
        assert!(!removed.view.valid);
        assert!(removed
            .view
            .issues
            .iter()
            .any(|issue| issue.code == "operation_removed"));
    }

    #[test]
    fn legacy_mcp_grants_remain_valid_with_an_explicit_migration_warning() {
        let store = SqliteSessionStore::open(":memory:").expect("open store");
        let mut spec = template_spec(ConnectionBindingV1 {
            connection_id: Uuid::new_v4(),
            capability_revision: 1,
            operation_grants: vec![OperationGrantV1 {
                operation_id: "unused".to_string(),
            }],
        });
        spec.connection_bindings.clear();
        spec.capabilities
            .mcp_servers
            .insert(Uuid::new_v4().to_string());

        let access =
            resolve_agent_template_connection_access(&store, &spec).expect("resolve legacy access");

        assert!(access.view.valid);
        assert_eq!(access.view.mode, ConnectionAccessMode::Legacy);
        assert!(access.view.issues.iter().any(|issue| {
            issue.code == "legacy_mcp_server_grants"
                && issue.severity == ConnectionAccessIssueSeverity::Warning
        }));
    }

    #[test]
    fn migrated_legacy_auth_is_warning_only_without_a_credential_reference() {
        let store = SqliteSessionStore::open(":memory:").expect("open store");
        let server = store
            .insert_mcp_server(McpServerConfig::new(
                "Legacy CRM runtime".to_string(),
                "crm-mcp".to_string(),
            ))
            .expect("insert runtime");
        let definition = store
            .insert_integration_definition(&IntegrationDefinitionV1::new(
                format!("legacy-mcp-{}", server.server_id),
                "Legacy CRM".to_string(),
                IntegrationKindV1::Mcp,
                IntegrationAuthSchemeV1::External,
                CapabilityDiscoveryKindV1::McpToolsList,
            ))
            .expect("insert definition");
        let connection = store
            .insert_connection(&ConnectionV1::new(
                definition.id,
                "Migrated CRM".to_string(),
                ConnectionOwnerTypeV1::Personal,
                "local".to_string(),
                ConnectionRuntimeBindingV1::McpServer {
                    server_id: server.server_id,
                },
                ConnectionAuthContextV1 {
                    credential_ref: None,
                    verification: ConnectionAuthVerificationV1::LegacyUnverified,
                    account: ConnectionAccountV1::default(),
                    granted_scopes: Vec::new(),
                    expires_at: None,
                },
            ))
            .expect("insert connection");
        let selected = tool(server.server_id, "customers_get", "Read customer");
        let connection = publish_capabilities(&store, &connection, 1, &[selected]);
        let operation_id = format!("connection:{}:tool:customers_get", connection.id);
        let spec = template_spec(ConnectionBindingV1 {
            connection_id: connection.id,
            capability_revision: 1,
            operation_grants: vec![OperationGrantV1 { operation_id }],
        });

        let warning_only = resolve_agent_template_connection_access(&store, &spec)
            .expect("resolve migrated legacy auth");
        assert!(warning_only.view.valid);
        assert!(warning_only.view.issues.iter().any(|issue| {
            issue.code == "legacy_auth_unverified"
                && issue.severity == ConnectionAccessIssueSeverity::Warning
        }));

        let mut ordinary_definition = definition.clone();
        ordinary_definition.touch();
        ordinary_definition.key = "crm".to_string();
        let ordinary_definition = store
            .update_integration_definition(&ordinary_definition, definition.revision)
            .expect("replace legacy definition key");
        let ordinary_definition_blocked = resolve_agent_template_connection_access(&store, &spec)
            .expect("resolve ordinary definition with legacy auth marker");
        assert!(!ordinary_definition_blocked.view.valid);
        assert!(ordinary_definition_blocked
            .view
            .issues
            .iter()
            .any(|issue| issue.code == "connection_auth_unverified"));

        let mut restored_definition = ordinary_definition.clone();
        restored_definition.touch();
        restored_definition.key = format!("legacy-mcp-{}", server.server_id);
        store
            .update_integration_definition(&restored_definition, ordinary_definition.revision)
            .expect("restore legacy definition key");

        let mut credential_bound = connection.clone();
        credential_bound.touch();
        credential_bound.auth_context.credential_ref =
            Some("vault://connections/legacy-crm".to_string());
        store
            .update_connection(&credential_bound, connection.revision)
            .expect("attach credential reference");
        let blocked = resolve_agent_template_connection_access(&store, &spec)
            .expect("resolve invalid legacy auth");
        assert!(!blocked.view.valid);
        assert!(blocked
            .view
            .issues
            .iter()
            .any(|issue| issue.code == "connection_auth_unverified"));
    }
}
