import { useState } from "react";
import { Check, ExternalLink, FileText, Shield } from "lucide-react";
import { openExternal } from "../../platform";
import { providerDisplayName } from "../../providerSettings";
import type {
  AppSettings,
  CodexAccountStatus,
  CodexLoginStart,
  ProviderHealth,
  ProviderSettings,
  WindowsSandboxSetupStatus,
} from "../../types";
import { Badge, Button, Panel } from "../ui";
import { SettingsGroup, SettingsPage, SettingsRow } from "../SettingsLayout";

export function PermissionSettings({
  permissionMode,
  sandbox,
  isWindows,
  onPermissionModeChange,
  onSandboxChange,
  windowsSetup,
  windowsSetupBusy,
  windowsSetupError,
  onSetupWindowsSandbox,
  onRemoveWindowsSandbox,
}: {
  permissionMode: "chat" | "read_only" | "auto" | "approve" | "full_access";
  sandbox: AppSettings["sandbox"];
  isWindows: boolean;
  onPermissionModeChange(mode: "auto" | "approve" | "full_access"): void;
  onSandboxChange(settings: AppSettings["sandbox"]): void;
  windowsSetup: WindowsSandboxSetupStatus | null;
  windowsSetupBusy: boolean;
  windowsSetupError: string | null;
  onSetupWindowsSandbox(): Promise<WindowsSandboxSetupStatus>;
  onRemoveWindowsSandbox(): Promise<WindowsSandboxSetupStatus>;
}) {
  async function configureWindowsSandbox() {
    try {
      await onSetupWindowsSandbox();
    } catch {
      return;
    }
  }

  async function removeWindowsSandbox() {
    if (
      !window.confirm(
        "移除会删除 OpenTopia 的隔离账户、离线网络规则和已记录的目录权限。继续吗？",
      )
    ) {
      return;
    }
    try {
      await onRemoveWindowsSandbox();
    } catch {
      return;
    }
  }

  return (
    <SettingsPage title="权限" description="控制工具调用的审批与系统访问范围。">
      <SettingsGroup title="审批策略">
        <div className="settings-permission-options">
          {(
            [
              ["approve", "请求批准", "每次高风险操作前等待确认。"],
              ["auto", "自动审批", "按策略自动处理常规权限请求。"],
              ["full_access", "完全访问", "关闭系统沙箱并允许网络访问。"],
            ] as const
          ).map(([id, title, description]) => (
            <button
              key={id}
              type="button"
              className={permissionMode === id ? "active" : ""}
              aria-pressed={permissionMode === id}
              onClick={() => {
                if (
                  id === "full_access" &&
                  !window.confirm(
                    "完全访问会允许命令访问当前用户可用的文件和网络。确定继续吗？",
                  )
                ) {
                  return;
                }
                onPermissionModeChange(id);
              }}
            >
              <span>{permissionMode === id ? <Check size={15} /> : null}</span>
              <strong>{title}</strong>
              <small>{description}</small>
            </button>
          ))}
        </div>
      </SettingsGroup>

      <SettingsGroup title="沙箱">
        {isWindows && windowsSetup === null ? (
          <div
            className="settings-warning-notice settings-sandbox-status"
            role="status"
          >
            <Shield size={16} />
            <span>
              {windowsSetupBusy
                ? "正在读取 Windows 强制沙箱状态…"
                : "尚未读取到 Windows 强制沙箱状态，可从这里重新配置。"}
            </span>
            <Button
              size="compact"
              variant="secondary"
              disabled={windowsSetupBusy}
              onClick={() => void configureWindowsSandbox()}
            >
              {windowsSetupBusy ? "读取中…" : "配置或修复"}
            </Button>
          </div>
        ) : null}
        {isWindows && windowsSetup?.state === "not_configured" ? (
          <div
            className="settings-warning-notice settings-sandbox-status"
            role="status"
          >
            <Shield size={16} />
            <span>
              Windows 强制沙箱尚未配置。点击后会出现标准 UAC
              授权窗口，并创建两个隔离的普通用户。
            </span>
            <Button
              size="compact"
              variant="secondary"
              disabled={windowsSetupBusy || !windowsSetup.helperAvailable}
              onClick={() => void configureWindowsSandbox()}
            >
              {windowsSetupBusy ? "配置中…" : "配置强制沙箱"}
            </Button>
          </div>
        ) : null}
        {isWindows && windowsSetup?.state === "degraded" ? (
          <div
            className="settings-danger-notice settings-sandbox-status"
            role="alert"
          >
            <Shield size={16} />
            <span>
              Windows 沙箱配置不完整：
              {windowsSetup.issues.join("；") || "组件健康检查未通过"}
            </span>
            <Button
              size="compact"
              variant="secondary"
              disabled={windowsSetupBusy || !windowsSetup.helperAvailable}
              onClick={() => void configureWindowsSandbox()}
            >
              {windowsSetupBusy ? "处理中…" : "修复"}
            </Button>
            <Button
              size="compact"
              variant="danger"
              disabled={windowsSetupBusy || !windowsSetup.helperAvailable}
              onClick={() => void removeWindowsSandbox()}
            >
              移除
            </Button>
          </div>
        ) : null}
        {isWindows && windowsSetup?.state === "ready" ? (
          <div
            className="settings-success-notice settings-sandbox-status"
            role="status"
          >
            <Shield size={16} />
            <span>Windows 专用账户沙箱已就绪，运行工具时不需要再次授权。</span>
            <Button
              size="compact"
              variant="danger"
              disabled={windowsSetupBusy}
              onClick={() => void removeWindowsSandbox()}
            >
              {windowsSetupBusy ? "移除中…" : "移除"}
            </Button>
          </div>
        ) : null}
        {isWindows && windowsSetup?.state === "unavailable" ? (
          <div
            className="settings-danger-notice settings-sandbox-status"
            role="alert"
          >
            <Shield size={16} />
            <span>
              Windows 沙箱不可用：
              {windowsSetup.issues.join("；") || "未找到兼容的沙箱组件"}
            </span>
          </div>
        ) : null}
        {isWindows && windowsSetupError ? (
          <div
            className="settings-danger-notice settings-sandbox-status"
            role="alert"
          >
            <Shield size={16} />
            <span>管理 Windows 沙箱失败：{windowsSetupError}</span>
          </div>
        ) : null}
        <div className="settings-form-grid settings-sandbox-grid">
          <label>
            <span>访问模式</span>
            <select
              value={sandbox.sandboxMode}
              onChange={(event) => {
                const sandboxMode = event.target
                  .value as AppSettings["sandbox"]["sandboxMode"];
                const danger = sandboxMode === "danger-full-access";
                onSandboxChange({
                  ...sandbox,
                  sandboxMode,
                  enforcement: danger
                    ? "disabled"
                    : sandbox.enforcement === "disabled"
                      ? "enforce"
                      : sandbox.enforcement,
                  network: danger ? "allow" : sandbox.network,
                });
              }}
            >
              <option value="read-only">只读</option>
              <option value="workspace-write">工作区可写</option>
              <option value="danger-full-access">完整系统访问</option>
            </select>
          </label>
          <label>
            <span>系统隔离</span>
            <select
              value={sandbox.enforcement}
              disabled={sandbox.sandboxMode === "danger-full-access"}
              onChange={(event) =>
                onSandboxChange({
                  ...sandbox,
                  enforcement: event.target
                    .value as AppSettings["sandbox"]["enforcement"],
                })
              }
            >
              <option value="enforce">强制</option>
              <option value="best-effort">尽力执行</option>
              <option value="disabled">关闭</option>
            </select>
          </label>
          <label>
            <span>网络</span>
            <select
              value={sandbox.network}
              disabled={sandbox.sandboxMode === "danger-full-access"}
              onChange={(event) =>
                onSandboxChange({
                  ...sandbox,
                  network: event.target
                    .value as AppSettings["sandbox"]["network"],
                })
              }
            >
              <option value="deny">拒绝</option>
              <option value="inherit">继承</option>
              <option value="allow">允许</option>
            </select>
          </label>
          <label className="settings-field-wide">
            <span>额外可写目录</span>
            <textarea
              rows={3}
              value={sandbox.writableRoots.join("\n")}
              placeholder="每行一个绝对路径"
              onChange={(event) =>
                onSandboxChange({
                  ...sandbox,
                  writableRoots: parsePathList(event.target.value),
                })
              }
            />
          </label>
          <label className="settings-field-wide">
            <span>额外可读路径</span>
            <textarea
              rows={3}
              value={sandbox.readPaths.join("\n")}
              placeholder="每行一个绝对路径"
              onChange={(event) =>
                onSandboxChange({
                  ...sandbox,
                  readPaths: parsePathList(event.target.value),
                })
              }
            />
          </label>
        </div>
        {sandbox.sandboxMode === "danger-full-access" ||
        sandbox.enforcement === "disabled" ? (
          <div className="settings-danger-notice" role="status">
            <Shield size={16} />
            系统沙箱已关闭，命令可访问当前用户有权访问的文件与网络。
          </div>
        ) : sandbox.enforcement === "best-effort" ? (
          <div className="settings-warning-notice" role="status">
            <Shield size={16} />
            尽力执行模式在隔离后端不可用时可能降级运行。
          </div>
        ) : null}
      </SettingsGroup>
    </SettingsPage>
  );
}

