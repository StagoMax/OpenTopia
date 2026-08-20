import { useEffect, useState } from "react";
import { AlertCircle, RefreshCw } from "lucide-react";
import type { ApiClient } from "../../api/client";
import type { OfficeRuntimeStatus, ShellRuntimeStatus } from "../../types";
import { SettingsGroup, SettingsRow } from "../SettingsLayout";
import { Badge, Button, type BadgeVariant } from "../ui";

const statusPollMilliseconds = 1_500;

type StatusPresentation = {
  badge: string;
  badgeVariant: BadgeVariant;
  description: string;
};

export function ManagedRuntimeSettings({
  client,
  isWindows,
}: {
  client: ApiClient | null;
  isWindows: boolean;
}) {
  const [office, setOffice] = useState<OfficeRuntimeStatus | null>(null);
  const [powerShell, setPowerShell] = useState<ShellRuntimeStatus | null>(null);
  const [loading, setLoading] = useState<"office" | "powershell" | null>(null);
  const [requestError, setRequestError] = useState<string | null>(null);
  const [refreshRevision, setRefreshRevision] = useState(0);

  useEffect(() => {
    let cancelled = false;
    let timer: number | null = null;

    async function loadStatus() {
      if (!client) {
        if (!cancelled) setRequestError("OpenTopia 服务尚未连接。");
        return;
      }
      try {
        const health = await client.health();
        if (cancelled) return;
        setOffice(health.officeRuntime);
        setPowerShell(health.shellRuntime);
        setRequestError(null);
        if (
          health.officeRuntime.managedStatus === "downloading" ||
          (isWindows && health.shellRuntime.managedStatus === "downloading")
        ) {
          timer = window.setTimeout(loadStatus, statusPollMilliseconds);
        }
      } catch (error) {
        if (!cancelled) {
          setRequestError(
            error instanceof Error ? error.message : String(error),
          );
        }
      }
    }

    void loadStatus();
    return () => {
      cancelled = true;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [client, isWindows, refreshRevision]);

  async function retry(kind: "office" | "powershell") {
    if (!client || loading) return;
    setLoading(kind);
    setRequestError(null);
    try {
      if (kind === "office") {
        setOffice(await client.retryManagedOfficeRuntime());
      } else {
        setPowerShell(await client.retryManagedPowerShell());
      }
      setRefreshRevision((current) => current + 1);
    } catch (error) {
      setRequestError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoading(null);
    }
  }

  return (
    <>
      <OfficeRuntimeGroup
        status={office}
        loading={loading === "office"}
        clientAvailable={client !== null}
        requestError={requestError}
        onRetry={() => void retry("office")}
      />
      {isWindows ? (
        <PowerShellRuntimeGroup
          status={powerShell}
          loading={loading === "powershell"}
          clientAvailable={client !== null}
          requestError={requestError}
          onRetry={() => void retry("powershell")}
        />
      ) : null}
    </>
  );
}

function OfficeRuntimeGroup({
  status,
  loading,
  clientAvailable,
  requestError,
  onRetry,
}: {
  status: OfficeRuntimeStatus | null;
  loading: boolean;
  clientAvailable: boolean;
  requestError: string | null;
  onRetry(): void;
}) {
  const presentation = presentOfficeStatus(status);
  const canRetry =
    status?.managedStatus === "failed" || status?.managedStatus === "pending";
  const diagnostic = requestError ?? status?.managedError ?? null;
  const title = status?.runtime
    ? `Python ${status.runtime.pythonVersion} · openpyxl ${status.runtime.openpyxlVersion}`
    : "Office Python 尚未就绪";
  return (
    <SettingsGroup
      title="Office Python 运行时"
      description="OpenTopia 使用自带的独立 Python 处理 Office 文档，不依赖系统 Python。"
      actions={
        <Badge variant={presentation.badgeVariant}>{presentation.badge}</Badge>
      }
    >
      <SettingsRow
        title={title}
        description={presentation.description}
        control={
          canRetry ? (
            <RetryButton
              label="重试安装 Office Python"
              loading={loading}
              disabled={!clientAvailable}
              onClick={onRetry}
            />
          ) : undefined
        }
      />
      <RuntimeDiagnostic label="Office Python" diagnostic={diagnostic} />
    </SettingsGroup>
  );
}

function PowerShellRuntimeGroup({
  status,
  loading,
  clientAvailable,
  requestError,
  onRetry,
}: {
  status: ShellRuntimeStatus | null;
  loading: boolean;
  clientAvailable: boolean;
  requestError: string | null;
  onRetry(): void;
}) {
  const presentation = presentPowerShellStatus(status);
  const canRetry =
    status?.managedStatus === "failed" || status?.managedStatus === "pending";
  const diagnostic = requestError ?? status?.managedError ?? null;
  const shellName =
    status?.runtime.dialect === "power_shell7"
      ? "PowerShell 7"
      : "Windows PowerShell 5.1";
  const title = status?.runtime.version
    ? `${shellName} · ${status.runtime.version}`
    : shellName;
  return (
    <SettingsGroup
      title="PowerShell 运行时"
      description="OpenTopia 优先使用 PowerShell 7；安装失败时仍保留 Windows PowerShell 5.1 后备。"
      actions={
        <Badge variant={presentation.badgeVariant}>{presentation.badge}</Badge>
      }
    >
      <SettingsRow
        title={status ? title : "正在读取运行时状态…"}
        description={presentation.description}
        control={
          canRetry ? (
            <RetryButton
              label="重试安装 PowerShell 7"
              loading={loading}
              disabled={!clientAvailable}
              onClick={onRetry}
            />
          ) : undefined
        }
      />
      <RuntimeDiagnostic label="PowerShell" diagnostic={diagnostic} />
    </SettingsGroup>
  );
}

function RetryButton({
  label,
  loading,
  disabled,
  onClick,
}: {
  label: string;
  loading: boolean;
  disabled: boolean;
  onClick(): void;
}) {
  return (
    <Button
      size="compact"
      variant="secondary"
      disabled={disabled || loading}
      onClick={onClick}
    >
      <RefreshCw
        size={14}
        aria-hidden="true"
        className={loading ? "spin" : undefined}
      />
      {loading ? "正在重试…" : label}
    </Button>
  );
}

function RuntimeDiagnostic({
  label,
  diagnostic,
}: {
  label: string;
  diagnostic: string | null;
}) {
  return diagnostic ? (
    <div className="settings-danger-notice" role="alert">
      <AlertCircle size={16} aria-hidden="true" />
      <span>
        {label} 运行时操作失败：{diagnostic}
      </span>
    </div>
  ) : null;
}

function presentOfficeStatus(
  status: OfficeRuntimeStatus | null,
): StatusPresentation {
  if (!status) return loadingPresentation("正在检查 Office Python 运行时。");
  const runtimePath = status.runtime?.executable;
  switch (status.managedStatus) {
    case "ready":
      return successPresentation(`受管理运行时已启用 · ${runtimePath}`);
    case "not_required":
      return successPresentation(`发布包中的独立运行时已就绪 · ${runtimePath}`);
    case "downloading":
      return installingPresentation(
        `正在后台下载、校验并安装 ${status.managedVersion}。`,
      );
    case "pending":
      return pendingPresentation(
        `独立 Python ${status.managedVersion} 尚未安装。`,
      );
    case "disabled":
      return disabledPresentation(
        "Office Python 自动安装已禁用或当前平台不受支持。",
      );
    case "failed":
      return failedPresentation(
        "Office 功能会使用原生后备实现；可以手动重试安装。",
      );
  }
}

function presentPowerShellStatus(
  status: ShellRuntimeStatus | null,
): StatusPresentation {
  if (!status)
    return loadingPresentation("正在检查当前 Shell 与受管理运行时。");
  const runtimePath = status.runtime.program;
  switch (status.managedStatus) {
    case "ready":
      return successPresentation(
        `受管理的 PowerShell ${status.managedVersion} 已启用 · ${runtimePath}`,
      );
    case "not_required":
      return successPresentation(`已使用可用的 PowerShell 7 · ${runtimePath}`);
    case "downloading":
      return installingPresentation(
        `正在后台安装 PowerShell ${status.managedVersion}；当前继续使用 ${runtimePath}`,
      );
    case "pending":
      return pendingPresentation(
        `PowerShell ${status.managedVersion} 尚未安装；当前使用 ${runtimePath}`,
      );
    case "disabled":
      return disabledPresentation(`自动安装已禁用；当前使用 ${runtimePath}`);
    case "failed":
      return failedPresentation(
        `当前继续使用 ${runtimePath}，可以手动重试安装。`,
      );
  }
}

function loadingPresentation(description: string): StatusPresentation {
  return { badge: "读取中", badgeVariant: "neutral", description };
}

function successPresentation(description: string): StatusPresentation {
  return { badge: "已就绪", badgeVariant: "success", description };
}

function installingPresentation(description: string): StatusPresentation {
  return { badge: "安装中", badgeVariant: "info", description };
}

function pendingPresentation(description: string): StatusPresentation {
  return { badge: "等待安装", badgeVariant: "warning", description };
}

function disabledPresentation(description: string): StatusPresentation {
  return { badge: "自动安装已关闭", badgeVariant: "neutral", description };
}

function failedPresentation(description: string): StatusPresentation {
  return { badge: "安装失败", badgeVariant: "danger", description };
}
