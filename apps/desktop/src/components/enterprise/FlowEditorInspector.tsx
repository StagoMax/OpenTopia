import {
  BadgeCheck,
  Bot,
  CheckCircle2,
  ChevronLeft,
  FileJson2,
  Gauge,
  GitBranch,
  Inbox,
  Link2,
  Merge,
  Repeat2,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Settings2,
  Wrench,
} from "lucide-react";
import type {
  AgentTemplateVersionView,
  FlowDraftView,
  FlowRun,
  FlowSpec,
} from "../../types";
import { Badge, Button, SelectField, TextField } from "../ui";
import {
  workflowNodeLabel,
  type WorkflowNodeSelection,
} from "./workflowNodeSelection";
import { FlowConnectionConfiguration } from "./FlowConnectionConfiguration";
import type { WorkflowConnection } from "./workflowGraphOperations";
import type { WorkflowEdgeConfiguration } from "./workflowNodeSelection";
import { FlowNodeConfiguration } from "./FlowNodeConfiguration";
import { FlowNodeTestRunDetails } from "./FlowNodeTestRunDetails";

export function FlowEditorInspector({
  draft,
  error,
  flowId,
  name,
  nodes,
  notice,
  onChangeFlow,
  onChangeNode,
  onChangeConnection,
  onEditTrigger,
  onChangeRuntimeConfiguration,
  onSelectConnection,
  onSelectNode,
  outcome,
  owner,
  runtimeConfiguration,
  selectedNodeId,
  selectedConnection,
  successfulTestRun,
  testRun,
  templates,
}: {
  draft: FlowDraftView | null;
  error: string | null;
  flowId: string;
  name: string;
  nodes: WorkflowNodeSelection[];
  notice: string | null;
  onChangeFlow(change: Partial<FlowConfiguration>): void;
  onChangeNode(node: WorkflowNodeSelection): void;
  onChangeConnection(
    connection: WorkflowConnection,
    configuration: WorkflowEdgeConfiguration,
  ): void;
  onChangeRuntimeConfiguration(configuration: FlowRuntimeConfiguration): void;
  onEditTrigger(nodeId: string): void;
  onSelectConnection(connection: WorkflowConnection | null): void;
  onSelectNode(nodeId: string | null): void;
  outcome: string;
  owner: string;
  runtimeConfiguration: FlowRuntimeConfiguration;
  selectedNodeId: string | null;
  selectedConnection: WorkflowConnection | null;
  successfulTestRun: boolean;
  testRun: FlowRun | null;
  templates: AgentTemplateVersionView[];
}) {
  const selectedNode = nodes.find((node) => node.id === selectedNodeId) ?? null;

  return (
    <aside className="flow-editor-inspector" aria-label="Flow 配置">
      <header className="flow-editor-inspector__header">
        <span className="flow-editor-inspector__title">
          {selectedConnection ? (
            <Link2 aria-hidden="true" size={16} />
          ) : selectedNode ? (
            <NodeIcon kind={selectedNode.kind} />
          ) : (
            <SlidersHorizontal aria-hidden="true" size={16} />
          )}
          <span>
            <strong>
              {selectedConnection
                ? "连线配置"
                : selectedNode
                  ? "节点设置"
                  : "Flow 设置"}
            </strong>
            <small>
              {selectedConnection
                ? `${selectedConnection.sourceId} → ${selectedConnection.targetId}`
                : selectedNode
                  ? workflowNodeLabel(selectedNode, templates)
                  : `${nodes.filter((node) => node.kind !== "output").length} 个步骤 · ${draft ? "草稿已保存" : "尚未保存"}`}
            </small>
          </span>
        </span>
        {selectedNode || selectedConnection ? (
          <Button
            onClick={() => {
              onSelectConnection(null);
              onSelectNode(null);
            }}
            size="compact"
            variant="quiet"
          >
            <ChevronLeft aria-hidden="true" size={14} /> Flow 设置
          </Button>
        ) : null}
      </header>

      <div className="flow-editor-inspector__body">
        {selectedConnection ? (
          <FlowConnectionConfiguration
            connection={selectedConnection}
            nodes={nodes}
            onChange={(configuration) =>
              onChangeConnection(selectedConnection, configuration)
            }
            templates={templates}
          />
        ) : selectedNode ? (
          <>
            <FlowNodeConfiguration
              key={selectedNode.id}
              node={selectedNode}
              nodes={nodes}
              onChange={onChangeNode}
              onEditTrigger={onEditTrigger}
              templates={templates}
            />
            <FlowNodeTestRunDetails nodeId={selectedNode.id} run={testRun} />
          </>
        ) : (
          <>
            <section className="flow-editor-inspector__section">
              <header>
                <span>
                  <strong>Flow 目标</strong>
                  <small>先说明要做什么；技术标识放在高级设置中。</small>
                </span>
                <Badge variant={draft ? "neutral" : "warning"}>
                  {draft ? "已保存" : "未保存"}
                </Badge>
              </header>
              <TextField
                label="名称"
                onChange={(event) => onChangeFlow({ name: event.target.value })}
                value={name}
              />
              <label className="flow-editor-inspector__textarea">
                <span>要完成的结果</span>
                <textarea
                  onChange={(event) =>
                    onChangeFlow({ outcome: event.target.value })
                  }
                  rows={5}
                  value={outcome}
                />
              </label>
            </section>

            <section className="flow-editor-inspector__section">
              <header>
                <strong>发布进度</strong>
              </header>
              <WorkflowProgress
                draft={draft}
                successfulTestRun={successfulTestRun}
              />
            </section>

            <details className="flow-editor-inspector__advanced">
              <summary>
                <Settings2 aria-hidden="true" size={14} /> 高级设置
              </summary>
              <div className="flow-editor-inspector__advanced-body">
                <TextField
                  hint="用于 API、日志和版本引用。"
                  label="Workflow ID"
                  onChange={(event) =>
                    onChangeFlow({ flowId: event.target.value })
                  }
                  value={flowId}
                />
                <TextField
                  label="所有者"
                  onChange={(event) =>
                    onChangeFlow({ owner: event.target.value })
                  }
                  value={owner}
                />
                <SelectField<FlowSpec["riskClass"]>
                  hint="高风险 Flow 会在运行边界采用更严格的人工控制。"
                  label="风险级别"
                  onChange={(riskClass) =>
                    onChangeRuntimeConfiguration({
                      ...runtimeConfiguration,
                      riskClass,
                    })
                  }
                  options={[
                    { value: "low", label: "低" },
                    { value: "medium", label: "中" },
                    { value: "high", label: "高" },
                    { value: "critical", label: "关键" },
                  ]}
                  value={runtimeConfiguration.riskClass}
                />
                <fieldset className="flow-editor-budget">
                  <legend>
                    <Gauge aria-hidden="true" size={14} /> 运行预算
                  </legend>
                  <TextField
                    label="最多节点执行次数"
                    min={1}
                    onChange={(event) =>
                      onChangeRuntimeConfiguration({
                        ...runtimeConfiguration,
                        budget: {
                          ...runtimeConfiguration.budget,
                          maxNodeExecutions: positiveInteger(
                            event.target.value,
                            runtimeConfiguration.budget.maxNodeExecutions,
                          ),
                        },
                      })
                    }
                    type="number"
                    value={runtimeConfiguration.budget.maxNodeExecutions}
                  />
                  <TextField
                    label="最多工具调用次数"
                    min={1}
                    onChange={(event) =>
                      onChangeRuntimeConfiguration({
                        ...runtimeConfiguration,
                        budget: {
                          ...runtimeConfiguration.budget,
                          maxToolCalls: positiveInteger(
                            event.target.value,
                            runtimeConfiguration.budget.maxToolCalls,
                          ),
                        },
                      })
                    }
                    type="number"
                    value={runtimeConfiguration.budget.maxToolCalls}
                  />
                  <TextField
                    hint="单位：秒"
                    label="最长运行时间"
                    min={1}
                    onChange={(event) =>
                      onChangeRuntimeConfiguration({
                        ...runtimeConfiguration,
                        budget: {
                          ...runtimeConfiguration.budget,
                          maxDurationSeconds: positiveInteger(
                            event.target.value,
                            runtimeConfiguration.budget.maxDurationSeconds,
                          ),
                        },
                      })
                    }
                    type="number"
                    value={runtimeConfiguration.budget.maxDurationSeconds}
                  />
                  <TextField
                    label="最多循环次数"
                    min={1}
                    onChange={(event) =>
                      onChangeRuntimeConfiguration({
                        ...runtimeConfiguration,
                        budget: {
                          ...runtimeConfiguration.budget,
                          maxLoopIterations: positiveInteger(
                            event.target.value,
                            runtimeConfiguration.budget.maxLoopIterations,
                          ),
                        },
                      })
                    }
                    type="number"
                    value={runtimeConfiguration.budget.maxLoopIterations}
                  />
                </fieldset>
                {draft ? (
                  <details className="flow-editor-inspector__json">
                    <summary>
                      <FileJson2 aria-hidden="true" size={14} /> 查看草稿 JSON
                    </summary>
                    <pre>{JSON.stringify(draft.draft.spec, null, 2)}</pre>
                  </details>
                ) : null}
              </div>
            </details>
          </>
        )}

        {error ? (
          <p className="enterprise-page__message is-error" role="alert">
            {error}
          </p>
        ) : null}
        {notice ? (
          <p className="enterprise-page__message is-success" role="status">
            {notice}
          </p>
        ) : null}
      </div>
    </aside>
  );
}

