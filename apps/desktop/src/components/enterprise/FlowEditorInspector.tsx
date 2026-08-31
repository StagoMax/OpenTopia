import {
  Bot,
  CheckCircle2,
  ChevronLeft,
  FileJson2,
  Inbox,
  RadioTower,
  ShieldCheck,
  SlidersHorizontal,
} from "lucide-react";
import type { AgentTemplateVersionView, FlowDraftView } from "../../types";
import {
  FLOW_LIBRARY_PROVIDER_OPTIONS,
  type FlowLibraryProviderSelection,
} from "../../flowLibraryBinding";
import { Badge, Button, SelectField, TextField } from "../ui";
import { activationLabel, templateKey } from "./flowActivation";
import {
  workflowNodeLabel,
  type WorkflowNodeSelection,
} from "./workflowNodeSelection";

export function FlowEditorInspector({
  draft,
  error,
  flowId,
  libraryProvider,
  name,
  nodes,
  notice,
  onChangeFlow,
  onChangeLibraryProvider,
  onChangeNode,
  onEditTrigger,
  onSelectNode,
  outcome,
  owner,
  passedDryRun,
  selectedNodeId,
  successfulTestRun,
  templates,
}: {
  draft: FlowDraftView | null;
  error: string | null;
  flowId: string;
  libraryProvider: FlowLibraryProviderSelection;
  name: string;
  nodes: WorkflowNodeSelection[];
  notice: string | null;
  onChangeFlow(change: Partial<FlowConfiguration>): void;
  onChangeLibraryProvider(provider: FlowLibraryProviderSelection): void;
  onChangeNode(node: WorkflowNodeSelection): void;
  onEditTrigger(nodeId: string): void;
  onSelectNode(nodeId: string | null): void;
  outcome: string;
  owner: string;
  passedDryRun: boolean;
  selectedNodeId: string | null;
  successfulTestRun: boolean;
  templates: AgentTemplateVersionView[];
}) {
  const selectedNode = nodes.find((node) => node.id === selectedNodeId) ?? null;

  return (
    <aside className="flow-editor-inspector" aria-label="Flow 配置">
      <header className="flow-editor-inspector__header">
        <span className="flow-editor-inspector__title">
          {selectedNode ? (
            <NodeIcon kind={selectedNode.kind} />
          ) : (
            <SlidersHorizontal aria-hidden="true" size={16} />
          )}
          <span>
            <strong>{selectedNode ? "Node 配置" : "Flow 配置"}</strong>
            <small>
              {selectedNode
                ? workflowNodeLabel(selectedNode, templates)
                : `${nodes.length} 个节点 · ${draft ? "草稿已保存" : "未保存"}`}
            </small>
          </span>
        </span>
        {selectedNode ? (
          <Button
            onClick={() => onSelectNode(null)}
            size="compact"
            variant="quiet"
          >
            <ChevronLeft aria-hidden="true" size={14} /> Flow
          </Button>
        ) : null}
      </header>

      <div className="flow-editor-inspector__body">
        {selectedNode ? (
          <NodeConfiguration
            node={selectedNode}
            nodes={nodes}
            onChange={onChangeNode}
            onEditTrigger={onEditTrigger}
            templates={templates}
          />
        ) : (
          <>
            <section className="flow-editor-inspector__section">
              <header>
                <strong>基本信息</strong>
                <Badge variant={draft ? "neutral" : "warning"}>
                  {draft ? "Draft" : "Unsaved"}
                </Badge>
              </header>
              <TextField
                label="Workflow ID"
                onChange={(event) =>
                  onChangeFlow({ flowId: event.target.value })
                }
                value={flowId}
              />
              <TextField
                label="名称"
                onChange={(event) => onChangeFlow({ name: event.target.value })}
                value={name}
              />
              <TextField
                label="所有者"
                onChange={(event) =>
                  onChangeFlow({ owner: event.target.value })
                }
                value={owner}
              />
              <SelectField<FlowLibraryProviderSelection>
                hint="只选择检索后端，不绑定具体数据库或 namespace；Agent 仍需具备 library_search 权限。"
                label="运行资料库"
                onChange={onChangeLibraryProvider}
                options={FLOW_LIBRARY_PROVIDER_OPTIONS}
                value={libraryProvider}
              />
              <label className="flow-editor-inspector__textarea">
                <span>业务结果</span>
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
                passedDryRun={passedDryRun}
                successfulTestRun={successfulTestRun}
              />
            </section>

            {draft ? (
              <details className="flow-editor-inspector__advanced">
                <summary>
                  <FileJson2 aria-hidden="true" size={14} /> Advanced JSON
                </summary>
                <pre>{JSON.stringify(draft.draft.spec, null, 2)}</pre>
              </details>
            ) : null}
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

function NodeConfiguration({
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
  return (
    <>
      <section className="flow-editor-inspector__section">
        <header>
          <strong>节点</strong>
          <Badge variant={node.kind === "approval" ? "warning" : "neutral"}>
            {node.kind}
          </Badge>
        </header>
        <TextField label="Node ID" readOnly value={node.id} />
        {node.kind === "agent" ? (
          <SelectField
            label="Agent"
            onChange={(nextTemplateKey) =>
              onChange({ ...node, templateKey: nextTemplateKey })
            }
            options={templates.map((item) => ({
              value: templateKey(item),
              label: `${item.template.name} · ${item.template.templateId}@${item.template.version}`,
            }))}
            value={node.templateKey}
          />
        ) : null}
        {node.kind === "approval" ? (
          <>
            <TextField
              label="名称"
              onChange={(event) =>
                onChange({ ...node, label: event.target.value })
              }
              value={node.label}
            />
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
          </>
        ) : null}
        {node.kind === "output" ? (
          <>
            <TextField label="名称" readOnly value={node.label} />
            <p className="flow-editor-inspector__note">
              Output 是当前 Flow 的固定终点，负责将最终结果写入 Inbox。
            </p>
          </>
        ) : null}
      </section>

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

function NodeIcon({ kind }: { kind: WorkflowNodeSelection["kind"] }) {
  const Icon =
    kind === "agent" ? Bot : kind === "approval" ? ShieldCheck : Inbox;
  return <Icon aria-hidden="true" size={16} />;
}

function WorkflowProgress({
  draft,
  passedDryRun,
  successfulTestRun,
}: {
  draft: FlowDraftView | null;
  passedDryRun: boolean;
  successfulTestRun: boolean;
}) {
  const steps = [
    ["Draft", Boolean(draft)],
    ["Validated", Boolean(draft?.draft.lastValidation?.valid)],
    ["Dry Run", passedDryRun],
    ["Test Run", successfulTestRun],
    ["Activated", draft?.draft.status === "published"],
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
