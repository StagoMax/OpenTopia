import { Bot, Braces, CircleDot, Plus, RadioTower, Trash2 } from "lucide-react";
import type { AgentTemplateVersionView } from "../../types";
import { Button, IconButton } from "../ui";
import {
  activationAgentFinalNodeIds,
  activationLabel,
  createFinalActivation,
  createManualActivation,
  templateKey,
  type WorkflowAgentSelection,
} from "./flowActivation";
import "./workflow-graph.css";

const NODE_WIDTH = 252;
const NODE_HEIGHT = 176;
const COLUMN_GAP = 96;
const ROW_GAP = 72;
const CANVAS_PADDING = 32;

export function WorkflowGraphEditor({
  disabled,
  onChange,
  onEditAgent,
  onEditTrigger,
  selections,
  templates,
}: {
  disabled?: boolean;
  onChange(selections: WorkflowAgentSelection[]): void;
  onEditAgent(nodeId: string): void;
  onEditTrigger(nodeId: string): void;
  selections: WorkflowAgentSelection[];
  templates: AgentTemplateVersionView[];
}) {
  const positions = layoutNodes(selections);
  const rows = Math.max(1, Math.ceil(selections.length / 3));
  const columns = Math.max(1, Math.min(3, selections.length));
  const width =
    CANVAS_PADDING * 2 + columns * NODE_WIDTH + (columns - 1) * COLUMN_GAP;
  const height = CANVAS_PADDING * 2 + rows * NODE_HEIGHT + (rows - 1) * ROW_GAP;
  const edges = selections.flatMap((target) =>
    activationAgentFinalNodeIds(target.activation).map((sourceId) => ({
      sourceId,
      targetId: target.id,
    })),
  );

  function addAgent() {
    const option = templates[0];
    if (!option) return;
    const previous = selections.at(-1);
    onChange([
      ...selections,
      {
        id: `agent-${crypto.randomUUID()}`,
        templateKey: templateKey(option),
        activation: previous
          ? createFinalActivation(previous.id)
          : createManualActivation(),
      },
    ]);
  }

  function removeAgent(id: string) {
    const remaining = selections.filter((item) => item.id !== id);
    onChange(
      remaining.map((item, index) => {
        const referencesRemoved = activationAgentFinalNodeIds(
          item.activation,
        ).includes(id);
        if (!referencesRemoved) return item;
        const previous = remaining[index - 1];
        return {
          ...item,
          activation: previous
            ? createFinalActivation(previous.id)
            : createManualActivation(),
        };
      }),
    );
  }

  return (
    <section className="workflow-graph" aria-label="Flow graph / Flow 图">
      <header className="workflow-graph__header">
        <span>
          <strong>Flow graph / Flow 图</strong>
          <small>
            连线表示 Agent Final 订阅；每个节点仍可拥有独立的外部 Trigger。
          </small>
        </span>
        <Button
          disabled={disabled || templates.length === 0}
          onClick={addAgent}
          size="compact"
          variant="quiet"
        >
          <Plus aria-hidden="true" size={14} /> 添加 Agent
        </Button>
      </header>
      <div className="workflow-graph__viewport">
        <div
          className="workflow-graph__canvas"
          style={{ height, minWidth: width }}
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
              const endY = target.y + 28;
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
            const template = templates.find(
              (item) => templateKey(item) === selection.templateKey,
            );
            return (
              <article
                className="workflow-node"
                key={selection.id}
                style={{ left: position.x, top: position.y }}
              >
                <button
                  className="workflow-node__trigger"
                  onClick={() => onEditTrigger(selection.id)}
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
                <button
                  className="workflow-node__agent"
                  onClick={() => onEditAgent(selection.id)}
                  type="button"
                >
                  <Bot aria-hidden="true" size={17} />
                  <span>
                    <strong>{template?.template.name ?? "选择 Agent"}</strong>
                    <small>
                      {template
                        ? `${template.template.templateId}@${template.template.version}`
                        : "未绑定"}
                    </small>
                  </span>
                  <Braces aria-hidden="true" size={14} />
                </button>
                <footer className="workflow-node__final">
                  <CircleDot aria-hidden="true" size={12} />
                  <span>Final / 完成通知</span>
                  <IconButton
                    aria-label={`移除 ${template?.template.name ?? "Agent"}`}
                    disabled={disabled || selections.length === 1}
                    onClick={() => removeAgent(selection.id)}
                    size="compact"
                    variant="danger"
                  >
                    <Trash2 aria-hidden="true" size={13} />
                  </IconButton>
                </footer>
              </article>
            );
          })}
        </div>
      </div>
    </section>
  );
}

function layoutNodes(selections: readonly WorkflowAgentSelection[]) {
  return new Map(
    selections.map((selection, index) => [
      selection.id,
      {
        x: CANVAS_PADDING + (index % 3) * (NODE_WIDTH + COLUMN_GAP),
        y: CANVAS_PADDING + Math.floor(index / 3) * (NODE_HEIGHT + ROW_GAP),
      },
    ]),
  );
}
