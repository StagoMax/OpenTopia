import { Cable, ContactRound, Database, PlugZap } from "lucide-react";
import { Badge, Button, Panel } from "./ui";
import "../styles/flow-library-panel.css";

export type FlowLibraryConnectorKind = "mcp" | "database" | "business_app";

export type FlowLibraryConnector = {
  id: string;
  kind: FlowLibraryConnectorKind;
  name: string;
  provider?: string;
  description?: string;
  status: "connected" | "syncing" | "attention" | "disabled";
};

export type FlowLibraryPanelProps = {
  connectors?: readonly FlowLibraryConnector[];
  onAddConnector?: (kind: FlowLibraryConnectorKind) => void;
  onManageConnector?: (connector: FlowLibraryConnector) => void;
};

const FLOW_LIBRARY_KINDS: ReadonlyArray<{
  kind: FlowLibraryConnectorKind;
  title: string;
  description: string;
}> = [
  {
    kind: "mcp",
    title: "MCP 服务",
    description: "连接本地或远程 MCP，登录后按账号、租户与授权范围开放工具。",
  },
  {
    kind: "database",
    title: "业务数据库",
    description: "连接 SQL、数据仓库或内部数据服务，并按身份授予读写范围。",
  },
  {
    kind: "business_app",
    title: "业务系统",
    description: "连接 CRM、ERP、工单与协作应用，并保留独立的凭据引用和权限范围。",
  },
];

export function FlowLibraryPanel({
  connectors = [],
  onAddConnector,
  onManageConnector,
}: FlowLibraryPanelProps) {
  return (
    <div className="flow-library-panel">
      <header className="flow-library-panel__intro">
        <span className="flow-library-panel__intro-icon">
          <PlugZap aria-hidden="true" size={18} />
        </span>
        <span>
          <strong>Connections / 连接</strong>
          <small>MCP、API、数据库与业务系统授权</small>
        </span>
        <Badge variant="neutral">接口预留</Badge>
      </header>

      <div className="flow-library-panel__catalog">
        {FLOW_LIBRARY_KINDS.map((item) => (
          <Panel
            actions={
              <Button
                disabled={!onAddConnector}
                onClick={() => onAddConnector?.(item.kind)}
                size="compact"
                variant="quiet"
              >
                <Cable aria-hidden="true" size={14} /> 接入
              </Button>
            }
            key={item.kind}
            title={item.title}
          >
            <span className="flow-library-panel__catalog-icon">
              {libraryKindIcon(item.kind)}
            </span>
            <p>{item.description}</p>
          </Panel>
        ))}
      </div>

      <Panel
        actions={<Badge variant="neutral">{connectors.length} connected</Badge>}
        title="已连接服务"
      >
        {connectors.length > 0 ? (
          <div className="flow-library-panel__sources">
            {connectors.map((connector) => (
              <article key={connector.id}>
                <span className="flow-library-panel__source-icon">
                  {libraryKindIcon(connector.kind)}
                </span>
                <span>
                  <strong>{connector.name}</strong>
                  <small>
                    {connector.provider ?? libraryKindLabel(connector.kind)}
                    {connector.description ? ` · ${connector.description}` : ""}
                  </small>
                </span>
                <Badge variant={libraryStatusVariant(connector.status)}>
                  {connector.status}
                </Badge>
                <Button
                  disabled={!onManageConnector}
                  onClick={() => onManageConnector?.(connector)}
                  size="compact"
                  variant="quiet"
                >
                  管理
                </Button>
              </article>
            ))}
          </div>
        ) : (
          <div className="flow-library-panel__empty">
            <Cable aria-hidden="true" size={18} />
            <strong>尚未配置 Connection</strong>
            <span>连接配置保存能力与凭据引用；Agent Template 只复用配置，不复制账号密钥。</span>
          </div>
        )}
      </Panel>
    </div>
  );
}

function libraryKindIcon(kind: FlowLibraryConnectorKind) {
  if (kind === "database") return <Database aria-hidden="true" size={16} />;
  if (kind === "business_app") {
    return <ContactRound aria-hidden="true" size={16} />;
  }
  return <Cable aria-hidden="true" size={16} />;
}

function libraryKindLabel(kind: FlowLibraryConnectorKind) {
  if (kind === "database") return "Database";
  if (kind === "business_app") return "CRM / ERP / App";
  return "MCP";
}

function libraryStatusVariant(status: FlowLibraryConnector["status"]) {
  if (status === "connected") return "success" as const;
  if (status === "attention") return "warning" as const;
  if (status === "disabled") return "neutral" as const;
  return "info" as const;
}
