#[test]
fn connection_control_plane_persists_account_and_capability_revisions() {
    use crate::connection::{
        CapabilityDiscoveryKindV1, ConnectionAccountV1, ConnectionAuthContextV1,
        ConnectionAuthVerificationV1, ConnectionCapabilityRevisionV1, ConnectionOwnerTypeV1,
        ConnectionRuntimeBindingV1, ConnectionStatusV1, ConnectionV1, IntegrationAuthSchemeV1,
        IntegrationDefinitionV1, IntegrationKindV1,
    };

    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let mcp = store
        .insert_mcp_server(McpServerConfig::new(
            "CRM runtime".to_string(),
            "crm-mcp".to_string(),
        ))
        .expect("insert MCP runtime");
    let definition = store
        .insert_integration_definition(&IntegrationDefinitionV1::new(
            "crm".to_string(),
            "CRM".to_string(),
            IntegrationKindV1::Mcp,
            IntegrationAuthSchemeV1::External,
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
                server_id: mcp.server_id,
            },
            ConnectionAuthContextV1 {
                credential_ref: Some("vault://connections/crm-production".to_string()),
                verification: ConnectionAuthVerificationV1::Unverified,
                account: ConnectionAccountV1 {
                    tenant_name: Some("Northwind".to_string()),
                    ..ConnectionAccountV1::default()
                },
                granted_scopes: vec!["customers.read".to_string()],
                expires_at: None,
            },
        ))
        .expect("insert connection");

    let tools = vec![McpToolDescriptor {
        public_name: "crm_runtime__customers_get".to_string(),
        server_id: mcp.server_id,
        tool_name: "customers_get".to_string(),
        description: Some("Read customer".to_string()),
        input_schema: serde_json::json!({"type": "object"}),
        annotations: serde_json::json!({"readOnlyHint": true}),
        meta: serde_json::json!({"notPersisted": true}),
        permission_labels: vec!["read".to_string()],
    }];
    let capability = ConnectionCapabilityRevisionV1::from_mcp_tools(connection.id, 1, &tools)
        .expect("build capability revision");
    let expected_revision = connection.revision;
    let mut ready = connection.clone();
    ready.touch();
    ready.status = ConnectionStatusV1::Ready;
    ready.active_capability_revision = Some(capability.revision);
    let (ready, saved_capability) = store
        .publish_connection_capability_revision(&ready, expected_revision, &capability)
        .expect("publish capability revision");

    assert_eq!(
        store
            .list_connections(Some(definition.id), Some(ConnectionStatusV1::Ready))
            .expect("list ready connections"),
        vec![ready.clone()]
    );
    assert_eq!(
        store
            .get_connection_capability_revision(connection.id, 1)
            .expect("get capability revision"),
        Some(saved_capability)
    );
    assert_eq!(
        store
            .list_connection_capability_revisions(connection.id)
            .expect("list capability revisions")
            .len(),
        1
    );
}

#[test]
fn connection_update_is_cas_protected_and_mcp_runtime_is_account_exclusive() {
    use crate::connection::{
        CapabilityDiscoveryKindV1, ConnectionAuthContextV1, ConnectionOwnerTypeV1,
        ConnectionRuntimeBindingV1, ConnectionV1, IntegrationAuthSchemeV1, IntegrationDefinitionV1,
        IntegrationKindV1,
    };

    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let mcp = store
        .insert_mcp_server(McpServerConfig::new(
            "ERP runtime".to_string(),
            "erp-mcp".to_string(),
        ))
        .expect("insert MCP runtime");
    let definition = store
        .insert_integration_definition(&IntegrationDefinitionV1::new(
            "erp".to_string(),
            "ERP".to_string(),
            IntegrationKindV1::Mcp,
            IntegrationAuthSchemeV1::None,
            CapabilityDiscoveryKindV1::McpToolsList,
        ))
        .expect("insert definition");
    let connection = store
        .insert_connection(&ConnectionV1::new(
            definition.id,
            "ERP local".to_string(),
            ConnectionOwnerTypeV1::Personal,
            "local".to_string(),
            ConnectionRuntimeBindingV1::McpServer {
                server_id: mcp.server_id,
            },
            ConnectionAuthContextV1::default(),
        ))
        .expect("insert connection");

    let duplicate = ConnectionV1::new(
        definition.id,
        "Other account".to_string(),
        ConnectionOwnerTypeV1::Personal,
        "local".to_string(),
        ConnectionRuntimeBindingV1::McpServer {
            server_id: mcp.server_id,
        },
        ConnectionAuthContextV1::default(),
    );
    let duplicate_error = store
        .insert_connection(&duplicate)
        .expect_err("runtime must not be shared");
    assert!(matches!(
        duplicate_error.downcast_ref::<ConnectionStoreError>(),
        Some(ConnectionStoreError::McpRuntimeAlreadyBound(id)) if *id == mcp.server_id
    ));

    let mut update = connection.clone();
    update.touch();
    store
        .update_connection(&update, connection.revision)
        .expect("first CAS update");
    let conflict = store
        .update_connection(&update, connection.revision)
        .expect_err("stale CAS must conflict");
    assert!(matches!(
        conflict.downcast_ref::<ConnectionStoreError>(),
        Some(ConnectionStoreError::ConnectionRevisionConflict(revision))
            if *revision == update.revision
    ));
}

#[test]
fn legacy_projection_compensation_removes_runtime_connection_then_definition() {
    use crate::connection::{
        CapabilityDiscoveryKindV1, ConnectionAuthContextV1, ConnectionOwnerTypeV1,
        ConnectionRuntimeBindingV1, ConnectionV1, IntegrationAuthSchemeV1, IntegrationDefinitionV1,
        IntegrationKindV1,
    };

    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let server = store
        .insert_mcp_server(McpServerConfig::new(
            "Temporary runtime".to_string(),
            "temporary-mcp".to_string(),
        ))
        .expect("insert MCP runtime");
    let mut definition = IntegrationDefinitionV1::new(
        format!("legacy-mcp-{}", server.server_id),
        server.name.clone(),
        IntegrationKindV1::Mcp,
        IntegrationAuthSchemeV1::None,
        CapabilityDiscoveryKindV1::McpToolsList,
    );
    definition.id = server.server_id;
    store
        .insert_integration_definition(&definition)
        .expect("insert definition");
    let mut connection = ConnectionV1::new(
        definition.id,
        server.name.clone(),
        ConnectionOwnerTypeV1::Personal,
        "local".to_string(),
        ConnectionRuntimeBindingV1::McpServer {
            server_id: server.server_id,
        },
        ConnectionAuthContextV1::default(),
    );
    connection.id = server.server_id;
    store
        .insert_connection(&connection)
        .expect("insert connection");

    assert!(
        store.delete_integration_definition(definition.id).is_err(),
        "a definition with a live Connection must remain protected"
    );
    assert!(store
        .delete_mcp_server(server.server_id)
        .expect("delete runtime"));
    assert_eq!(
        store
            .get_connection(connection.id)
            .expect("load cascaded connection"),
        None
    );
    assert!(store
        .delete_integration_definition(definition.id)
        .expect("delete orphan definition"));
    assert_eq!(
        store
            .get_integration_definition(definition.id)
            .expect("load deleted definition"),
        None
    );
}
