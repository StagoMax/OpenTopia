use super::AppState;
use crate::connection_operation_runtime::StoreConnectionOperationInvocationGate;
use anyhow::Context;
use opentopia_core::{
    validate_json_schema_value, ConnectionOperationInvocationGate, FlowRunV1, HumanTaskV1,
    SessionStore, WorkflowDeliveryReceiptV1, WorkflowDeliveryStatusV1, WorkflowOutputSpecV1,
};
use serde_json::{json, Value};
use std::time::Duration;

/// Executes one durable output contract. The receipt is claimed with CAS before
/// touching an external provider, and every provider call carries the receipt's
/// stable idempotency key. `force` is reserved for an explicit operator retry.
pub(crate) async fn deliver_run_output(
    state: &AppState,
    run: &FlowRunV1,
    force: bool,
) -> anyhow::Result<WorkflowDeliveryReceiptV1> {
    let deployment_id = run
        .deployment_id
        .context("Flow run is not backed by a Workflow deployment")?;
    let snapshot = run
        .deployment_snapshot
        .as_ref()
        .context("Flow run has no immutable deployment snapshot")?;
    let output = &snapshot.output;

    let mut receipt = match state.store.get_workflow_delivery_receipt_for_run(run.id)? {
        Some(receipt) => receipt,
        None => {
            let candidate =
                WorkflowDeliveryReceiptV1::pending(run.id, deployment_id, output.kind_name());
            match state.store.insert_workflow_delivery_receipt(&candidate) {
                Ok(receipt) => receipt,
                Err(_) => state
                    .store
                    .get_workflow_delivery_receipt_for_run(run.id)?
                    .context("DeliveryReceipt creation raced but no receipt exists")?,
            }
        }
    };

    match receipt.status {
        WorkflowDeliveryStatusV1::Delivered
        | WorkflowDeliveryStatusV1::WaitingHuman
        | WorkflowDeliveryStatusV1::Cancelled => return Ok(receipt),
        WorkflowDeliveryStatusV1::Pending if receipt.attempt > 0 && !force => return Ok(receipt),
        WorkflowDeliveryStatusV1::Failed if !force => return Ok(receipt),
        WorkflowDeliveryStatusV1::Pending | WorkflowDeliveryStatusV1::Failed => {}
    }

    let expected_revision = receipt.revision;
    receipt.begin_attempt();
    receipt = match state
        .store
        .update_workflow_delivery_receipt(&receipt, expected_revision)
    {
        Ok(receipt) => receipt,
        Err(_) => {
            return state
                .store
                .get_workflow_delivery_receipt_for_run(run.id)?
                .context("DeliveryReceipt claim raced but no receipt exists")
        }
    };
    let claimed_revision = receipt.revision;

    let output_value = run.output.as_ref().unwrap_or(&Value::Null);
    let output_validation = validate_json_schema_value(
        &snapshot.compiled_workflow.output_schema,
        output_value,
        "$output",
    )
    .map_err(|error| format!("Output schema validation failed: {error}"))
    .and_then(|_| {
        if matches!(
            output,
            WorkflowOutputSpecV1::Webhook { .. } | WorkflowOutputSpecV1::ConnectionOperation { .. }
        ) {
            validate_outbound_payload(output_value)
        } else {
            Ok(())
        }
    });
    if let Err(error) = output_validation {
        receipt.mark_failed(None, error);
        let final_receipt = state
            .store
            .update_workflow_delivery_receipt(&receipt, claimed_revision)?;
        ensure_delivery_recovery_task(state, run, &final_receipt)?;
        return Ok(final_receipt);
    }

    match output {
        WorkflowOutputSpecV1::Inbox => {
            receipt.mark_delivered(None, Some(json!({ "stored": "flow_run.output" })));
        }
        WorkflowOutputSpecV1::Webhook {
            endpoint,
            credential_ref,
        } => {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()?;
            let mut request = client
                .post(endpoint)
                .header("idempotency-key", &receipt.idempotency_key)
                .json(&json!({
                    "schemaVersion": 1,
                    "runId": run.id,
                    "deploymentId": deployment_id,
                    "flowId": run.flow_id,
                    "flowVersion": run.flow_version,
                    "output": run.output.clone().unwrap_or(Value::Null),
                }));
            if let Some(reference) = credential_ref {
                let env_name = reference
                    .strip_prefix("env:")
                    .context("invalid webhook credential reference")?;
                let secret = std::env::var(env_name)
                    .with_context(|| format!("Webhook credential {env_name} is unavailable"))?;
                request = request.bearer_auth(secret);
            }
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let response_status = Some(status.as_u16());
                    let body = response.text().await.unwrap_or_default();
                    if status.is_success() {
                        receipt.mark_delivered(
                            response_status,
                            Some(json!({ "body": bounded_provider_text(&body) })),
                        );
                    } else {
                        receipt.mark_failed(
                            response_status,
                            format!("Webhook returned HTTP {}", status.as_u16()),
                        );
                    }
                }
                Err(error) => {
                    receipt.mark_failed(None, format!("Webhook delivery failed: {error}"))
                }
            }
        }
        WorkflowOutputSpecV1::ConnectionOperation { operation } => {
            let gate = StoreConnectionOperationInvocationGate::new(state.store.clone());
            let result = async {
                gate.authorize(operation).await?;
                let runtime = state
                    .store
                    .get_mcp_server(operation.mcp_server_id)?
                    .context("Connection MCP runtime no longer exists")?;
                state.mcp_host.ensure_server(runtime).await?;
                let arguments = run.output.clone().unwrap_or_else(|| json!({}));
                let result = state
                    .mcp_host
                    .call_server_tool(
                        operation.mcp_server_id,
                        &operation.provider_tool_name,
                        &operation.pinned_operation_fingerprint,
                        arguments,
                    )
                    .await?;
                anyhow::ensure!(!result.is_error, "Connection operation reported an error");
                Ok::<_, anyhow::Error>(serde_json::to_value(result)?)
            }
            .await;
            match result {
                Ok(result) => receipt.mark_delivered(None, Some(result)),
                Err(error) => {
                    receipt.mark_failed(None, format!("Connection output failed: {error}"))
                }
            }
        }
        WorkflowOutputSpecV1::HumanTask {
            title,
            description,
            assigned_to,
        } => {
            let task = HumanTaskV1::delivery_handoff(
                run.thread_id,
                receipt.id,
                title.clone(),
                description.clone(),
                assigned_to.clone(),
                json!({
                    "runId": run.id,
                    "deploymentId": deployment_id,
                    "output": run.output,
                }),
            );
            if let Err(error) = state.store.insert_human_task(&task) {
                if state.store.get_human_task(task.id)?.is_none() {
                    receipt.mark_failed(None, format!("HumanTask creation failed: {error}"));
                }
            }
            if receipt.status != WorkflowDeliveryStatusV1::Failed {
                receipt.mark_waiting_human(task.id);
            }
        }
    }

    let final_receipt = state
        .store
        .update_workflow_delivery_receipt(&receipt, claimed_revision)?;
    if final_receipt.status == WorkflowDeliveryStatusV1::Failed {
        ensure_delivery_recovery_task(state, run, &final_receipt)?;
    }
    Ok(final_receipt)
}