type FlowConfiguration = {
  flowId: string;
  name: string;
  owner: string;
  outcome: string;
};

export type FlowRuntimeConfiguration = {
  budget: FlowSpec["budget"];
  riskClass: FlowSpec["riskClass"];
};

function NodeIcon({ kind }: { kind: WorkflowNodeSelection["kind"] }) {
  const Icon =
    kind === "agent"
      ? Bot
      : kind === "skill"
        ? Sparkles
        : kind === "tool"
          ? Wrench
          : kind === "condition"
            ? GitBranch
            : kind === "validator"
              ? BadgeCheck
              : kind === "approval"
                ? ShieldCheck
                : kind === "join"
                  ? Merge
                  : kind === "loop"
                    ? Repeat2
                    : Inbox;
  return <Icon aria-hidden="true" size={16} />;
}

function positiveInteger(value: string, fallback: number) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function WorkflowProgress({
  draft,
  successfulTestRun,
}: {
  draft: FlowDraftView | null;
  successfulTestRun: boolean;
}) {
  const steps = [
    ["保存草稿", Boolean(draft)],
    ["通过校验", Boolean(draft?.draft.lastValidation?.valid)],
    ["完成 Test Run", successfulTestRun],
    ["激活 Flow", draft?.draft.status === "published"],
  ] as const;
  return (
    <ol className="flow-editor-progress" aria-label="Flow 激活进度">
      {steps.map(([label, done]) => (
        <li className={done ? "is-done" : undefined} key={label}>
          <CheckCircle2 aria-hidden="true" size={14} />
          <span>{label}</span>
        </li>
      ))}
    </ol>
  );
}
