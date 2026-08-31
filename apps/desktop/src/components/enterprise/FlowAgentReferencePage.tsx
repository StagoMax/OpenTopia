import { Bot, Cable, Database, ShieldCheck } from "lucide-react";
import { agentKnowledgeBindingSummary } from "../../agentKnowledgeBinding";
import type { AgentTemplateVersionView } from "../../types";
import { Badge, SelectField } from "../ui";
import { templateKey } from "./flowActivation";
import type { WorkflowAgentSelection } from "./workflowNodeSelection";

export function FlowAgentReferencePage({
  node,
  onChange,
  templates,
}: {
  node: WorkflowAgentSelection;
  onChange(templateKey: string): void;
  templates: AgentTemplateVersionView[];
}) {
  const selected = templates.find(
    (item) => templateKey(item) === node.templateKey,
  );
  return (
    <div className="flow-agent-reference-page">
      <header>
        <span className="flow-trigger-page__icon">
          <Bot aria-hidden="true" size={18} />
        </span>
        <span>
          <strong>Agent reference / Agent 引用</strong>
          <small>
            Flow 节点只固定 Agent 版本和节点 Trigger；Agent 配置本身可在 Flow
            之外独立复用。
          </small>
        </span>
      </header>
      <SelectField
        label="Agent"
        onChange={onChange}
        options={templates.map((item) => ({
          value: templateKey(item),
          label: `${item.template.name} · ${item.template.templateId}@${item.template.version}`,
        }))}
        value={node.templateKey}
      />
      {selected ? (
        <div className="flow-agent-reference-page__details">
          <section>
            <header>
              <strong>{selected.template.name}</strong>
              <Badge
                variant={
                  selected.template.status === "published"
                    ? "success"
                    : "warning"
                }
              >
                {selected.template.status === "published" ? "已发布" : "草稿"}
              </Badge>
            </header>
            <p>{selected.template.spec.description || "暂无说明"}</p>
            <pre>{selected.template.spec.instructions}</pre>
          </section>
          <dl>
            <div>
              <dt>
                <Cable aria-hidden="true" size={14} /> Connections
              </dt>
              <dd>
                {selected.template.spec.connectionBindings?.length ?? 0} 个绑定
              </dd>
            </div>
            <div>
              <dt>
                <Database aria-hidden="true" size={14} /> Knowledge
              </dt>
              <dd>
                {agentKnowledgeBindingSummary(
                  selected.template.spec.knowledgeBinding,
                )}
              </dd>
            </div>
            <div>
              <dt>
                <ShieldCheck aria-hidden="true" size={14} /> Permissions
              </dt>
              <dd>{selected.template.spec.riskClass}</dd>
            </div>
          </dl>
        </div>
      ) : null}
    </div>
  );
}
