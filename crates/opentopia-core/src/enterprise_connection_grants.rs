use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use uuid::Uuid;

pub const MAX_TEMPLATE_CONNECTION_BINDINGS: usize = 32;
pub const MAX_TEMPLATE_OPERATION_GRANTS: usize = 256;

/// Pins one account-level Connection and the immutable capability revision that
/// was reviewed when an Agent template was published.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionBindingV1 {
    pub connection_id: Uuid,
    pub capability_revision: u32,
    #[serde(default)]
    pub operation_grants: Vec<OperationGrantV1>,
}

/// Grants one stable provider operation. The operation ID is deliberately not
/// a runtime tool name; the server resolves it through the pinned capability
/// revision before execution.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationGrantV1 {
    pub operation_id: String,
}

/// Server-resolved projection used to freeze a validated execution boundary.
/// It contains no credentials or provider-private metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConnectionBindingV1 {
    pub binding: ConnectionBindingV1,
    pub operations: BTreeMap<String, ExecutionConnectionOperationV1>,
}

/// Credential-free operation route frozen into an Agent instance. The runtime
/// must still revalidate the live Connection and fingerprint immediately before
/// crossing the external-call boundary.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionConnectionOperationV1 {
    pub connection_id: Uuid,
    pub capability_revision: u32,
    pub operation_id: String,
    pub mcp_server_id: Uuid,
    pub provider_tool_name: String,
    pub model_tool_name: String,
    pub pinned_operation_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionGrantChangeKind {
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionGrantChange {
    pub connection_id: Uuid,
    pub operation_id: Option<String>,
    pub kind: ConnectionGrantChangeKind,
}

impl ResolvedConnectionBindingV1 {
    pub fn mcp_server_ids(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.operations
            .values()
            .map(|operation| operation.mcp_server_id)
    }

    pub fn model_tool_names(&self) -> impl Iterator<Item = &String> {
        self.operations
            .values()
            .map(|operation| &operation.model_tool_name)
    }
}

/// Builds a provider-safe, account-scoped model alias. Provider-native names
/// remain the fixed wire route and never become the authorization key.
pub fn connection_model_tool_name(connection_id: Uuid, provider_tool_name: &str) -> String {
    const MAX_SLUG_LEN: usize = 42;
    let mut slug = provider_tool_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    while slug.contains("__") {
        slug = slug.replace("__", "_");
    }
    let slug = slug.trim_matches('_');
    let slug = if slug.is_empty() { "operation" } else { slug };
    let slug = slug.chars().take(MAX_SLUG_LEN).collect::<String>();
    let digest = Sha256::digest(format!("{connection_id}\0{provider_tool_name}").as_bytes());
    let suffix = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("mcp_{slug}_{suffix}")
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConnectionGrantShapeError {
    #[error("Agent template cannot bind more than {MAX_TEMPLATE_CONNECTION_BINDINGS} Connections")]
    TooManyConnections,
    #[error("Agent template cannot grant more than {MAX_TEMPLATE_OPERATION_GRANTS} operations")]
    TooManyOperations,
    #[error("Connection capability revision must be greater than zero: {0}")]
    InvalidCapabilityRevision(Uuid),
    #[error("duplicate Connection binding: {0}")]
    DuplicateConnection(Uuid),
    #[error("Connection binding must grant at least one operation: {0}")]
    EmptyOperationGrants(Uuid),
    #[error("operationId must be non-empty and at most 512 characters")]
    InvalidOperationId,
    #[error("duplicate operationId for Connection {connection_id}: {operation_id}")]
    DuplicateOperation {
        connection_id: Uuid,
        operation_id: String,
    },
}

pub fn validate_connection_bindings_shape(
    bindings: &[ConnectionBindingV1],
) -> Result<(), ConnectionGrantShapeError> {
    if bindings.len() > MAX_TEMPLATE_CONNECTION_BINDINGS {
        return Err(ConnectionGrantShapeError::TooManyConnections);
    }
    let mut connection_ids = BTreeSet::new();
    let mut operation_count = 0usize;
    for binding in bindings {
        if binding.capability_revision == 0 {
            return Err(ConnectionGrantShapeError::InvalidCapabilityRevision(
                binding.connection_id,
            ));
        }
        if !connection_ids.insert(binding.connection_id) {
            return Err(ConnectionGrantShapeError::DuplicateConnection(
                binding.connection_id,
            ));
        }
        if binding.operation_grants.is_empty() {
            return Err(ConnectionGrantShapeError::EmptyOperationGrants(
                binding.connection_id,
            ));
        }
        let mut operation_ids = BTreeSet::new();
        for grant in &binding.operation_grants {
            operation_count = operation_count.saturating_add(1);
            if operation_count > MAX_TEMPLATE_OPERATION_GRANTS {
                return Err(ConnectionGrantShapeError::TooManyOperations);
            }
            let operation_id = grant.operation_id.trim();
            if operation_id.is_empty()
                || operation_id.chars().count() > 512
                || operation_id != grant.operation_id
            {
                return Err(ConnectionGrantShapeError::InvalidOperationId);
            }
            if !operation_ids.insert(operation_id) {
                return Err(ConnectionGrantShapeError::DuplicateOperation {
                    connection_id: binding.connection_id,
                    operation_id: operation_id.to_string(),
                });
            }
        }
    }
    Ok(())
}

/// Intersection is the only composition operation. It can remove a Connection
/// or operation but can never introduce one that is absent from `boundary`.
pub fn intersect_connection_bindings(
    boundary: &[ConnectionBindingV1],
    requested: &[ConnectionBindingV1],
) -> Vec<ConnectionBindingV1> {
    let requested = requested
        .iter()
        .map(|binding| {
            (
                binding.connection_id,
                (
                    binding.capability_revision,
                    binding
                        .operation_grants
                        .iter()
                        .map(|grant| grant.operation_id.as_str())
                        .collect::<BTreeSet<_>>(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    boundary
        .iter()
        .filter_map(|binding| {
            let (requested_revision, requested_operations) =
                requested.get(&binding.connection_id)?;
            if binding.capability_revision != *requested_revision {
                return None;
            }
            let operation_grants = binding
                .operation_grants
                .iter()
                .filter(|grant| requested_operations.contains(grant.operation_id.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            (!operation_grants.is_empty()).then(|| ConnectionBindingV1 {
                connection_id: binding.connection_id,
                capability_revision: binding.capability_revision,
                operation_grants,
            })
        })
        .collect()
}

pub fn connection_bindings_are_subset(
    candidate: &[ConnectionBindingV1],
    boundary: &[ConnectionBindingV1],
) -> bool {
    let boundary = boundary
        .iter()
        .map(|binding| {
            (
                binding.connection_id,
                (
                    binding.capability_revision,
                    binding
                        .operation_grants
                        .iter()
                        .map(|grant| grant.operation_id.as_str())
                        .collect::<BTreeSet<_>>(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    candidate.iter().all(|binding| {
        boundary
            .get(&binding.connection_id)
            .is_some_and(|(revision, allowed)| {
                binding.capability_revision == *revision
                    && binding
                        .operation_grants
                        .iter()
                        .all(|grant| allowed.contains(grant.operation_id.as_str()))
            })
    })
}

pub fn resolved_bindings_match(
    expected: &[ConnectionBindingV1],
    resolved: &[ResolvedConnectionBindingV1],
) -> bool {
    if expected.len() != resolved.len() {
        return false;
    }
    let resolved = resolved
        .iter()
        .map(|binding| (binding.binding.connection_id, binding))
        .collect::<BTreeMap<_, _>>();
    expected.iter().all(|binding| {
        resolved
            .get(&binding.connection_id)
            .is_some_and(|candidate| {
                if candidate.binding != *binding {
                    return false;
                }
                let granted = binding
                    .operation_grants
                    .iter()
                    .map(|grant| grant.operation_id.as_str())
                    .collect::<BTreeSet<_>>();
                candidate.operations.len() == granted.len()
                    && candidate
                        .operations
                        .iter()
                        .all(|(operation_id, operation)| {
                            granted.contains(operation_id.as_str())
                                && operation.connection_id == binding.connection_id
                                && operation.capability_revision == binding.capability_revision
                                && operation.operation_id == *operation_id
                        })
            })
    })
}

pub fn diff_connection_bindings(
    previous: &[ConnectionBindingV1],
    next: &[ConnectionBindingV1],
) -> Vec<ConnectionGrantChange> {
    let index = |bindings: &[ConnectionBindingV1]| {
        bindings
            .iter()
            .map(|binding| {
                (
                    binding.connection_id,
                    (
                        binding.capability_revision,
                        binding
                            .operation_grants
                            .iter()
                            .map(|grant| grant.operation_id.clone())
                            .collect::<BTreeSet<_>>(),
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let previous = index(previous);
    let next = index(next);
    let mut changes = Vec::new();
    for connection_id in next.keys().filter(|id| !previous.contains_key(id)) {
        changes.push(ConnectionGrantChange {
            connection_id: *connection_id,
            operation_id: None,
            kind: ConnectionGrantChangeKind::Added,
        });
    }
    for connection_id in previous.keys().filter(|id| !next.contains_key(id)) {
        changes.push(ConnectionGrantChange {
            connection_id: *connection_id,
            operation_id: None,
            kind: ConnectionGrantChangeKind::Removed,
        });
    }
    for (connection_id, (next_revision, next_operations)) in &next {
        let Some((previous_revision, previous_operations)) = previous.get(connection_id) else {
            continue;
        };
        if previous_revision != next_revision {
            changes.push(ConnectionGrantChange {
                connection_id: *connection_id,
                operation_id: None,
                kind: ConnectionGrantChangeKind::Removed,
            });
            changes.push(ConnectionGrantChange {
                connection_id: *connection_id,
                operation_id: None,
                kind: ConnectionGrantChangeKind::Added,
            });
            continue;
        }
        for operation_id in next_operations.difference(previous_operations) {
            changes.push(ConnectionGrantChange {
                connection_id: *connection_id,
                operation_id: Some(operation_id.clone()),
                kind: ConnectionGrantChangeKind::Added,
            });
        }
        for operation_id in previous_operations.difference(next_operations) {
            changes.push(ConnectionGrantChange {
                connection_id: *connection_id,
                operation_id: Some(operation_id.clone()),
                kind: ConnectionGrantChangeKind::Removed,
            });
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(connection_id: Uuid, operations: &[&str]) -> ConnectionBindingV1 {
        ConnectionBindingV1 {
            connection_id,
            capability_revision: 1,
            operation_grants: operations
                .iter()
                .map(|operation_id| OperationGrantV1 {
                    operation_id: (*operation_id).to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn intersection_only_narrows_operations() {
        let connection_id = Uuid::new_v4();
        let boundary = vec![binding(connection_id, &["read", "write"])];
        let requested = vec![binding(connection_id, &["read", "delete"])];

        let effective = intersect_connection_bindings(&boundary, &requested);

        assert_eq!(effective, vec![binding(connection_id, &["read"])]);
        assert!(connection_bindings_are_subset(&effective, &boundary));
        assert!(!connection_bindings_are_subset(&requested, &boundary));

        let mut different_revision = binding(connection_id, &["read"]);
        different_revision.capability_revision = 2;
        assert!(intersect_connection_bindings(&boundary, &[different_revision]).is_empty());
    }

    #[test]
    fn duplicate_connections_and_operations_are_rejected() {
        let connection_id = Uuid::new_v4();
        let duplicate_operation = binding(connection_id, &["read", "read"]);
        assert!(matches!(
            validate_connection_bindings_shape(&[duplicate_operation]),
            Err(ConnectionGrantShapeError::DuplicateOperation { .. })
        ));
        assert!(matches!(
            validate_connection_bindings_shape(&[
                binding(connection_id, &["read"]),
                binding(connection_id, &["write"]),
            ]),
            Err(ConnectionGrantShapeError::DuplicateConnection(_))
        ));
        let whitespace = binding(connection_id, &[" read"]);
        assert_eq!(
            validate_connection_bindings_shape(&[whitespace]),
            Err(ConnectionGrantShapeError::InvalidOperationId)
        );
    }

    #[test]
    fn revision_changes_require_reapproval_and_removals_do_not_widen() {
        let connection_id = Uuid::new_v4();
        let previous = vec![binding(connection_id, &["read", "write"])];
        let mut next_revision = binding(connection_id, &["read", "write"]);
        next_revision.capability_revision = 2;
        let revision_changes = diff_connection_bindings(&previous, &[next_revision]);
        assert!(revision_changes
            .iter()
            .any(|change| change.kind == ConnectionGrantChangeKind::Added));

        let removals = diff_connection_bindings(&previous, &[binding(connection_id, &["read"])]);
        assert!(removals
            .iter()
            .all(|change| change.kind == ConnectionGrantChangeKind::Removed));

        let additions = diff_connection_bindings(&[binding(connection_id, &["read"])], &previous);
        assert!(additions
            .iter()
            .any(|change| change.kind == ConnectionGrantChangeKind::Added));
    }

    #[test]
    fn model_tool_aliases_are_account_scoped_and_provider_safe() {
        let first = connection_model_tool_name(Uuid::new_v4(), "customers/get");
        let second = connection_model_tool_name(Uuid::new_v4(), "customers/get");

        assert_ne!(first, second);
        assert!(first.len() <= 64);
        assert!(first
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'));
    }
}
