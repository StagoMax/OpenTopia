use super::{codex_app_server_command, codex_wait_for_response, codex_write_rpc};
use anyhow::Context;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;
/// Public account controls for the local Codex App Server.
///
/// The server owns the child process and keeps authentication inside Codex.
/// OpenTopia only exposes non-secret account metadata and the documented login
/// instructions to its UI.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountStatus {
    pub logged_in: bool,
    pub auth_mode: Option<String>,
    pub plan_type: Option<String>,
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub login_pending: bool,
    pub login_id: Option<String>,
    pub login_type: Option<String>,
    pub auth_url: Option<String>,
    pub verification_url: Option<String>,
    pub user_code: Option<String>,
    pub rate_limits: Option<Value>,
    pub usage: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexLoginStart {
    pub login_id: String,
    pub login_type: String,
    pub auth_url: Option<String>,
    pub verification_url: Option<String>,
    pub user_code: Option<String>,
}

struct CodexAccountSession {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    login: Option<CodexLoginStart>,
}

/// A serialized controller for account/login and account/rateLimits calls.
///
/// Login must remain attached to the same App Server process until the
/// completion notification arrives. Turn providers may start their own App
/// Server children; Codex persists the resulting login in its normal local
/// credential store, so those children use the same account.
#[derive(Default)]
pub struct CodexAccountManager {
    session: Mutex<Option<CodexAccountSession>>,
}

impl CodexAccountManager {
    async fn ensure_session(
        &self,
    ) -> anyhow::Result<tokio::sync::MutexGuard<'_, Option<CodexAccountSession>>> {
        let mut guard = self.session.lock().await;
        let needs_restart = guard
            .as_mut()
            .is_some_and(|session| session.child.try_wait().ok().flatten().is_some());
        if needs_restart {
            if let Some(mut session) = guard.take() {
                session.cleanup().await;
            }
        }
        if guard.is_none() {
            let mut command = codex_app_server_command();
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let mut child = command.spawn().context(
                "failed to start the local Codex App Server; install Codex or add it to PATH",
            )?;
            let stdin = child
                .stdin
                .take()
                .context("Codex App Server did not expose stdin")?;
            let stdout = child
                .stdout
                .take()
                .context("Codex App Server did not expose stdout")?;
            let mut session = CodexAccountSession {
                child,
                stdin,
                stdout: BufReader::new(stdout).lines(),
                login: None,
            };
            codex_write_rpc(
                &mut session.stdin,
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "clientInfo": { "name": "OpenTopia", "version": env!("CARGO_PKG_VERSION") },
                        "capabilities": { "experimentalApi": true }
                    }
                }),
            )
            .await?;
            if let Err(error) = codex_wait_for_response(&mut session.stdout, 1).await {
                session.cleanup().await;
                return Err(error);
            }
            *guard = Some(session);
        }
        Ok(guard)
    }

    pub async fn status(&self) -> anyhow::Result<CodexAccountStatus> {
        let mut guard = self.ensure_session().await?;
        let session = guard
            .as_mut()
            .context("Codex account session unavailable")?;
        let account = codex_account_request(session, 2, "account/read", json!({})).await?;
        let auth_mode = account_string(&account, "authMode");
        let plan_type = account_string(&account, "planType");
        let email = account_string(&account, "email");
        let account_id = account_string(&account, "accountId");
        let logged_in = auth_mode
            .as_deref()
            .is_some_and(|mode| !mode.is_empty() && mode != "null");
        if logged_in {
            session.login = None;
        }
        let rate_limits = if logged_in {
            codex_account_request(session, 3, "account/rateLimits/read", json!({}))
                .await
                .ok()
        } else {
            None
        };
        let usage = if logged_in {
            codex_account_request(session, 4, "account/usage/read", json!({}))
                .await
                .ok()
        } else {
            None
        };
        let login = session.login.clone();
        Ok(CodexAccountStatus {
            logged_in,
            auth_mode,
            plan_type,
            email,
            account_id,
            login_pending: login.is_some(),
            login_id: login.as_ref().map(|value| value.login_id.clone()),
            login_type: login.as_ref().map(|value| value.login_type.clone()),
            auth_url: login.as_ref().and_then(|value| value.auth_url.clone()),
            verification_url: login
                .as_ref()
                .and_then(|value| value.verification_url.clone()),
            user_code: login.as_ref().and_then(|value| value.user_code.clone()),
            rate_limits,
            usage,
        })
    }

    pub async fn start_chatgpt_login(&self, device_code: bool) -> anyhow::Result<CodexLoginStart> {
        let mut guard = self.ensure_session().await?;
        let session = guard
            .as_mut()
            .context("Codex account session unavailable")?;
        if let Some(login) = &session.login {
            return Ok(login.clone());
        }
        let login_type = if device_code {
            "chatgptDeviceCode"
        } else {
            "chatgpt"
        };
        let result = codex_account_request(
            session,
            5,
            "account/login/start",
            json!({ "type": login_type }),
        )
        .await?;
        let login = CodexLoginStart {
            login_id: result
                .get("loginId")
                .and_then(Value::as_str)
                .context("Codex login response omitted loginId")?
                .to_string(),
            login_type: result
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or(login_type)
                .to_string(),
            auth_url: result
                .get("authUrl")
                .and_then(Value::as_str)
                .map(str::to_string),
            verification_url: result
                .get("verificationUrl")
                .and_then(Value::as_str)
                .map(str::to_string),
            user_code: result
                .get("userCode")
                .and_then(Value::as_str)
                .map(str::to_string),
        };
        session.login = Some(login.clone());
        Ok(login)
    }

    pub async fn cancel_login(&self) -> anyhow::Result<()> {
        let mut guard = self.ensure_session().await?;
        let session = guard
            .as_mut()
            .context("Codex account session unavailable")?;
        if let Some(login) = session.login.take() {
            codex_account_request(
                session,
                6,
                "account/login/cancel",
                json!({ "loginId": login.login_id }),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn logout(&self) -> anyhow::Result<()> {
        let mut guard = self.ensure_session().await?;
        let session = guard
            .as_mut()
            .context("Codex account session unavailable")?;
        codex_account_request(session, 7, "account/logout", json!({})).await?;
        session.login = None;
        Ok(())
    }
}

impl CodexAccountSession {
    async fn cleanup(&mut self) {
        let _ = self.stdin.shutdown().await;
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

async fn codex_account_request(
    session: &mut CodexAccountSession,
    request_id: u64,
    method: &str,
    params: Value,
) -> anyhow::Result<Value> {
    codex_write_rpc(
        &mut session.stdin,
        json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        }),
    )
    .await?;
    codex_wait_for_response(&mut session.stdout, request_id).await
}

fn account_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .or_else(|| value.get("account").and_then(|account| account.get(key)))
        .and_then(Value::as_str)
        .map(str::to_string)
}
