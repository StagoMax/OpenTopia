import { Bot, RefreshCw } from "lucide-react";
import { useCallback, useState } from "react";
import type { ApiClient } from "../../api/client";
import type { AppSettings } from "../../types";
import { AgentTemplatePanel } from "../AgentTemplatePanel";
import { Badge, Button, Panel } from "../ui";
import { shortId } from "./model";
import { useEnterpriseStore } from "./store";
import type { EnterprisePageHeaderChange } from "./pageHeader";

export function AgentsPage({
  client,
  threadId,
  workspaceRoot,
  settings,
  onPageHeaderChange,
}: {
  client: ApiClient;
  threadId: string | null;
  workspaceRoot: string | null;
  settings: AppSettings | null;
  onPageHeaderChange?: EnterprisePageHeaderChange;
}) {
  const { snapshot, store } = useEnterpriseStore(client);
  const [templateEditorOpen, setTemplateEditorOpen] = useState(false);
  const handlePageHeaderChange = useCallback<EnterprisePageHeaderChange>(
    (header) => {
      setTemplateEditorOpen(Boolean(header));
      onPageHeaderChange?.(header);
    },
    [onPageHeaderChange],
  );
  return (
    <div className="enterprise-page">
      {!templateEditorOpen ? (
        <Panel
          title="Agent identities / Agent 身份"
          actions={
            <Button
              aria-label="刷新 Agent 身份"
              onClick={() => void store.load(true)}
              size="compact"
              variant="quiet"
            >
              <RefreshCw aria-hidden="true" size={14} /> 刷新
            </Button>
          }
        >
          <p className="enterprise-page__lede">
            Agent 是从已发布模板创建的稳定执行身份；它的模板版本、Connection
            操作和资源范围在创建时冻结。
          </p>
          <ol className="enterprise-card-list">
            {snapshot.agents.map((agent) => (
              <li key={agent.id}>
                <Bot aria-hidden="true" size={17} />
                <span>
                  <strong>
                    {agent.templateId}@{agent.templateVersion}
                  </strong>
                  <small>
                    Agent {shortId(agent.id)} · Thread {shortId(agent.threadId)}
                  </small>
                </span>
                <Badge
                  variant={agent.status === "active" ? "success" : "neutral"}
                >
                  {agent.status}
                </Badge>
              </li>
            ))}
            {snapshot.agents.length === 0 ? (
              <li className="enterprise-list__empty">尚未创建 Agent 身份。</li>
            ) : null}
          </ol>
        </Panel>
      ) : null}
      <AgentTemplatePanel
        client={client}
        onPageHeaderChange={handlePageHeaderChange}
        settings={settings}
        threadId={threadId}
        workspaceRoot={workspaceRoot}
      />
    </div>
  );
}
