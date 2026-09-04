import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import { memo } from "react";
import {
  BadgeCheck,
  Bot,
  Braces,
  CheckCircle2,
  CircleDot,
  GitBranch,
  Inbox,
  LoaderCircle,
  Merge,
  PauseCircle,
  RadioTower,
  Repeat2,
  Sparkles,
  ShieldCheck,
  Trash2,
  Wrench,
  XCircle,
} from "lucide-react";
import { IconButton } from "../ui";
import type { FlowNodeKind, FlowNodeRun } from "../../types";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import {
  interfaceMessage,
  type ApplicationLanguage,
} from "../../applicationLanguage";

export type WorkflowCanvasRunStatus = FlowNodeRun["status"] | "ready";

export type WorkflowCanvasNodeData = Record<string, unknown> & {
  activationText: string;
  disabled: boolean;
  kind: FlowNodeKind;
  label: string;
  onEditTrigger(nodeId: string): void;
  onRemove(nodeId: string): void;
  onSelect(nodeId: string): void;
  readOnly: boolean;
  runStatus?: WorkflowCanvasRunStatus;
  subtitle: string;
};

export type WorkflowCanvasNodeType = Node<
  WorkflowCanvasNodeData,
  "workflowNode"
>;

export const WorkflowCanvasNode = memo(function WorkflowCanvasNode({
  data,
  id,
  selected,
}: NodeProps<WorkflowCanvasNodeType>) {
  const { t } = useApplicationLanguage();
  const {
    activationText,
    disabled,
    kind,
    label,
    onEditTrigger,
    onRemove,
    onSelect,
    readOnly,
    runStatus,
    subtitle,
  } = data;
  const NodeIcon = nodeIcon(kind);
  const canConnect = !readOnly && !disabled;

  return (
    <article
      aria-label={`${label}，${kind} ${t("flow.canvas.node")}`}
      className={`workflow-node workflow-node--${kind}${
        selected ? " is-selected" : ""
      }${runStatus ? ` is-run-${runStatus}` : ""}`}
    >
      <Handle
        aria-label={`${t("flow.canvas.connectTo")} ${label}`}
        className="workflow-node__handle workflow-node__handle--target"
        id="input"
        isConnectable={canConnect}
        position={Position.Left}
        type="target"
      />
      {kind === "output" || readOnly ? (
        <div className="workflow-node__trigger">
          <RadioTower aria-hidden="true" size={14} />
          <span>
            <small>{t("flow.canvas.input")}</small>
            <strong title={activationText}>{activationText}</strong>
          </span>
        </div>
      ) : (
        <button
          className="workflow-node__trigger nodrag nopan"
          disabled={disabled}
          onClick={() => onEditTrigger(id)}
          type="button"
        >
          <RadioTower aria-hidden="true" size={14} />
          <span>
            <small>{t("flow.canvas.trigger")}</small>
            <strong title={activationText}>{activationText}</strong>
          </span>
        </button>
      )}
      <button
        aria-pressed={selected}
        className="workflow-node__body"
        onClick={() => onSelect(id)}
        type="button"
      >
        <NodeIcon aria-hidden="true" size={17} />
        <span>
          <strong title={label}>{label}</strong>
          <small title={subtitle}>{subtitle}</small>
        </span>
        {runStatus ? (
          <NodeRunStatus status={runStatus} />
        ) : kind === "agent" ? (
          <Braces aria-hidden="true" size={14} />
        ) : null}
      </button>
      <footer className="workflow-node__final">
        <CircleDot aria-hidden="true" size={12} />
        <span>
          {kind === "output"
            ? t("flow.canvas.output")
            : t("flow.canvas.final")}
        </span>
        {!readOnly && kind !== "output" ? (
          <IconButton
            aria-label={`${t("flow.canvas.remove")} ${label}`}
            className="nodrag nopan"
            disabled={disabled}
            onClick={(event) => {
              event.stopPropagation();
              onRemove(id);
            }}
            size="compact"
            variant="danger"
          >
            <Trash2 aria-hidden="true" size={13} />
          </IconButton>
        ) : null}
      </footer>
      {kind !== "output" ? (
        <Handle
          aria-label={`${t("flow.canvas.connectFrom")} ${label}`}
          className="workflow-node__handle workflow-node__handle--source"
          id="final"
          isConnectable={canConnect}
          position={Position.Right}
          type="source"
        />
      ) : null}
    </article>
  );
}, workflowCanvasNodePropsEqual);

function NodeRunStatus({ status }: { status: WorkflowCanvasRunStatus }) {
  const { language } = useApplicationLanguage();
  const Icon =
    status === "succeeded"
      ? CheckCircle2
      : status === "failed" || status === "cancelled"
        ? XCircle
        : status === "waiting_approval" || status === "waiting_human"
          ? PauseCircle
          : LoaderCircle;
  const label = runStatusLabel(status, language);
  return (
    <span className="workflow-node__run-status" title={label}>
      <Icon aria-hidden="true" size={14} />
      <span>{label}</span>
    </span>
  );
}

function runStatusLabel(
  status: WorkflowCanvasRunStatus,
  language: ApplicationLanguage,
) {
  if (status === "ready")
    return interfaceMessage(language, "flow.nodeTest.ready");
  if (status === "succeeded")
    return interfaceMessage(language, "flow.nodeStatus.succeeded");
  if (status === "failed")
    return interfaceMessage(language, "flow.nodeStatus.failed");
  if (status === "cancelled")
    return interfaceMessage(language, "flow.canvas.statusCancelled");
  if (status === "waiting_approval" || status === "waiting_human")
    return interfaceMessage(language, "flow.nodeTest.waitingHuman");
  if (status === "resuming")
    return interfaceMessage(language, "flow.nodeStatus.resuming");
  return interfaceMessage(language, "flow.nodeStatus.running");
}

function workflowCanvasNodePropsEqual(
  previous: NodeProps<WorkflowCanvasNodeType>,
  next: NodeProps<WorkflowCanvasNodeType>,
) {
  // React Flow moves the wrapper with a transform. The card itself only needs
  // to render again when its visible data or selection state changes.
  return (
    previous.id === next.id &&
    previous.selected === next.selected &&
    previous.data === next.data
  );
}

function nodeIcon(kind: FlowNodeKind) {
  if (kind === "agent") return Bot;
  if (kind === "skill") return Sparkles;
  if (kind === "tool") return Wrench;
  if (kind === "condition") return GitBranch;
  if (kind === "validator") return BadgeCheck;
  if (kind === "approval") return ShieldCheck;
  if (kind === "join") return Merge;
  if (kind === "loop") return Repeat2;
  return Inbox;
}