export function CodexAccountSettings({
  account,
  loading,
  error,
  onRefresh,
  onStartLogin,
  onCancelLogin,
  onLogout,
}: {
  account: CodexAccountStatus | null;
  loading: boolean;
  error: string | null;
  onRefresh(): void;
  onStartLogin(): Promise<CodexLoginStart | null>;
  onCancelLogin(): Promise<void>;
  onLogout(): Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const loginUrl = account?.verificationUrl ?? account?.authUrl ?? null;

  async function startLogin() {
    setBusy(true);
    try {
      const login = await onStartLogin();
      const url = login?.verificationUrl ?? login?.authUrl;
      if (url) void openExternal(url);
    } finally {
      setBusy(false);
    }
  }

  async function cancelLogin() {
    setBusy(true);
    try {
      await onCancelLogin();
    } finally {
      setBusy(false);
    }
  }

  async function logout() {
    setBusy(true);
    try {
      await onLogout();
    } finally {
      setBusy(false);
    }
  }

  return (
    <Panel
      className="settings-codex-account"
      title="ChatGPT / Codex 账号"
      actions={
        <Button
          size="compact"
          variant="quiet"
          disabled={loading || busy}
          onClick={onRefresh}
        >
          刷新
        </Button>
      }
    >
      <div className="settings-codex-account-status">
        <Badge variant={account?.loggedIn ? "success" : "warning"}>
          {account?.loggedIn ? "已登录" : "未登录"}
        </Badge>
        {account?.planType ? <span>{account.planType}</span> : null}
        {account?.email ? <span>{account.email}</span> : null}
        {account?.rateLimits ? <span>额度已同步</span> : null}
        {account?.usage ? <span>用量已同步</span> : null}
      </div>

      {account?.loginPending ? (
        <div className="settings-codex-login-instructions">
          <strong>请完成 ChatGPT 登录</strong>
          {account.userCode ? <code>{account.userCode}</code> : null}
          {loginUrl ? (
            <Button
              size="compact"
              variant="primary"
              disabled={busy}
              onClick={() => void openExternal(loginUrl)}
            >
              打开登录页面
              <ExternalLink size={14} aria-hidden="true" />
            </Button>
          ) : null}
          <Button
            size="compact"
            variant="quiet"
            disabled={busy}
            onClick={() => void cancelLogin()}
          >
            取消
          </Button>
        </div>
      ) : account?.loggedIn ? (
        <Button
          size="compact"
          variant="danger"
          disabled={busy}
          onClick={() => void logout()}
        >
          退出登录
        </Button>
      ) : (
        <Button
          size="compact"
          variant="primary"
          disabled={busy || loading}
          onClick={() => void startLogin()}
        >
          登录 ChatGPT 账号
        </Button>
      )}

      {error ? (
        <p className="settings-codex-account-error" role="alert">
          {error}
        </p>
      ) : null}
      <small className="settings-codex-account-hint">
        登录后，Codex Provider 会使用该账号可用的 ChatGPT/Codex 额度；API Key
        模式仍使用 API 平台额度。
      </small>
    </Panel>
  );
}

export function AdvancedSettings({
  providers,
  providerHealth,
  onOpenLogs,
}: {
  providers: ProviderSettings[];
  providerHealth: ProviderHealth[];
  onOpenLogs(): void;
}) {
  return (
    <SettingsPage title="高级" description="检查模型连接状态并打开诊断信息。">
      <SettingsGroup title="供应商连接">
        {providers.map((provider) => {
          const displayName = providerDisplayName(provider);
          const health = providerHealth.find((item) => item.id === provider.id);
          return (
            <SettingsRow
              key={provider.id}
              title={displayName}
              description={`${provider.model} · ${health?.status ?? "未检测"}`}
            />
          );
        })}
      </SettingsGroup>
      <SettingsGroup title="诊断">
        <SettingsRow
          title="应用日志"
          description="查看主进程、服务与崩溃日志。"
          control={
            <button
              type="button"
              className="secondary-button"
              onClick={onOpenLogs}
            >
              <FileText size={15} />
              打开日志
            </button>
          }
        />
      </SettingsGroup>
    </SettingsPage>
  );
}

function parsePathList(value: string): string[] {
  return [
    ...new Set(
      value
        .split(/\r?\n/)
        .map((path) => path.trim())
        .filter(Boolean),
    ),
  ];
}
