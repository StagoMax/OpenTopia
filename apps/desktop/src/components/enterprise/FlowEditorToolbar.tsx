import {
  FlaskConical,
  Play,
  Plus,
  RefreshCw,
  Send,
  ShieldCheck,
} from "lucide-react";
import { Badge, Button } from "../ui";

export function FlowEditorToolbar({
  activeTestRun,
  busy,
  canActivate,
  canCreateDraft,
  canDryRun,
  canTestRun,
  draftExists,
  flowId,
  name,
  nodeCount,
  onActivate,
  onCreateDraft,
  onDryRun,
  onRefresh,
  onTestRun,
  onValidate,
  threadReady,
  validated,
}: {
  activeTestRun: boolean;
  busy: string | null;
  canActivate: boolean;
  canCreateDraft: boolean;
  canDryRun: boolean;
  canTestRun: boolean;
  draftExists: boolean;
  flowId: string;
  name: string;
  nodeCount: number;
  onActivate(): void;
  onCreateDraft(): void;
  onDryRun(): void;
  onRefresh(): void;
  onTestRun(): void;
  onValidate(): void;
  threadReady: boolean;
  validated: boolean;
}) {
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
              ? `${nodeCount} 个节点 · ${flowId}`
              : "请先新建或选择一个 Flow 任务以保存草稿"}
          </small>
        </span>
      </span>
      <div className="workflow-editor__actions">
        <Button
          disabled={!canCreateDraft || Boolean(busy)}
          onClick={onCreateDraft}
          size="compact"
          variant={draftExists ? "secondary" : "primary"}
        >
          <Plus aria-hidden="true" size={14} />
          {busy === "create" ? "创建中…" : "创建草稿"}
        </Button>
        <Button
          disabled={!draftExists || Boolean(busy)}
          onClick={onValidate}
          size="compact"
        >
          <ShieldCheck aria-hidden="true" size={14} /> 验证
        </Button>
        <Button
          disabled={!canDryRun || Boolean(busy)}
          onClick={onDryRun}
          size="compact"
        >
          <Play aria-hidden="true" size={14} /> Dry Run
        </Button>
        <Button
          disabled={!canTestRun || Boolean(busy) || activeTestRun}
          onClick={onTestRun}
          size="compact"
        >
          <FlaskConical aria-hidden="true" size={14} />
          {activeTestRun ? "测试运行中…" : "Test Run"}
        </Button>
        <Button
          disabled={!draftExists || Boolean(busy)}
          onClick={onRefresh}
          size="compact"
          variant="quiet"
        >
          <RefreshCw aria-hidden="true" size={14} /> 刷新
        </Button>
        <Button
          disabled={!canActivate || Boolean(busy)}
          onClick={onActivate}
          size="compact"
          variant={canActivate ? "primary" : "secondary"}
        >
          <Send aria-hidden="true" size={14} /> 激活 Flow
        </Button>
      </div>
    </header>
  );
}
