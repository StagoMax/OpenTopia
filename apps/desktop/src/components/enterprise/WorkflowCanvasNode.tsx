import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import {
  Bot,
  Braces,
  CircleDot,
  Inbox,
  RadioTower,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { IconButton } from "../ui";
import type { WorkflowNodeSelection } from "./workflowNodeSelection";

export type WorkflowCanvasNodeData = Record<string, unknown> & {
  activationText: string;
  disabled: boolean;
  label: string;
  onEditTrigger(nodeId: string): void;
  onRemove(nodeId: string): void;
  onSelect(nodeId: string): void;
  readOnly: boolean;
  selection: WorkflowNodeSelection;
  subtitle: string;
};

export type WorkflowCanvasNodeType = Node<
  WorkflowCanvasNodeData,
  "workflowNode"
>;

export function WorkflowCanvasNode({
  data,
  selected,
}: NodeProps<WorkflowCanvasNodeType>) {
  const {
    activationText,
    disabled,
    label,
    onEditTrigger,
    onRemove,
    onSelect,
    readOnly,
    selection,
    subtitle,
  } = data;
  const NodeIcon =
    selection.kind === "agent"
      ? Bot
      : selection.kind === "approval"
        ? ShieldCheck
        : Inbox;
  const canConnect = !readOnly && !disabled;

  return (
    <article
      aria-label={`${label}，${selection.kind} node`}
      className={`workflow-node workflow-node--${selection.kind}${
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
      {selection.kind === "output" || readOnly ? (
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
          onClick={() => onEditTrigger(selection.id)}
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
        onClick={() => onSelect(selection.id)}
        type="button"
      >
        <NodeIcon aria-hidden="true" size={17} />
        <span>
          <strong title={label}>{label}</strong>
          <small title={subtitle}>{subtitle}</small>
        </span>
        {selection.kind === "agent" ? (
          <Braces aria-hidden="true" size={14} />
        ) : null}
      </button>
      <footer className="workflow-node__final">
        <CircleDot aria-hidden="true" size={12} />
        <span>
          {selection.kind === "output"
            ? "Terminal / 流程输出"
            : "Final / 完成通知"}
        </span>
        {!readOnly && selection.kind !== "output" ? (
          <IconButton
            aria-label={`移除 ${label}`}
            className="nodrag nopan"
            disabled={disabled}
            onClick={(event) => {
              event.stopPropagation();
              onRemove(selection.id);
            }}
            size="compact"
            variant="danger"
          >
            <Trash2 aria-hidden="true" size={13} />
          </IconButton>
        ) : null}
      </footer>
      {selection.kind !== "output" ? (
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
