import { RadioTower, Settings2 } from "lucide-react";
import type { AgentTemplateVersionView } from "../../types";
import { Badge, Button, SelectField, Switch, TextField } from "../ui";
import { AgentCapabilitySummary } from "./AgentCapabilitySummary";
import { activationLabel, templateKey } from "./flowActivation";
import { latestPublishedTemplateVersions } from "./model";
import type { WorkflowNodeSelection } from "./workflowNodeSelection";
import { WorkflowStateWritesEditor } from "./WorkflowStateWritesEditor";

export function FlowNodeConfiguration({
  node,
  nodes,
  onChange,
  onEditTrigger,
  templates,
}: {
  node: WorkflowNodeSelection;
  nodes: WorkflowNodeSelection[];
  onChange(node: WorkflowNodeSelection): void;
  onEditTrigger(nodeId: string): void;
  templates: AgentTemplateVersionView[];
}) {
  const selectedTemplate =
    node.kind === "agent"
      ? templates.find((item) => templateKey(item) === node.templateKey)
      : undefined;
  const agentTemplates = latestPublishedTemplateVersions(templates);
  const selectedAgentId = selectedTemplate?.template.templateId ?? "";
  const selectedAgentVersions = selectedTemplate
    ? templates
        .filter((item) => item.template.templateId === selectedAgentId)
        .sort((left, right) => right.template.version - left.template.version)
    : [];
  return (
    <>
      <section className="flow-editor-inspector__section">
        <header>
          <strong>基本信息</strong>
          <Badge variant={node.kind === "approval" ? "warning" : "neutral"}>
            {nodeKindLabel(node.kind)}
          </Badge>
        </header>
        {node.kind !== "agent" && node.kind !== "output" ? (
          <TextField
            label="名称"
            onChange={(event) =>
              onChange({ ...node, label: event.target.value })
            }
            value={node.label}
          />
        ) : null}
        {node.kind === "agent" && agentTemplates.length > 0 ? (
          <>
            <SelectField
              hint="这里只决定这个节点由哪个 Agent 执行。"
              label="关联 Agent"
              onChange={(templateId) => {
                const latest = agentTemplates.find(
                  (item) => item.template.templateId === templateId,
                );
                if (latest) {
                  onChange({ ...node, templateKey: templateKey(latest) });
                }
              }}
              options={[
                {
                  value: "",
                  label: "请选择一个已发布 Agent",
                  disabled: true,
                },
                ...agentTemplates.map((item) => ({
                  value: item.template.templateId,
                  label: item.template.name,
                })),
              ]}
              required
              value={selectedAgentId}
            />
            {selectedTemplate ? (
              <SelectField
                hint="默认使用所选 Agent 的最新已发布版本，也可以固定到旧版本。"
                label="版本"
                onChange={(nextTemplateKey) =>
                  onChange({ ...node, templateKey: nextTemplateKey })
                }
                options={selectedAgentVersions.map((item, index) => ({
                  value: templateKey(item),
                  label: `版本 ${item.template.version}${index === 0 ? "（最新）" : ""}`,
                }))}
                value={node.templateKey}
              />
            ) : (
              <p className="flow-editor-inspector__note">
                选择 Agent 后再决定使用哪个已发布版本。
              </p>
            )}
          </>
        ) : null}
        {node.kind === "agent" && agentTemplates.length === 0 ? (
          <p className="flow-editor-inspector__note" role="status">
            还没有已发布的 Agent。请先在 Agents 中创建并发布，再回来完成关联。
          </p>
        ) : null}
        {node.kind === "tool" ? (
          <p className="flow-editor-inspector__note">
            Action 会按确定步骤直接调用 Tool，不经过 Agent 推理。若希望 Agent
            自主决定何时调用，请把 Tool 配置到 Agent 模板中。
          </p>
        ) : null}
        {node.kind === "skill" || node.kind === "tool" ? (
          <TextField
            hint={
              node.kind === "skill"
                ? "填写 ExecutionContext 中可见的 Skill ID"
                : "填写 Tool Registry 暴露的精确工具名"
            }
            label={
              node.kind === "skill" ? "Skill reference" : "执行 Tool / API"
            }
            onChange={(event) =>
              onChange({ ...node, reference: event.target.value })
            }
            placeholder={node.kind === "skill" ? "skill-id" : "tool_name"}
            value={node.reference}
          />
        ) : null}
        {node.kind === "tool" ? (
          <label className="flow-editor-inspector__switch-row">
            <span>
              <strong>允许并行</strong>
              <small>仅对确认线程安全且无冲突写入的工具开启</small>
            </span>
            <Switch
              checked={node.parallelSafe}
              label="允许 Tool 并行执行"
              onChange={(parallelSafe) => onChange({ ...node, parallelSafe })}
            />
          </label>
        ) : null}
        {node.kind === "condition" ? (
          <>
            <p className="flow-editor-inspector__note">
              这是旧版兼容节点。新建分支时，请选中连线并直接配置路由条件。
            </p>
            <TextField
              hint="支持路径真值、!path、== 和 !=；例如 score == 1"
              label="条件表达式"
              onChange={(event) =>
                onChange({ ...node, expression: event.target.value })
              }
              placeholder="matched == true"
              value={node.expression}
            />
          </>
        ) : null}
        {node.kind === "validator" ? (
          <>
            <TextField
              hint="用逗号分隔；支持点路径，例如 customer.id"
              label="必填字段"
              onChange={(event) =>
                onChange({
                  ...node,
                  requiredFields: commaSeparatedValues(event.target.value),
                })
              }
              placeholder="customer.id, amount"
              value={node.requiredFields.join(", ")}
            />
            <TextField
              hint="留空时仅校验必填字段"
              label="附加表达式（可选）"
              onChange={(event) =>
                onChange({ ...node, expression: event.target.value })
              }
              placeholder="score != 0"
              value={node.expression}
            />
          </>
        ) : null}
        {node.kind === "approval" ? (
          <label className="flow-editor-inspector__textarea">
            <span>审批说明</span>
            <textarea
              onChange={(event) =>
                onChange({ ...node, instructions: event.target.value })
              }
              rows={5}
              value={node.instructions}
            />
          </label>
        ) : null}
        {node.kind === "join" ? (
          <p className="flow-editor-inspector__note">
            Join 会等待全部上游节点完成，并把各上游输出按节点 ID 汇合后继续。
          </p>
        ) : null}
        {node.kind === "loop" ? (
          <p className="flow-editor-inspector__note">
            这是旧版兼容节点。新建循环时，直接把节点连回上游；编辑器会自动创建受限反馈边。
          </p>
        ) : null}
        {node.kind === "skill" ? (
          <p className="flow-editor-inspector__note">
            这是旧版兼容节点。新 Flow 的 Skill 能力由 Agent 模板统一配置。
          </p>
        ) : null}
        {node.kind === "output" ? (
          <>
            <TextField label="名称" readOnly value={node.label} />
            <p className="flow-editor-inspector__note">
              Output 是当前 Flow 的固定终点，负责将最终结果写入 Inbox。
            </p>
          </>
        ) : null}
        <details className="flow-editor-inspector__advanced">
          <summary>
            <Settings2 aria-hidden="true" size={14} /> 节点高级信息
          </summary>
          <div className="flow-editor-inspector__advanced-body">
            <TextField label="Node ID" readOnly value={node.id} />
          </div>
        </details>
      </section>

      {node.kind === "agent" ? (
        <AgentCapabilitySummary template={selectedTemplate} />
      ) : null}

      <WorkflowStateWritesEditor
        onChange={(stateWrites) => onChange({ ...node, stateWrites })}
        writes={node.stateWrites ?? []}
      />

      <section className="flow-editor-inspector__section">
        <header>
          <strong>Activation</strong>
        </header>
        <div className="flow-editor-inspector__activation">
          <RadioTower aria-hidden="true" size={14} />
          <span>
            <small>Trigger / 上游来源</small>
            <strong>
              {activationLabel(node.activation, nodes, templates)}
            </strong>
          </span>
        </div>
        {node.kind !== "output" ? (
          <Button
            onClick={() => onEditTrigger(node.id)}
            size="compact"
            variant="secondary"
          >
            <RadioTower aria-hidden="true" size={14} /> 配置 Trigger
          </Button>
        ) : null}
      </section>
    </>
  );
}

function nodeKindLabel(kind: WorkflowNodeSelection["kind"]) {
  if (kind === "tool") return "action";
  if (kind === "skill" || kind === "condition" || kind === "loop") {
    return `legacy ${kind}`;
  }
  return kind;
}

function commaSeparatedValues(value: string) {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}
