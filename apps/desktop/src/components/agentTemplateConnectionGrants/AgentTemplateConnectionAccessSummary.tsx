import {
  Cable,
  CircleAlert,
  RefreshCw,
  ShieldCheck,
  Wrench,
} from "lucide-react";
import type { AgentTemplateConnectionAccessView } from "../../api/generated/desktop-http-v1.generated";
import { Badge, Button } from "../ui";

const MAX_EFFECTIVE_TOOL_NAMES = 12;

export function AgentTemplateConnectionAccessSummary({
  access,
  error,
  loading,
  onRetry,
}: {
  access: AgentTemplateConnectionAccessView | null;
  error: string | null;
  loading: boolean;
  onRetry(): void;
}) {
  return (
    <section className="agent-template-panel__connection-access">
      <header className="agent-template-panel__section-title">
        <ShieldCheck aria-hidden="true" size={14} />
        Connection 执行边界
        {access ? (
          <Badge variant={access.valid ? "success" : "danger"}>
            {access.valid ? "有效" : "已阻断"}
          </Badge>
        ) : null}
        {access?.mode === "legacy" ? (
          <Badge variant="warning">Legacy</Badge>
        ) : null}
      </header>

      {loading ? (
        <p className="agent-template-panel__empty" role="status">
          正在解析 Connection 访问边界…
        </p>
      ) : null}
      {error ? (
        <div
          className="agent-template-panel__connection-access-state is-error"
          role="alert"
        >
          <CircleAlert aria-hidden="true" size={16} />
          <span>{error}</span>
          <Button onClick={onRetry} size="compact" variant="quiet">
            <RefreshCw aria-hidden="true" size={14} /> 重试
          </Button>
        </div>
      ) : null}

      {access?.mode === "none" ? (
        <p className="agent-template-panel__empty">
          此模板没有外部 Connection 权限。
        </p>
      ) : null}
      {access?.mode === "legacy" ? (
        <div
          className="agent-template-panel__connection-access-state is-warning"
          role="note"
        >
          <CircleAlert aria-hidden="true" size={16} />
          <span>
            Legacy MCP 绑定没有固定到 operation
            revision。运行继续兼容，但新版本应显式迁移。
          </span>
        </div>
      ) : null}

      {access?.bindings.length ? (
        <div className="agent-template-panel__connection-bindings">
          {access.bindings.map((binding) => (
            <article key={binding.connectionId}>
              <header>
                <Cable aria-hidden="true" size={14} />
                <span>
                  <strong>
                    {binding.connectionName ?? binding.connectionId}
                  </strong>
                  <small>
                    r{binding.capabilityRevision} · {binding.operations.length}{" "}
                    operations
                  </small>
                </span>
                <Badge variant={binding.valid ? "success" : "danger"}>
                  {binding.valid ? "可执行" : "Fail closed"}
                </Badge>
              </header>
              {binding.operations.length ? (
                <ul className="agent-template-panel__connection-operations">
                  {binding.operations.map((operation) => (
                    <li key={operation.operationId}>
                      <Wrench aria-hidden="true" size={14} />
                      <span>
                        <strong>
                          {operation.displayName ??
                            operation.name ??
                            operation.operationId}
                        </strong>
                        <code>{operation.operationId}</code>
                        {operation.modelToolName ? (
                          <small>模型工具：{operation.modelToolName}</small>
                        ) : null}
                      </span>
                      <span>
                        {operation.permissionLabels.map((label) => (
                          <Badge key={label}>{label}</Badge>
                        ))}
                      </span>
                    </li>
                  ))}
                </ul>
              ) : null}
              {binding.issues.length ? (
                <IssueList issues={binding.issues} />
              ) : null}
            </article>
          ))}
        </div>
      ) : null}

      {access?.issues.length && access.bindings.length === 0 ? (
        <IssueList issues={access.issues} />
      ) : null}

      {access?.effectiveModelToolNames.length ? (
        <div className="agent-template-panel__effective-tools">
          <strong>有效模型工具</strong>
          <span>
            {access.effectiveModelToolNames
              .slice(0, MAX_EFFECTIVE_TOOL_NAMES)
              .map((toolName) => (
                <code key={toolName}>{toolName}</code>
              ))}
            {access.effectiveModelToolNames.length >
            MAX_EFFECTIVE_TOOL_NAMES ? (
              <small>
                +
                {access.effectiveModelToolNames.length -
                  MAX_EFFECTIVE_TOOL_NAMES}{" "}
                项
              </small>
            ) : null}
          </span>
        </div>
      ) : null}
    </section>
  );
}

function IssueList({
  issues,
}: {
  issues: AgentTemplateConnectionAccessView["issues"];
}) {
  return (
    <ul className="agent-template-panel__connection-issues">
      {issues.map((issue, index) => (
        <li
          className={issue.severity === "error" ? "is-error" : "is-warning"}
          key={`${issue.code}:${issue.connectionId ?? ""}:${issue.operationId ?? ""}:${index}`}
        >
          <CircleAlert aria-hidden="true" size={14} />
          <span>{issueMessage(issue.code, issue.message)}</span>
          <code>{issue.code}</code>
        </li>
      ))}
    </ul>
  );
}

function issueMessage(code: string, fallback: string): string {
  return (
    {
      legacy_mcp_server_grants:
        "Legacy MCP 绑定没有 operation 级授权，新版本应显式迁移。",
      connection_not_found: "Connection 已不存在。",
      integration_definition_not_found: "Connection 的 Provider 定义已不存在。",
      integration_definition_disabled: "Connection 的 Provider 定义已停用。",
      connection_disabled: "Connection 已停用。",
      connection_not_ready: "Connection 必须先通过健康测试。",
      legacy_auth_unverified: "迁移的 Legacy MCP 凭据无法独立验证。",
      connection_auth_unverified: "Connection 账号尚未验证。",
      capability_revision_not_found: "授权固定的能力修订已不存在。",
      active_capability_revision_not_found: "Connection 没有活动能力修订。",
      operation_not_in_pinned_revision: "授权操作不在固定能力修订中。",
      operation_removed: "授权操作已从活动能力修订中移除。",
      operation_descriptor_changed: "授权操作描述已变更，必须重新审阅。",
      operation_runtime_mismatch: "授权操作不属于当前 Connection runtime。",
      mcp_runtime_not_found: "Connection 的 MCP runtime 已不存在。",
      mcp_runtime_disabled: "Connection 的 MCP runtime 已停用。",
    }[code] ?? fallback
  );
}
