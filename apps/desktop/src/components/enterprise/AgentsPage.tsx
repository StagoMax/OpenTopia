import { FileText } from "lucide-react";
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
            aria-labelledby="enterprise-agent-prompt-title"
            className="enterprise-agent-prompt"
          >
            <header className="enterprise-agent-prompt__header">
              <div className="enterprise-agent-prompt__title-row">
                <h2 id="enterprise-agent-prompt-title">
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
              </div>
              <div className="enterprise-agent-prompt__metadata">
                <FileText aria-hidden="true" size={14} />
                <span>
                  {selectedTemplate.template.templateId}@
                  {selectedTemplate.template.version}
                </span>
                <span aria-hidden="true">·</span>
                <span>System prompt / 系统提示词</span>
              </div>
              {selectedTemplate.template.spec.description ? (
                <p>{selectedTemplate.template.spec.description}</p>
              ) : null}
            </header>
            <pre>
              {selectedTemplate.template.spec.instructions || "暂无系统提示词"}
            </pre>
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
