import { BookOpen, Cable, ContactRound, Database, Library } from "lucide-react";
import { Badge, Button, Panel } from "./ui";
import "../styles/flow-library-panel.css";

export type FlowLibraryConnectorKind = "knowledge" | "database" | "crm";

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
    kind: "knowledge",
    title: "知识库 / RAG",
    description: "接入文档、网页与向量索引，为 Flow 提供可追溯的检索上下文。",
  },
  {
    kind: "database",
    title: "业务数据库",
    description: "连接 SQL、数据仓库或内部数据服务，并按身份授予读写范围。",
  },
  {
    kind: "crm",
    title: "CRM",
    description: "连接客户、销售线索与机会数据，供业务 Flow 查询或更新。",
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
          <Library aria-hidden="true" size={18} />
        </span>
        <span>
          <strong>Business Context Library</strong>
          <small>知识、数据与业务系统连接器</small>
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
        title="已连接资源"
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
            <BookOpen aria-hidden="true" size={18} />
            <strong>尚未连接业务上下文</strong>
            <span>接口已经预留，后续可接入 RAG、数据库与 CRM。</span>
          </div>
        )}
      </Panel>
    </div>
  );
}

function libraryKindIcon(kind: FlowLibraryConnectorKind) {
  if (kind === "database") return <Database aria-hidden="true" size={16} />;
  if (kind === "crm") return <ContactRound aria-hidden="true" size={16} />;
  return <BookOpen aria-hidden="true" size={16} />;
}

function libraryKindLabel(kind: FlowLibraryConnectorKind) {
  if (kind === "database") return "Database";
  if (kind === "crm") return "CRM";
  return "Knowledge / RAG";
}

function libraryStatusVariant(status: FlowLibraryConnector["status"]) {
  if (status === "connected") return "success" as const;
  if (status === "attention") return "warning" as const;
  if (status === "disabled") return "neutral" as const;
  return "info" as const;
}
