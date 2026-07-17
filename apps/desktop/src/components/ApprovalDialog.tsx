import { useCallback, useEffect, useState } from "react";
import {
  AlertTriangle,
  Globe2,
  Loader2,
  ShieldAlert,
  TerminalSquare,
} from "lucide-react";
import type { AgentEvent } from "../types";
import "./ApprovalDialog.css";

export type ApprovalRequest = Extract<
  AgentEvent["payload"],
  { type: "approval_requested" }
>;

type ApprovalDialogProps = {
  request: ApprovalRequest;
  queuePosition: number;
  queueLength: number;
  isSubmitting: boolean;
  error: string | null;
  onDecision(approved: boolean): void;
};

type RiskPresentation = {
  label: string;
  description: string;
  level: "medium" | "high";
  icon: typeof ShieldAlert;
};

function isContinuationAction(action: string): boolean {
  return action.trim().toLowerCase() === "continue agent execution";
}

function actionKind(action: string): string {
  const normalized = action.trimStart().toLowerCase();
  const firstLine = normalized.split(/\r?\n/, 1)[0] ?? "";
  return firstLine.split(/\s+/, 1)[0] ?? "";
}

function describeRisk(action: string): RiskPresentation {
  const kind = actionKind(action);

  if (isContinuationAction(action)) {
    return {
      label: "继续执行",
      description: "当前任务已完成一个执行阶段，需要确认是否继续下一阶段。",
      level: "medium",
      icon: ShieldAlert,
    };
  }

  if (kind.startsWith("browser:") || kind.startsWith("network:")) {
    return {
      label: "网络访问",
      description: "该操作将访问外部网络或向外部服务发送请求。",
      level: "medium",
      icon: Globe2,
    };
  }

  if (
    kind === "/run" ||
    kind.startsWith("shell:") ||
    kind.startsWith("command:") ||
    kind.startsWith("terminal:") ||
    kind.startsWith("execute:")
  ) {
    return {
      label: "高风险命令",
      description: "运行该命令可能读取或修改文件，也可能启动本机进程。",
      level: "high",
      icon: TerminalSquare,
    };
  }

  if (
    kind === "/write" ||
    kind === "/patch" ||
    kind === "/read" ||
    kind === "/list" ||
    kind === "/search"
  ) {
    const readOnly = kind === "/read" || kind === "/list";
    return {
      label: readOnly ? "工作区外读取" : "工作区外文件访问",
      description: readOnly
        ? "该操作将读取当前自动授权范围之外的文件或目录。"
        : "该操作可能读取或修改当前自动授权范围之外的文件。",
      level: "high",
      icon: ShieldAlert,
    };
  }

  return {
    label: "受限操作",
    description: "该操作超出了当前自动授权范围，需要你明确确认。",
    level: "high",
    icon: ShieldAlert,
  };
}

function displayReason(request: ApprovalRequest): string {
  if (isContinuationAction(request.action)) {
    if (/context budget/i.test(request.reason)) {
      return "当前执行阶段的上下文额度已使用完。继续后，Agent 会整理现有上下文并进入下一阶段。";
    }
    if (/tool-decision budget/i.test(request.reason)) {
      return "当前执行阶段的工具调用额度已使用完。继续后，Agent 会保留任务进度并进入下一阶段。";
    }
    return "当前执行阶段已经结束。继续后，Agent 会保留任务进度并开始下一阶段。";
  }

  const reason = request.reason.replace(/^approval required:\s*/i, "").trim();
  const pathReason =
    /^(reading outside the workspace|writing outside the workspace|write requires approval):\s*(.+)$/i.exec(
      reason,
    );
  if (pathReason) {
    const readOnly = /^reading/i.test(pathReason[1]);
    return `${readOnly ? "该操作需要读取" : "该操作需要写入"}当前工作区之外或受保护的位置：${pathReason[2]}`;
  }
  const commandReason = /^command requires approval:\s*(.+)$/i.exec(reason);
  if (commandReason) return `该命令需要你确认后才能运行：${commandReason[1]}`;
  const networkReason = /^network access requires approval:\s*(.+)$/i.exec(
    reason,
  );
  if (networkReason) return `该操作需要访问网络目标：${networkReason[1]}`;
  if (/blocked by the sandbox/i.test(reason)) {
    return "该操作超出了当前沙箱允许的范围，需要你确认后才能继续。";
  }
  return reason;
}

