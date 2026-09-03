import { Bot, Cable, FileText, Workflow } from "lucide-react";
import { useEffect } from "react";
import type { ApiClient } from "../../api/client";
import type { AppSettings } from "../../types";
import { AgentTemplatePanel } from "../AgentTemplatePanel";
import { Badge } from "../ui";
import { useEnterpriseStore } from "./store";
import {
  templateKeyForAgent,
  FlowInspectorPortal,
  useFlowAgentSelection,
  useFlowWorkspaceTitle,
} from "./flowAgentSelection";

export function AgentsPage({
  client,
  settings,
  threadId,
  workspaceRoot,
}: {
  client: ApiClient;
  settings: AppSettings | null;
  threadId: string | null;
  workspaceRoot: string | null;
}) {
  const { snapshot } = useEnterpriseStore(client);
  const selection = useFlowAgentSelection();
  const requestedTemplateKey = selection?.selectedTemplateKey ?? null;
  const selectedTemplate = selection?.creatingAgent
    ? null
    : requestedTemplateKey
      ? (snapshot.templates.find(
          (view) =>
            templateKeyForAgent(
              view.template.templateId,
              view.template.version,
            ) === requestedTemplateKey,
        ) ?? null)
      : (snapshot.templates[0] ?? null);
  const selectedTemplateKey = selectedTemplate
    ? templateKeyForAgent(
        selectedTemplate.template.templateId,
        selectedTemplate.template.version,
      )
    : null;

  useEffect(() => {
    if (
      selection &&
      !selection.creatingAgent &&
      !requestedTemplateKey &&
      selectedTemplateKey
    ) {
      selection.setSelectedTemplateKey(selectedTemplateKey);
    }
  }, [requestedTemplateKey, selectedTemplateKey, selection]);

  useFlowWorkspaceTitle(
    selection?.creatingAgent
      ? "New Agent / 新建 Agent"
      : selectedTemplate?.template.name,
  );
  const selectedAgent = selectedTemplate?.template;
  const usedByFlows = selectedAgent
    ? snapshot.flows.filter((flow) =>
        Object.values(flow.activeRevision.compiledWorkflow.agentSpecs).some(
          (agent) =>
            agent.templateId === selectedAgent.templateId &&
            agent.templateVersion === selectedAgent.version,
        ),
      )
    : [];
  const matchingInstances = selectedAgent
    ? snapshot.agents.filter(
        (agent) =>
          agent.templateId === selectedAgent.templateId &&
          agent.templateVersion === selectedAgent.version,
      )
    : [];
  const activeInstances = matchingInstances.filter(
    (agent) => agent.status === "active",
  ).length;
  const connectionCount =
    selectedAgent?.spec.connectionBindings?.length ??
    selectedAgent?.spec.capabilities.mcpServers.length ??
    0;

  return (
    <>
      <FlowInspectorPortal>
        <AgentTemplatePanel
          client={client}
          settings={settings}
          showTemplateCollection={false}
          threadId={threadId}
          variant="rail"
          workspaceRoot={workspaceRoot}
        />
      </FlowInspectorPortal>
      <div className="enterprise-page enterprise-agents-page">
        {selectedTemplate ? (
          <article
            aria-labelledby="enterprise-agent-title"
            className="enterprise-agent-overview"
          >
            <section className="enterprise-core-detail__summary">
              <span className="enterprise-core-detail__icon" aria-hidden="true">
                <Bot size={20} />
              </span>
              <div>
                <small>
                  {selectedTemplate.template.owner} · 版本{" "}
                  {selectedTemplate.template.version}
                </small>
                <span className="enterprise-agent-overview__title-row">
                  <h2 id="enterprise-agent-title">
                    {selectedTemplate.template.name}
                  </h2>
                  <Badge
                    variant={
                      selectedTemplate.template.status === "published"
                        ? "success"
                        : "warning"
                    }
                  >
                    {selectedTemplate.template.status === "published"
                      ? "已发布"
                      : "草稿"}
                  </Badge>
                </span>
                <p>
                  {selectedTemplate.template.spec.description ||
                    "此 Agent 尚未填写用途说明。"}
                </p>
              </div>
            </section>

            <dl
              className="enterprise-agent-overview__facts"
              aria-label="Agent 使用摘要"
            >
              <div>
                <Workflow aria-hidden="true" size={16} />
                <span>
                  <strong>{usedByFlows.length}</strong>
                  <small>使用中的 Flow</small>
                </span>
              </div>
              <div>
                <Bot aria-hidden="true" size={16} />
                <span>
                  <strong>{activeInstances}</strong>
                  <small>活跃实例</small>
                </span>
              </div>
              <div>
                <Cable aria-hidden="true" size={16} />
                <span>
                  <strong>{connectionCount}</strong>
                  <small>外部连接</small>
                </span>
              </div>
            </dl>

            {usedByFlows.length > 0 ? (
              <section className="enterprise-core-detail__payload enterprise-agent-overview__usage">
                <header>
                  <h3>用于这些 Flow</h3>
                </header>
                <ul>
                  {usedByFlows.slice(0, 5).map((flow) => (
                    <li key={flow.flowId}>{flow.name}</li>
                  ))}
                </ul>
              </section>
            ) : null}

            <details className="enterprise-agent-instructions">
              <summary>
                <FileText aria-hidden="true" size={14} />
                查看完整职责说明
              </summary>
              <pre>
                {selectedTemplate.template.spec.instructions || "暂无职责说明"}
              </pre>
            </details>
          </article>
        ) : (
          <div className="enterprise-agent-prompt-empty" role="status">
            <span>
              <FileText aria-hidden="true" size={20} />
            </span>
            <strong>
              {selection?.creatingAgent
                ? "配置新的 Agent"
                : snapshot.status === "loading"
                  ? "正在加载 Agent"
                  : requestedTemplateKey
                    ? "正在同步 Agent"
                    : "尚未配置 Agent"}
            </strong>
            <p>
              {selection?.creatingAgent
                ? "在右侧填写 Agent 配置；保存后会自动出现在左侧列表。"
                : snapshot.status === "error"
                  ? snapshot.error || "Agent 加载失败，请稍后重试。"
                  : "从左侧选择 Agent，或使用新建按钮创建一个 Agent。"}
            </p>
          </div>
        )}
      </div>
    </>
  );
}
