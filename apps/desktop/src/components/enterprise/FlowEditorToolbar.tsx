import { FlaskConical, Plus, RefreshCw, Send, ShieldCheck } from "lucide-react";
import { Badge, Button } from "../ui";
import { nextFlowEditorStage } from "./flowEditorProgress";

export function FlowEditorToolbar({
  activeTestRun,
  busy,
  canActivate,
  canCreateDraft,
  canTestRun,
  draftExists,
  flowId,
  name,
  nodeCount,
  onActivate,
  onCreateDraft,
  onRefresh,
  onTestRun,
  onValidate,
  successfulTestRun,
  threadReady,
  validated,
}: {
  activeTestRun: boolean;
  busy: string | null;
  canActivate: boolean;
  canCreateDraft: boolean;
  canTestRun: boolean;
  draftExists: boolean;
  flowId: string;
  name: string;
  nodeCount: number;
  onActivate(): void;
  onCreateDraft(): void;
  onRefresh(): void;
  onTestRun(): void;
  onValidate(): void;
  successfulTestRun: boolean;
  threadReady: boolean;
  validated: boolean;
}) {
  const stage = nextFlowEditorStage({
    draftExists,
    successfulTestRun,
    validated,
  });
  const nextAction =
    stage === "save"
      ? {
          disabled: !canCreateDraft,
          icon: Plus,
          label: busy === "create" ? "创建中…" : "保存草稿",
          onClick: onCreateDraft,
        }
      : stage === "validate"
        ? {
            disabled: false,
            icon: ShieldCheck,
            label: busy === "validate" ? "验证中…" : "验证",
            onClick: onValidate,
          }
        : stage === "test"
          ? {
              disabled: !canTestRun || activeTestRun,
              icon: FlaskConical,
              label: busy === "test-run" ? "启动中…" : "Test Run",
              onClick: onTestRun,
            }
          : {
              disabled: !canActivate,
              icon: Send,
              label: busy === "activate" ? "激活中…" : "激活 Flow",
              onClick: onActivate,
            };
  const NextIcon = nextAction.icon;

  return (
    <header className="workflow-editor__toolbar">
      <span className="workflow-editor__summary">
        <Badge
          variant={validated ? "success" : draftExists ? "neutral" : "warning"}
        >
          {activeTestRun
            ? "Testing"
            : validated
              ? "Validated"
              : draftExists
                ? "Draft"
                : "Unsaved"}
        </Badge>
        <span>
          <strong>{name || "Untitled Flow"}</strong>
          <small>
            {threadReady
              ? `${nodeCount} 个步骤 · ${flowId}`
              : "请先新建或选择一个 Flow 任务以保存草稿"}
          </small>
        </span>
      </span>
      <div className="workflow-editor__actions">
        <Button
          disabled={!draftExists || Boolean(busy)}
          onClick={onRefresh}
          size="compact"
          variant="quiet"
        >
          <RefreshCw aria-hidden="true" size={14} /> 刷新
        </Button>
        <Button
          disabled={nextAction.disabled || Boolean(busy) || activeTestRun}
          onClick={nextAction.onClick}
          size="compact"
          variant="primary"
        >
          <NextIcon aria-hidden="true" size={14} /> {nextAction.label}
        </Button>
      </div>
    </header>
  );
}
