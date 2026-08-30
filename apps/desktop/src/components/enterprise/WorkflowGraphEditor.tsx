import {
  Bot,
  Braces,
  CircleDot,
  Inbox,
  Plus,
  RadioTower,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import type { AgentTemplateVersionView } from "../../types";
import { Button, IconButton, Popover } from "../ui";
import {
  activationLabel,
  activationSourceNodeIds,
  templateKey,
} from "./flowActivation";
import {
  addWorkflowNode,
  removeWorkflowNode,
  workflowNodeLabel,
  type AddableWorkflowNodeKind,
  type WorkflowNodeSelection,
} from "./workflowNodeSelection";
import "./workflow-graph.css";

const NODE_WIDTH = 260;
const NODE_HEIGHT = 156;
const COLUMN_GAP = 80;
const CANVAS_PADDING = 48;

export function WorkflowGraphEditor({
  disabled,
  onChange,
  onEditTrigger,
  onSelectNode,
  readOnly = false,
  selections,
  selectedNodeId,
  templates,
}: {
  disabled?: boolean;
  onChange?(selections: WorkflowNodeSelection[]): void;
  onEditTrigger?(nodeId: string): void;
  onSelectNode(nodeId: string): void;
  readOnly?: boolean;
  selections: WorkflowNodeSelection[];
  selectedNodeId: string | null;
  templates: AgentTemplateVersionView[];
}) {
  const positions = layoutNodes(selections);
  const width =
    CANVAS_PADDING * 2 +
    selections.length * NODE_WIDTH +
    Math.max(0, selections.length - 1) * COLUMN_GAP;
  const height = CANVAS_PADDING * 2 + NODE_HEIGHT;
  const edges = selections.flatMap((target) =>
    activationSourceNodeIds(target.activation).map((sourceId) => ({
      sourceId,
      targetId: target.id,
    })),
  );

  function addNode(kind: AddableWorkflowNodeKind) {
    onChange?.(addWorkflowNode(selections, kind, templates[0]));
  }

  return (
    <section className="workflow-graph" aria-label="Flow graph / Flow 图">
      <header className="workflow-graph__header">
        <span>
          <strong>Flow graph / Flow 图</strong>
          <small>
            Agent、Approval 与 Output 都是 Flow Node；连线表示上游 Final 订阅。
          </small>
        </span>
        {!readOnly ? (
          <Popover
            align="end"
            label="选择要添加的节点类型"
            placement="bottom"
            trigger={(props) => (
              <Button
                {...props}
                disabled={disabled}
                size="compact"
                variant="secondary"
              >
                <Plus aria-hidden="true" size={14} /> 添加节点
              </Button>
            )}
          >
            {({ close }) => (
              <div className="workflow-node-picker">
                <button
                  disabled={templates.length === 0}
                  onClick={() => {
                    addNode("agent");
                    close();
                  }}
                  type="button"
                >
                  <Bot aria-hidden="true" size={16} />
                  <span>
                    <strong>Agent</strong>
                    <small>运行一个已发布的 Agent 模板</small>
                  </span>
                </button>
                <button
                  onClick={() => {
                    addNode("approval");
                    close();
                  }}
                  type="button"
                >
                  <ShieldCheck aria-hidden="true" size={16} />
                  <span>
                    <strong>Approval</strong>
                    <small>暂停流程并等待人工审批</small>
                  </span>
                </button>
              </div>
            )}
          </Popover>
        ) : null}
      </header>
      <div className="workflow-graph__viewport">
        <div
          className="workflow-graph__canvas"
          style={{ minHeight: height, minWidth: width }}
        >
          <svg
            aria-hidden="true"
            className="workflow-graph__edges"
            height={height}
            viewBox={`0 0 ${width} ${height}`}
            width={width}
          >
            {edges.map((edge) => {
              const source = positions.get(edge.sourceId);
              const target = positions.get(edge.targetId);
              if (!source || !target) return null;
              const startX = source.x + NODE_WIDTH;
              const startY = source.y + NODE_HEIGHT / 2;
              const endX = target.x;
              const endY = target.y + NODE_HEIGHT / 2;
              const bend = Math.max(36, Math.abs(endX - startX) / 2);
              return (
                <path
                  d={`M ${startX} ${startY} C ${startX + bend} ${startY}, ${endX - bend} ${endY}, ${endX} ${endY}`}
                  key={`${edge.sourceId}:${edge.targetId}`}
                />
              );
            })}
          </svg>
          {selections.map((selection) => {
            const position = positions.get(selection.id)!;
            const label = workflowNodeLabel(selection, templates);
            const template =
              selection.kind === "agent"
                ? templates.find(
                    (item) => templateKey(item) === selection.templateKey,
                  )
                : null;
            const NodeIcon =
              selection.kind === "agent"
                ? Bot
                : selection.kind === "approval"
                  ? ShieldCheck
                  : Inbox;
            return (
              <article
                className={`workflow-node workflow-node--${selection.kind}${
                  selectedNodeId === selection.id ? " is-selected" : ""
                }`}
                key={selection.id}
                style={{ left: position.x, top: position.y }}
              >
                {selection.kind === "output" || readOnly ? (
                  <div className="workflow-node__trigger">
                    <RadioTower aria-hidden="true" size={14} />
                    <span>
                      <small>Input / 输入</small>
                      <strong>
                        {activationLabel(
                          selection.activation,
                          selections,
                          templates,
                        )}
                      </strong>
                    </span>
                  </div>
                ) : (
                  <button
                    className="workflow-node__trigger"
                    disabled={disabled}
                    onClick={() => onEditTrigger?.(selection.id)}
                    type="button"
                  >
                    <RadioTower aria-hidden="true" size={14} />
                    <span>
                      <small>Trigger / 触发器</small>
                      <strong>
                        {activationLabel(
                          selection.activation,
                          selections,
                          templates,
                        )}
                      </strong>
                    </span>
                  </button>
                )}
                <button
                  aria-pressed={selectedNodeId === selection.id}
                  className="workflow-node__body"
                  onClick={() => onSelectNode(selection.id)}
                  type="button"
                >
                  <NodeIcon aria-hidden="true" size={17} />
                  <span>
                    <strong>{label}</strong>
                    <small>
                      {selection.kind === "agent" && template
                        ? `${template.template.templateId}@${template.template.version}`
                        : `${selection.kind} node`}
                    </small>
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
                      disabled={disabled}
                      onClick={() =>
                        onChange?.(removeWorkflowNode(selections, selection.id))
                      }
                      size="compact"
                      variant="danger"
                    >
                      <Trash2 aria-hidden="true" size={13} />
                    </IconButton>
                  ) : null}
                </footer>
              </article>
            );
          })}
        </div>
      </div>
    </section>
  );
}

function layoutNodes(selections: readonly WorkflowNodeSelection[]) {
  return new Map(
    selections.map((selection, index) => [
      selection.id,
      {
        x: CANVAS_PADDING + index * (NODE_WIDTH + COLUMN_GAP),
        y: CANVAS_PADDING,
      },
    ]),
  );
}