fn ensure_delivery_recovery_task(
    state: &AppState,
    run: &FlowRunV1,
    receipt: &WorkflowDeliveryReceiptV1,
) -> anyhow::Result<()> {
    let task = HumanTaskV1::delivery_recovery(
        run.thread_id,
        receipt.id,
        receipt
            .error
            .clone()
            .unwrap_or_else(|| "输出投递失败，需要人工检查。".to_string()),
        json!({
            "runId": run.id,
            "deploymentId": receipt.deployment_id,
            "receiptId": receipt.id,
            "attempt": receipt.attempt,
            "outputKind": receipt.output_kind,
            "error": receipt.error,
        }),
    );
    if state.store.get_human_task(task.id)?.is_none() {
        state.store.insert_human_task(&task)?;
    }
    Ok(())
}

/// A provider call interrupted by process loss has an indeterminate external
/// outcome. Never replay it automatically: surface a recovery task so an
/// operator can inspect the provider and choose an explicit idempotent retry.
pub(crate) fn reconcile_interrupted_deliveries(state: &AppState) -> anyhow::Result<()> {
    let cutoff = chrono::Utc::now() - chrono::Duration::seconds(30);
    for mut receipt in state
        .store
        .list_workflow_delivery_receipts(None, Some(WorkflowDeliveryStatusV1::Pending))?
    {
        if receipt.attempt == 0 || receipt.updated_at > cutoff {
            continue;
        }
        let expected = receipt.revision;
        receipt.mark_failed(
            None,
            "Delivery was interrupted; provider outcome is indeterminate. Inspect before retrying.",
        );
        let receipt = match state
            .store
            .update_workflow_delivery_receipt(&receipt, expected)
        {
            Ok(receipt) => receipt,
            Err(_) => continue,
        };
        if let Some(run) = state.store.get_flow_run(receipt.run_id)? {
            ensure_delivery_recovery_task(state, &run, &receipt)?;
        }
    }
    Ok(())
}

fn bounded_provider_text(value: &str) -> String {
    value.chars().take(2_000).collect()
}

fn validate_outbound_payload(value: &Value) -> Result<(), String> {
    let encoded = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    if encoded.len() > 1_048_576 {
        return Err("Outbound payload exceeds the 1 MiB DLP boundary".to_string());
    }
    let mut sensitive_path = None;
    find_sensitive_field(value, "$output", &mut sensitive_path);
    if let Some(path) = sensitive_path {
        return Err(format!(
            "Outbound DLP blocked credential-like field {path}; use a credentialRef instead"
        ));
    }
    Ok(())
}

fn find_sensitive_field(value: &Value, path: &str, found: &mut Option<String>) {
    if found.is_some() {
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let normalized = key.to_ascii_lowercase().replace('-', "_");
                if matches!(
                    normalized.as_str(),
                    "password"
                        | "secret"
                        | "credential"
                        | "credentials"
                        | "api_key"
                        | "apikey"
                        | "access_token"
                        | "accesstoken"
                        | "refresh_token"
                        | "refreshtoken"
                        | "authorization"
                ) || normalized.ends_with("_password")
                    || normalized.ends_with("_secret")
                {
                    *found = Some(format!("{path}.{key}"));
                    return;
                }
                find_sensitive_field(child, &format!("{path}.{key}"), found);
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                find_sensitive_field(child, &format!("{path}[{index}]"), found);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_dlp_rejects_credentials_but_allows_business_tokens() {
        assert!(validate_outbound_payload(&json!({
            "orderTokenCount": 4,
            "customer": { "name": "Ada" }
        }))
        .is_ok());
        let error = validate_outbound_payload(&json!({
            "customer": { "accessToken": "do-not-send" }
        }))
        .expect_err("access token must be blocked");
        assert!(error.contains("$output.customer.accessToken"));
    }
}
