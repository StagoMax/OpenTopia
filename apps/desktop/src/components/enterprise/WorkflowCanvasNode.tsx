import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import {
  BadgeCheck,
  Bot,
  Braces,
  CircleDot,
  GitBranch,
  Inbox,
  Merge,
  RadioTower,
  Repeat2,
  Sparkles,
  ShieldCheck,
  Trash2,
  Wrench,
} from "lucide-react";
import { IconButton } from "../ui";
import type { FlowNodeKind } from "../../types";

export type WorkflowCanvasNodeData = Record<string, unknown> & {
  activationText: string;
  disabled: boolean;
  kind: FlowNodeKind;
  label: string;
  onEditTrigger(nodeId: string): void;
  onRemove(nodeId: string): void;
  onSelect(nodeId: string): void;
  readOnly: boolean;
  subtitle: string;
};

export type WorkflowCanvasNodeType = Node<
  WorkflowCanvasNodeData,
  "workflowNode"
>;

export function WorkflowCanvasNode({
  data,
  id,
  selected,
}: NodeProps<WorkflowCanvasNodeType>) {
  const {
    activationText,
    disabled,
    kind,
    label,
    onEditTrigger,
    onRemove,
    onSelect,
    readOnly,
    subtitle,
  } = data;
  const NodeIcon = nodeIcon(kind);
  const canConnect = !readOnly && !disabled;

  return (
    <article
      aria-label={`${label}，${kind} node`}
      className={`workflow-node workflow-node--${kind}${
        selected ? " is-selected" : ""
      }`}
    >
      <Handle
        aria-label={`连接到 ${label}`}
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
            <small>Input / 输入</small>
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
            <small>Trigger / 触发器</small>
            <strong title={activationText}>{activationText}</strong>
          </span>
        </button>
      )}
      <button
        aria-pressed={selected}
        className="workflow-node__body nodrag nopan"
        onClick={() => onSelect(id)}
        type="button"
      >
        <NodeIcon aria-hidden="true" size={17} />
        <span>
          <strong title={label}>{label}</strong>
          <small title={subtitle}>{subtitle}</small>
        </span>
        {kind === "agent" ? <Braces aria-hidden="true" size={14} /> : null}
      </button>
      <footer className="workflow-node__final">
        <CircleDot aria-hidden="true" size={12} />
        <span>
          {kind === "output" ? "Terminal / 流程输出" : "Final / 完成通知"}
        </span>
        {!readOnly && kind !== "output" ? (
          <IconButton
            aria-label={`移除 ${label}`}
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
          aria-label={`从 ${label} 的 Final 连线`}
          className="workflow-node__handle workflow-node__handle--source"
          id="final"
          isConnectable={canConnect}
          position={Position.Right}
          type="source"
        />
      ) : null}
    </article>
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