function displayAction(action: string): string {
  return isContinuationAction(action) ? "继续执行当前任务" : action;
}

function isCommandAction(action: string): boolean {
  const kind = actionKind(action);
  return (
    kind === "/run" ||
    kind.startsWith("shell:") ||
    kind.startsWith("command:") ||
    kind.startsWith("terminal:") ||
    kind.startsWith("execute:")
  );
}

export function ApprovalDialog({
  request,
  queuePosition,
  queueLength,
  isSubmitting,
  error,
  onDecision,
}: ApprovalDialogProps) {
  const [decisionRequested, setDecisionRequested] = useState(false);
  const risk = describeRisk(request.action);
  const RiskIcon = risk.icon;
  const commandAction = isCommandAction(request.action);
  const continuationAction = isContinuationAction(request.action);
  const decisionPending = isSubmitting || decisionRequested;

  useEffect(() => {
    if (!isSubmitting && error) setDecisionRequested(false);
  }, [error, isSubmitting]);

  const submitDecision = useCallback(
    (approved: boolean) => {
      if (decisionPending) return;
      setDecisionRequested(true);
      onDecision(approved);
    },
    [decisionPending, onDecision],
  );

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || decisionPending) return;
      event.preventDefault();
      event.stopPropagation();
      submitDecision(false);
    };

    document.addEventListener("keydown", handleKeyDown, true);
    return () => document.removeEventListener("keydown", handleKeyDown, true);
  }, [decisionPending, request.approval_id, submitDecision]);

  return (
    <aside
      className="approval-dialog"
      role="region"
      aria-live="assertive"
      aria-labelledby="approval-dialog-title"
      aria-describedby="approval-dialog-description"
    >
      <header className="approval-dialog-header">
        <span className="approval-dialog-icon" aria-hidden="true">
          <AlertTriangle size={17} />
        </span>
        <div className="approval-dialog-heading">
          <h2 id="approval-dialog-title">
            {continuationAction
              ? "是否继续执行当前任务？"
              : commandAction
                ? "是否允许运行这个命令？"
                : "是否允许执行这个操作？"}
          </h2>
          <p id="approval-dialog-description">
            <strong>{risk.label}</strong>
            <span>{risk.description}</span>
          </p>
        </div>
        {queueLength > 1 && (
          <span
            className="approval-dialog-queue"
            aria-label={`审批队列 ${queuePosition}/${queueLength}`}
          >
            {queuePosition}/{queueLength}
          </span>
        )}
      </header>

      <section className="approval-dialog-reason" aria-label="请求原因">
        <RiskIcon size={15} aria-hidden="true" />
        <p>{displayReason(request)}</p>
      </section>

      <pre className="approval-dialog-action" aria-label="完整命令或操作">
        <code>{displayAction(request.action)}</code>
      </pre>

      {error && (
        <p className="approval-dialog-error" role="alert">
          {error}
        </p>
      )}

      <footer className="approval-dialog-actions">
        <button
          type="button"
          className="approval-dialog-deny"
          disabled={decisionPending}
          onClick={() => submitDecision(false)}
        >
          跳过 <kbd>Esc</kbd>
        </button>
        <button
          type="button"
          className="approval-dialog-allow"
          disabled={decisionPending}
          onClick={() => submitDecision(true)}
        >
          {decisionPending ? (
            <>
              <Loader2
                className="approval-dialog-spinner"
                size={15}
                aria-hidden="true"
              />
              正在提交
            </>
          ) : continuationAction ? (
            "继续"
          ) : commandAction ? (
            "运行"
          ) : (
            "允许"
          )}
        </button>
      </footer>
    </aside>
  );
}
